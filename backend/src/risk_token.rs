//! Token-level safety assessment from a public risk report.
//!
//! This is a *pre-entry gate*, not a prediction. It answers "does this mint retain
//! permissions that let someone take the position away from us" — revocable supply,
//! freezable accounts, unlocked liquidity — which is checkable now, rather than
//! "will the price fall", which is not.
//!
//! A missing or unparseable report is **unknown**, never "safe". Under
//! `TOKEN_RISK_REQUIRED` an unknown token cannot be entered at all.
//!
//! Reported holder concentration includes pool and bonding-curve accounts, which
//! legitimately hold most of a young supply. It is therefore surfaced as context
//! and deliberately not used as a blocking threshold; blocking on it would reject
//! every pre-migration token for a reason that is not a defect.
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

pub const DEFAULT_BASE_URL: &str = "https://api.rugcheck.xyz/v1";

pub struct Config {
    pub base_url: String,
    pub max_score: f64,
    pub min_lp_locked_pct: f64,
    pub required: bool,
    pub max_age: i64,
}
impl Config {
    pub fn from_env() -> Self {
        let number = |key: &str, fallback: f64| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| v.is_finite() && *v >= 0.0)
                .unwrap_or(fallback)
        };
        Self {
            base_url: std::env::var("RUGCHECK_BASE_URL").unwrap_or(DEFAULT_BASE_URL.into()),
            max_score: number("TOKEN_RISK_MAX_SCORE", 40.0),
            min_lp_locked_pct: number("TOKEN_RISK_MIN_LP_LOCKED", 50.0),
            required: std::env::var("TOKEN_RISK_REQUIRED").unwrap_or_default() == "true",
            max_age: number("TOKEN_RISK_MAX_AGE", 86400.0) as i64,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Assessment {
    pub mint: String,
    /// Upstream normalized risk, 0 = lowest observed risk, 100 = highest.
    pub score: Option<f64>,
    pub rugged: bool,
    pub mint_authority: bool,
    pub freeze_authority: bool,
    pub metadata_mutable: bool,
    /// Percentage of pooled liquidity locked or burned, across observed markets.
    pub lp_locked_pct: Option<f64>,
    /// Includes pool accounts. Context only; never a blocking threshold.
    pub top_holder_pct: Option<f64>,
    pub holders: Option<i64>,
    pub creator: Option<String>,
    pub risks: Vec<String>,
}

/// Parses an upstream report. Absent fields stay `None` rather than defaulting to
/// a permissive value; authority flags default to *present*, because "the report
/// did not mention it" is not evidence that an authority was revoked.
pub fn assess(mint: &str, v: &Value) -> Option<Assessment> {
    if !crate::rpc::valid_address(mint) {
        return None;
    }
    let reported = v["mint"].as_str();
    if reported.is_some_and(|m| m != mint) {
        return None;
    }
    let token = &v["token"];
    if !v.is_object() || (token.is_null() && v["score_normalised"].is_null()) {
        return None;
    }
    let finite = |x: &Value| x.as_f64().filter(|n| n.is_finite());
    let markets = v["markets"].as_array();
    let lp_locked_pct = markets.and_then(|m| {
        let values: Vec<f64> = m
            .iter()
            .filter_map(|market| finite(&market["lp"]["lpLockedPct"]))
            .collect();
        (!values.is_empty()).then(|| values.iter().copied().fold(f64::INFINITY, f64::min))
    });
    let holders = v["topHolders"].as_array();
    Some(Assessment {
        mint: mint.into(),
        score: finite(&v["score_normalised"]),
        rugged: v["rugged"].as_bool().unwrap_or(false),
        // Absent key and absent object both mean "not reported as revoked".
        mint_authority: token.get("mintAuthority").is_none_or(|v| !v.is_null()),
        freeze_authority: token.get("freezeAuthority").is_none_or(|v| !v.is_null()),
        metadata_mutable: v["tokenMeta"]["mutable"].as_bool().unwrap_or(true),
        lp_locked_pct,
        top_holder_pct: holders
            .and_then(|h| h.iter().filter_map(|x| finite(&x["pct"])).next())
            .filter(|p| (0.0..=100.0).contains(p)),
        holders: v["totalHolders"].as_i64().filter(|n| *n >= 0),
        creator: v["creator"].as_str().map(str::to_owned),
        risks: v["risks"]
            .as_array()
            .map(|r| {
                r.iter()
                    .filter_map(|x| x["name"].as_str())
                    .take(12)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Returns the reason an entry is refused, or `None` when the token clears the gate.
/// Every branch names a checkable on-chain fact, not a forecast.
pub fn gate(a: &Assessment, cfg: &Config) -> Option<String> {
    if a.rugged {
        return Some("Token is reported rugged".into());
    }
    if a.mint_authority {
        return Some("Mint authority is still active: supply can be inflated".into());
    }
    if a.freeze_authority {
        return Some("Freeze authority is still active: holdings can be frozen".into());
    }
    if a.lp_locked_pct.is_some_and(|p| p < cfg.min_lp_locked_pct) {
        return Some(format!(
            "Pooled liquidity locked {:.1}% is below the {:.1}% minimum",
            a.lp_locked_pct.unwrap_or(0.0),
            cfg.min_lp_locked_pct
        ));
    }
    if a.score.is_some_and(|s| s > cfg.max_score) {
        return Some(format!(
            "Upstream risk score {:.0} exceeds the configured maximum of {:.0}",
            a.score.unwrap_or(0.0),
            cfg.max_score
        ));
    }
    if a.score.is_none() && cfg.required {
        return Some("No risk score was reported for this token".into());
    }
    None
}

/// Cached decision for the paper policy. Unknown is distinct from clear: under
/// `TOKEN_RISK_REQUIRED` a token with no stored assessment cannot be entered.
pub fn check(db: &Connection, mint: &str, cfg: &Config, at: i64) -> Option<String> {
    let stored = db
        .query_row(
            "SELECT blocked_reason,checked FROM token_risk WHERE mint=?1",
            [mint],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    match stored {
        Some((reason, checked)) if at - checked <= cfg.max_age => reason,
        Some(_) if cfg.required => Some("Stored token risk assessment is stale".into()),
        None if cfg.required => Some("No token risk assessment has been collected".into()),
        _ => None,
    }
}

pub fn store(
    db: &Connection,
    a: &Assessment,
    blocked: Option<&String>,
    raw: &Value,
    at: i64,
) -> Result<(), rusqlite::Error> {
    db.execute("INSERT INTO token_risk VALUES(?1,?2,?3,?4,?5) ON CONFLICT(mint) DO UPDATE SET score=excluded.score,blocked_reason=excluded.blocked_reason,data=excluded.data,checked=excluded.checked",
        params![a.mint, a.score, blocked, raw.to_string(), at])?;
    Ok(())
}

/// Fetches one report. A provider failure is an error, never an empty "clear" result.
pub async fn fetch(base_url: &str, mint: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot create risk client")?;
    for attempt in 0..3u64 {
        let response = client
            .get(format!("{base_url}/tokens/{mint}/report"))
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| "Risk provider connection failed".to_string())?;
        if response.status() == 429 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            continue;
        }
        if response.status() == 404 {
            return Err("Risk provider has no report for this mint".into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "Risk provider returned HTTP {}",
                response.status().as_u16()
            ));
        }
        return response
            .json()
            .await
            .map_err(|_| "Risk provider returned an unreadable report".into());
    }
    Err("Risk provider rate limit reached; retry later".into())
}

pub fn summary(a: &Assessment, blocked: Option<&String>) -> Value {
    json!({"assessment":a,"blocked":blocked,"note":"Checkable permission and liquidity state at collection time. Not a prediction of price, and not a guarantee the token is safe. Reported holder concentration includes pool accounts and is context only."})
}

#[cfg(test)]
mod tests {
    use super::*;
    const MINT: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
    fn report() -> Value {
        json!({"mint":MINT,"score_normalised":12,"rugged":false,
            "token":{"mintAuthority":null,"freezeAuthority":null,"decimals":6},
            "tokenMeta":{"mutable":false},"totalHolders":842,"creator":"creator",
            "markets":[{"lp":{"lpLockedPct":100.0}}],
            "topHolders":[{"pct":63.2},{"pct":4.0}],
            "risks":[{"name":"Low amount of LP Providers"}]})
    }
    fn cfg() -> Config {
        Config {
            base_url: DEFAULT_BASE_URL.into(),
            max_score: 40.0,
            min_lp_locked_pct: 50.0,
            required: false,
            max_age: 86400,
        }
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::RISK_SCHEMA).unwrap();
        db
    }
    #[test]
    fn clean_report_passes_and_keeps_concentration_as_context() {
        let a = assess(MINT, &report()).unwrap();
        assert_eq!(a.score, Some(12.0));
        assert_eq!(a.lp_locked_pct, Some(100.0));
        assert_eq!(a.top_holder_pct, Some(63.2));
        assert_eq!(a.holders, Some(842));
        assert_eq!(a.risks, vec!["Low amount of LP Providers".to_string()]);
        // 63% in one account is a pool, not automatically a defect.
        assert_eq!(gate(&a, &cfg()), None);
    }
    #[test]
    fn retained_authorities_block_entry() {
        let mut v = report();
        v["token"]["mintAuthority"] = json!("someauthority");
        assert!(
            gate(&assess(MINT, &v).unwrap(), &cfg())
                .unwrap()
                .contains("Mint authority")
        );
        v = report();
        v["token"]["freezeAuthority"] = json!("someauthority");
        assert!(
            gate(&assess(MINT, &v).unwrap(), &cfg())
                .unwrap()
                .contains("Freeze authority")
        );
        v = report();
        v["rugged"] = json!(true);
        assert!(
            gate(&assess(MINT, &v).unwrap(), &cfg())
                .unwrap()
                .contains("rugged")
        );
    }
    #[test]
    fn silence_about_an_authority_is_not_revocation() {
        // A report that omits the token object entirely must not read as "clear".
        let a = assess(MINT, &json!({"mint":MINT,"score_normalised":5})).unwrap();
        assert!(a.mint_authority);
        assert!(a.freeze_authority);
        assert!(a.metadata_mutable);
        assert!(gate(&a, &cfg()).is_some());
    }
    #[test]
    fn unlocked_liquidity_and_high_score_block_entry() {
        let mut v = report();
        v["markets"] = json!([{"lp":{"lpLockedPct":100.0}},{"lp":{"lpLockedPct":3.0}}]);
        let a = assess(MINT, &v).unwrap();
        assert_eq!(a.lp_locked_pct, Some(3.0)); // worst observed market, not the average
        assert!(gate(&a, &cfg()).unwrap().contains("below"));
        v = report();
        v["score_normalised"] = json!(88);
        assert!(
            gate(&assess(MINT, &v).unwrap(), &cfg())
                .unwrap()
                .contains("exceeds")
        );
    }
    #[test]
    fn mismatched_or_malformed_reports_are_rejected() {
        let mut v = report();
        v["mint"] = json!("So11111111111111111111111111111111111111112");
        assert!(assess(MINT, &v).is_none());
        assert!(assess("not-an-address", &report()).is_none());
        assert!(assess(MINT, &json!({"mint":MINT})).is_none());
    }
    #[test]
    fn unknown_tokens_block_only_when_required() {
        let db = db();
        let mut c = cfg();
        assert_eq!(check(&db, MINT, &c, 1000), None);
        c.required = true;
        assert!(
            check(&db, MINT, &c, 1000)
                .unwrap()
                .contains("No token risk")
        );

        let a = assess(MINT, &report()).unwrap();
        store(&db, &a, None, &report(), 1000).unwrap();
        assert_eq!(check(&db, MINT, &c, 1000), None);
        // A stored clear result expires; stale is unknown, not clear.
        assert!(
            check(&db, MINT, &c, 1000 + 86401)
                .unwrap()
                .contains("stale")
        );

        let blocked = "Mint authority is still active".to_string();
        store(&db, &a, Some(&blocked), &report(), 2000).unwrap();
        assert_eq!(check(&db, MINT, &c, 2000), Some(blocked));
    }
}
