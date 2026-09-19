//! Funding-graph clustering and entry-timing analysis.
//!
//! A wallet can be reliably profitable for reasons that cannot be copied: it was
//! funded by the deployer, it enters in the first seconds of a token's life, or it
//! operates alongside wallets fed from the same source. This module measures those
//! structural facts so they can be separated from selection skill.
//!
//! **Common funding is a lead, not proof of identity or misconduct.** Exchanges,
//! bridges and shared custody produce the same edge shape as coordination. Every
//! output here is a flag for a human to weigh, and nothing here asserts intent.
//!
//! Coverage is bounded by what has been archived. A wallet with no collected raw
//! transactions has an *unknown* funding graph, which is never reported as a clean
//! one.
use crate::analytics::Trade;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SYSTEM: &str = "11111111111111111111111111111111";

pub struct Config {
    /// A buy this soon after a token's first observed trade counts as an early entry.
    pub early_window_seconds: i64,
    /// Early-entry share above which copying is refused, as a percentage.
    pub max_early_entry_pct: f64,
    pub enforce: bool,
    /// Gate on a provable deployer→wallet transfer preceding a buy of that
    /// deployer's token. Off by default: it is the one funding pattern strong
    /// enough to act on, and even it is opt-in.
    pub enforce_deployer_funding: bool,
    /// How long before the buy the transfer must have landed.
    pub deployer_window_seconds: i64,
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
            early_window_seconds: number("EARLY_ENTRY_WINDOW_SECONDS", 30.0) as i64,
            max_early_entry_pct: number("MAX_EARLY_ENTRY_PCT", 80.0),
            enforce: std::env::var("ENFORCE_AUTHENTICITY").unwrap_or_default() != "false",
            enforce_deployer_funding: std::env::var("ENFORCE_DEPLOYER_FUNDING").unwrap_or_default()
                == "true",
            deployer_window_seconds: number("DEPLOYER_FUNDING_WINDOW", 86400.0) as i64,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Transfer {
    pub source: String,
    pub destination: String,
    pub lamports: i64,
}

/// Extracts native SOL transfers from one archived transaction.
///
/// Only the System Program's parsed `transfer` form is recognised. Token
/// transfers, `transferWithSeed`, CPI shapes the provider did not parse, and
/// anything with an implausible amount are skipped rather than guessed at, so a
/// missing edge means "not observed", never "did not happen".
pub fn transfers(v: &Value) -> Vec<Transfer> {
    if !v["meta"]["err"].is_null() {
        return vec![];
    }
    let mut all = vec![];
    if let Some(outer) = v["transaction"]["message"]["instructions"].as_array() {
        all.extend(outer);
    }
    if let Some(groups) = v["meta"]["innerInstructions"].as_array() {
        for group in groups {
            if let Some(inner) = group["instructions"].as_array() {
                all.extend(inner);
            }
        }
    }
    all.into_iter()
        .filter(|i| i["programId"].as_str() == Some(SYSTEM))
        .filter(|i| i["parsed"]["type"].as_str() == Some("transfer"))
        .filter_map(|i| {
            let info = &i["parsed"]["info"];
            let source = info["source"].as_str()?;
            let destination = info["destination"].as_str()?;
            let lamports = info["lamports"].as_i64().filter(|n| *n > 0)?;
            (source != destination
                && crate::rpc::valid_address(source)
                && crate::rpc::valid_address(destination))
            .then(|| Transfer {
                source: source.into(),
                destination: destination.into(),
                lamports,
            })
        })
        .collect()
}

/// Indexes funding edges for one wallet from its archived raw transactions.
pub fn index(db: &Connection, wallet: &str) -> Result<usize, rusqlite::Error> {
    let archived = {
        let mut stmt = db.prepare("SELECT signature,data FROM raw_transactions WHERE wallet=?1")?;
        stmt.query_map([wallet], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut written = 0;
    for (signature, data) in archived {
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let at = value["blockTime"].as_i64().unwrap_or(0);
        for t in transfers(&value) {
            written += db.execute(
                "INSERT OR IGNORE INTO funding_edges VALUES(?1,?2,?3,?4,?5)",
                params![signature, t.source, t.destination, t.lamports, at],
            )?;
        }
    }
    Ok(written)
}

/// Share of a wallet's buys placed within `window` seconds of the first trade
/// observed on that token. High values mean the wallet is consistently present at
/// the very start, which a follower arriving through a signal cannot reproduce.
pub fn early_entries(wallet: &str, all: &[Trade], window: i64) -> (usize, usize) {
    let mut first: BTreeMap<&str, i64> = BTreeMap::new();
    for t in all {
        let entry = first.entry(&t.token).or_insert(t.timestamp);
        *entry = (*entry).min(t.timestamp);
    }
    let buys: Vec<&Trade> = all
        .iter()
        .filter(|t| t.wallet == wallet && t.side == "buy")
        .collect();
    let early = buys
        .iter()
        .filter(|t| {
            first
                .get(t.token.as_str())
                .is_some_and(|f| t.timestamp - f <= window)
        })
        .count();
    (early, buys.len())
}

#[derive(Debug, Serialize)]
pub struct Authenticity {
    pub wallet: String,
    pub buys: usize,
    pub early_entries: usize,
    pub early_entry_pct: Option<f64>,
    pub early_window_seconds: i64,
    /// Addresses observed sending SOL to this wallet.
    pub funders: Vec<String>,
    /// Other wallets in our dataset fed by at least one of the same funders.
    pub co_funded_wallets: Vec<String>,
    /// A funder that also traded a token this wallet bought, before it bought.
    pub funded_by_counterparty: Vec<String>,
    /// False when no raw transactions have been archived for this wallet.
    pub funding_graph_observed: bool,
    pub flags: Vec<String>,
}

pub fn assess(
    db: &Connection,
    wallet: &str,
    all: &[Trade],
    cfg: &Config,
) -> Result<Authenticity, rusqlite::Error> {
    let (early, buys) = early_entries(wallet, all, cfg.early_window_seconds);
    let funders: Vec<String> = {
        let mut stmt = db.prepare(
            "SELECT DISTINCT source FROM funding_edges WHERE destination=?1 ORDER BY source LIMIT 50",
        )?;
        stmt.query_map([wallet], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let co_funded: Vec<String> = if funders.is_empty() {
        vec![]
    } else {
        let mut stmt = db.prepare("SELECT DISTINCT destination FROM funding_edges WHERE destination<>?1 AND source IN (SELECT source FROM funding_edges WHERE destination=?1) ORDER BY destination LIMIT 50")?;
        stmt.query_map([wallet], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let observed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM raw_transactions WHERE wallet=?1)",
        [wallet],
        |r| r.get(0),
    )?;

    // A funder that was already trading a token before this wallet bought it had
    // information this wallet plausibly shared. Suggestive, not conclusive.
    let funder_set: BTreeSet<&str> = funders.iter().map(String::as_str).collect();
    let mut counterparty = BTreeSet::new();
    for buy in all.iter().filter(|t| t.wallet == wallet && t.side == "buy") {
        for other in all
            .iter()
            .filter(|t| t.token == buy.token && t.timestamp < buy.timestamp)
        {
            if funder_set.contains(other.wallet.as_str()) {
                counterparty.insert(other.wallet.clone());
            }
        }
    }

    let pct = (buys > 0).then(|| early as f64 / buys as f64 * 100.0);
    let mut flags = vec![
        "Funding and timing are structural observations. Common funding is a lead, not proof of identity or misconduct.".to_string(),
    ];
    if !observed {
        flags.push("No raw transactions archived: the funding graph is unknown, not clean".into());
    }
    if pct.is_some_and(|p| p > cfg.max_early_entry_pct) {
        flags.push(format!(
            "{:.0}% of buys land within {}s of a token's first observed trade: a follower cannot reproduce this entry",
            pct.unwrap_or(0.0),
            cfg.early_window_seconds
        ));
    }
    if !counterparty.is_empty() {
        flags.push(
            "A funder of this wallet traded the same tokens beforehand: profits may reflect shared information rather than selection".into(),
        );
    }
    if co_funded.len() >= 3 {
        flags.push(format!(
            "{} other observed wallets share a funder with this one",
            co_funded.len()
        ));
    }
    Ok(Authenticity {
        wallet: wallet.into(),
        buys,
        early_entries: early,
        early_entry_pct: pct,
        early_window_seconds: cfg.early_window_seconds,
        funders,
        co_funded_wallets: co_funded,
        funded_by_counterparty: counterparty.into_iter().collect(),
        funding_graph_observed: observed,
        flags,
    })
}

/// The provable deployer-funded case, and only that case.
///
/// Requires all three, from our own records:
/// 1. the token's deployer is known,
/// 2. that deployer sent SOL directly to this wallet,
/// 3. the transfer landed within the window *before* the wallet bought that
///    deployer's token.
///
/// Published work isolating exactly this pattern found 87% of such snipes
/// profitable. An 87% hit rate is position, not skill, and it is not something
/// a follower can reproduce.
///
/// Everything weaker — a shared funder, an exchange edge, a co-funding set —
/// stays report-only, because those shapes are routinely innocent.
pub fn deployer_funded(
    db: &Connection,
    wallet: &str,
    token: &str,
    bought_at: i64,
    cfg: &Config,
) -> Result<Option<String>, rusqlite::Error> {
    let deployer: Option<String> = db
        .query_row(
            "SELECT creator FROM tokens WHERE mint=?1 AND creator IS NOT NULL",
            [token],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(deployer) = deployer else {
        return Ok(None);
    };
    if deployer == wallet {
        return Ok(Some(
            "This wallet deployed the token it is buying".to_string(),
        ));
    }
    let funded: Option<i64> = db
        .query_row(
            "SELECT MAX(timestamp) FROM funding_edges WHERE source=?1 AND destination=?2 AND timestamp<=?3 AND timestamp>=?4",
            params![
                deployer,
                wallet,
                bought_at,
                bought_at - cfg.deployer_window_seconds
            ],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(funded.map(|at| {
        format!(
            "The deployer sent SOL to this wallet {} seconds before it bought their token",
            bought_at - at
        )
    }))
}

/// Entry refusal for the paper policy. Only the timing measure gates, because it
/// is computed from our own trade record; funding leads are surfaced for review
/// rather than enforced, since a shared funder is routinely innocent.
pub fn refuse(wallet: &str, all: &[Trade], cfg: &Config) -> Option<String> {
    if !cfg.enforce {
        return None;
    }
    let (early, buys) = early_entries(wallet, all, cfg.early_window_seconds);
    let pct = (buys > 0).then(|| early as f64 / buys as f64 * 100.0)?;
    (pct > cfg.max_early_entry_pct).then(|| {
        format!(
            "Source wallet enters within {}s of a token's first trade on {:.0}% of buys",
            cfg.early_window_seconds, pct
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
    const B: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
    const C: &str = "So11111111111111111111111111111111111111112";
    fn cfg() -> Config {
        Config {
            early_window_seconds: 30,
            max_early_entry_pct: 80.0,
            enforce: true,
            enforce_deployer_funding: true,
            deployer_window_seconds: 86400,
        }
    }
    fn raw(source: &str, destination: &str, lamports: i64) -> Value {
        serde_json::json!({"blockTime":1000,"meta":{"err":null,"innerInstructions":[]},
            "transaction":{"message":{"instructions":[
                {"programId":SYSTEM,"parsed":{"type":"transfer","info":{"source":source,"destination":destination,"lamports":lamports}}}]}}})
    }
    fn trade(id: &str, wallet: &str, token: &str, side: &str, time: i64) -> Trade {
        Trade {
            id: id.into(),
            wallet: wallet.into(),
            token: token.into(),
            symbol: "T".into(),
            side: side.into(),
            quantity: 1.0,
            price_sol: 1.0,
            fee_sol: 0.0,
            timestamp: time,
            liquidity_sol: 1000.0,
            routed: None,
        }
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE raw_transactions(signature TEXT,wallet TEXT,data TEXT,created INTEGER,PRIMARY KEY(signature,wallet));").unwrap();
        db.execute_batch(crate::CLUSTER_SCHEMA).unwrap();
        db.execute_batch(crate::SOCIAL_SCHEMA).unwrap();
        db
    }
    fn archive(db: &Connection, signature: &str, wallet: &str, v: &Value) {
        db.execute(
            "INSERT INTO raw_transactions VALUES(?1,?2,?3,0)",
            params![signature, wallet, v.to_string()],
        )
        .unwrap();
    }
    #[test]
    fn parses_native_transfers_only() {
        let t = transfers(&raw(A, B, 50000));
        assert_eq!(
            t,
            vec![Transfer {
                source: A.into(),
                destination: B.into(),
                lamports: 50000
            }]
        );
        let mut v = raw(A, B, 50000);
        v["meta"]["err"] = serde_json::json!({"InstructionError":[0,"e"]});
        assert!(transfers(&v).is_empty());
        v = raw(A, B, 50000);
        v["transaction"]["message"]["instructions"][0]["parsed"]["type"] =
            serde_json::json!("createAccount");
        assert!(transfers(&v).is_empty());
        v = raw(A, A, 50000);
        assert!(transfers(&v).is_empty(), "self-transfer is not funding");
        v = raw(A, B, 0);
        assert!(transfers(&v).is_empty());
    }
    #[test]
    fn indexing_is_idempotent_and_finds_co_funded_wallets() {
        let db = db();
        archive(&db, "s1", B, &raw(A, B, 1_000_000));
        archive(&db, "s2", C, &raw(A, C, 2_000_000));
        assert_eq!(index(&db, B).unwrap(), 1);
        assert_eq!(index(&db, B).unwrap(), 0);
        index(&db, C).unwrap();
        let a = assess(&db, B, &[], &cfg()).unwrap();
        assert_eq!(a.funders, vec![A.to_string()]);
        assert_eq!(a.co_funded_wallets, vec![C.to_string()]);
        assert!(a.funding_graph_observed);
    }
    #[test]
    fn an_unarchived_wallet_is_unknown_not_clean() {
        let db = db();
        let a = assess(&db, B, &[], &cfg()).unwrap();
        assert!(!a.funding_graph_observed);
        assert!(a.funders.is_empty());
        assert!(a.flags.iter().any(|f| f.contains("unknown, not clean")));
    }
    #[test]
    fn early_entry_rate_measures_against_the_tokens_first_trade() {
        // `A` is always first; `B` arrives long after.
        let all = vec![
            trade("1", A, "x", "buy", 1000),
            trade("2", B, "x", "buy", 5000),
            trade("3", A, "y", "buy", 2000),
            trade("4", B, "y", "buy", 2010),
        ];
        assert_eq!(early_entries(A, &all, 30), (2, 2));
        assert_eq!(early_entries(B, &all, 30), (1, 2));
        assert!(refuse(A, &all, &cfg()).unwrap().contains("100%"));
        assert_eq!(refuse(B, &all, &cfg()), None);
    }
    #[test]
    fn enforcement_can_be_switched_off_and_is_silent_without_buys() {
        let all = vec![trade("1", A, "x", "buy", 1000)];
        let mut c = cfg();
        assert!(refuse(A, &all, &c).is_some());
        c.enforce = false;
        assert_eq!(refuse(A, &all, &c), None);
        assert_eq!(refuse(A, &[], &cfg()), None);
    }
    fn with_token(db: &Connection, mint: &str, creator: &str) {
        db.execute(
            "INSERT OR IGNORE INTO tokens(mint,creator,first_seen) VALUES(?1,?2,0)",
            params![mint, creator],
        )
        .unwrap();
    }
    #[test]
    fn a_deployer_funding_a_wallet_before_it_buys_is_the_provable_case() {
        let db = db();
        with_token(&db, "x", A);
        // A funded B at t=1000; B buys A's token at t=2000, inside the window.
        db.execute(
            "INSERT INTO funding_edges VALUES('sig',?1,?2,5000000,1000)",
            params![A, B],
        )
        .unwrap();
        let reason = deployer_funded(&db, B, "x", 2000, &cfg()).unwrap().unwrap();
        assert!(reason.contains("1000 seconds before"));
    }
    #[test]
    fn funding_after_the_buy_or_outside_the_window_proves_nothing() {
        let db = db();
        with_token(&db, "x", A);
        db.execute(
            "INSERT INTO funding_edges VALUES('sig',?1,?2,5000000,9000)",
            params![A, B],
        )
        .unwrap();
        // The transfer landed after the buy: not a pre-snipe.
        assert_eq!(deployer_funded(&db, B, "x", 2000, &cfg()).unwrap(), None);
        // And far enough before, it is outside the window.
        let mut narrow = cfg();
        narrow.deployer_window_seconds = 10;
        assert_eq!(deployer_funded(&db, B, "x", 9500, &narrow).unwrap(), None);
    }
    #[test]
    fn a_wallet_buying_its_own_deployment_is_refused_outright() {
        let db = db();
        with_token(&db, "x", A);
        assert!(
            deployer_funded(&db, A, "x", 2000, &cfg())
                .unwrap()
                .unwrap()
                .contains("deployed the token")
        );
    }
    #[test]
    fn an_unknown_deployer_or_unrelated_funder_is_not_a_hit() {
        let db = db();
        // No token record at all.
        assert_eq!(deployer_funded(&db, B, "x", 2000, &cfg()).unwrap(), None);
        // Known deployer, but the funding came from someone else.
        with_token(&db, "y", A);
        db.execute(
            "INSERT INTO funding_edges VALUES('sig',?1,?2,5000000,1000)",
            params![C, B],
        )
        .unwrap();
        assert_eq!(deployer_funded(&db, B, "y", 2000, &cfg()).unwrap(), None);
    }
    #[test]
    fn a_funder_trading_the_same_token_first_is_surfaced() {
        let db = db();
        archive(&db, "s1", B, &raw(A, B, 1_000_000));
        index(&db, B).unwrap();
        let all = vec![
            trade("1", A, "x", "buy", 1000),
            trade("2", B, "x", "buy", 2000),
        ];
        let a = assess(&db, B, &all, &cfg()).unwrap();
        assert_eq!(a.funded_by_counterparty, vec![A.to_string()]);
        assert!(a.flags.iter().any(|f| f.contains("shared information")));
        // Reversed order: the funder bought after, so there is nothing to flag.
        let later = vec![
            trade("1", B, "x", "buy", 1000),
            trade("2", A, "x", "buy", 2000),
        ];
        assert!(
            assess(&db, B, &later, &cfg())
                .unwrap()
                .funded_by_counterparty
                .is_empty()
        );
    }
}
