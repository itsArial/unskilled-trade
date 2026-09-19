//! What an address actually owns, and who owns a token.
//!
//! Both come straight from the chain, so neither needs a data vendor. Balances
//! are exact integers on chain; they are carried as raw amounts plus decimals
//! and only divided for display, because a balance is the one number a trader
//! will check against their wallet.
//!
//! A holding whose token has no price is worth *unknown*, not zero. A portfolio
//! total therefore reports how much of itself it could not value, rather than
//! quietly understating what someone owns.
use serde::Serialize;
use serde_json::{Value, json};

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

#[derive(Debug, Serialize, PartialEq)]
pub struct Holding {
    pub mint: String,
    /// Integer amount in the token's smallest unit, as a string: a u64 balance
    /// does not survive a round trip through a JSON number.
    pub raw_amount: String,
    pub decimals: u8,
    pub amount: f64,
    pub token_account: String,
}

/// Reads token balances from a `getTokenAccountsByOwner` result.
///
/// Zero balances are dropped: a closed or empty account is not a holding, and
/// listing hundreds of them buries what someone actually owns.
pub fn holdings(response: &Value) -> Vec<Holding> {
    let Some(accounts) = response["value"].as_array() else {
        return vec![];
    };
    accounts
        .iter()
        .filter_map(|a| {
            let info = &a["account"]["data"]["parsed"]["info"];
            let amount = &info["tokenAmount"];
            let decimals = amount["decimals"].as_u64().filter(|d| *d <= 18)? as u8;
            let raw = amount["amount"].as_str()?;
            let parsed = raw.parse::<u128>().ok()?;
            if parsed == 0 {
                return None;
            }
            let mint = info["mint"]
                .as_str()
                .filter(|m| crate::rpc::valid_address(m))?;
            Some(Holding {
                mint: mint.to_string(),
                raw_amount: raw.to_string(),
                decimals,
                amount: parsed as f64 / 10f64.powi(decimals as i32),
                token_account: a["pubkey"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Holder {
    pub address: String,
    pub amount: f64,
    /// Share of the reported supply. `None` when supply is unknown, because a
    /// percentage of an unknown total is not a percentage.
    pub share_pct: Option<f64>,
}

/// Largest token accounts, as the chain reports them.
///
/// These are **token accounts, not people**. One holder can hold through many
/// accounts, and the biggest entry is very often a pool or a program vault
/// rather than a person. The caller is expected to say so.
pub fn largest(response: &Value, supply: Option<f64>) -> Vec<Holder> {
    let Some(list) = response["value"].as_array() else {
        return vec![];
    };
    list.iter()
        .filter_map(|h| {
            let amount = h["uiAmount"]
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.0)?;
            Some(Holder {
                address: h["address"].as_str()?.to_string(),
                amount,
                share_pct: supply
                    .filter(|s| *s > 0.0)
                    .map(|s| (amount / s * 100.0).min(100.0)),
            })
        })
        .collect()
}

pub fn supply(response: &Value) -> Option<f64> {
    response["value"]["uiAmount"]
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
}

async fn call(url: &str, method: &str, params: Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot create RPC client")?;
    let response = client
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(|_| "RPC connection failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!("RPC returned HTTP {}", response.status().as_u16()));
    }
    let v: Value = response
        .json()
        .await
        .map_err(|_| "Unreadable RPC response")?;
    if v.get("error").is_some() {
        return Err("RPC rejected the request".into());
    }
    Ok(v["result"].clone())
}

/// Token balances across both token programs. A wallet can hold the same asset
/// under either, and omitting one would understate the portfolio.
pub async fn fetch_holdings(url: &str, owner: &str) -> Result<Vec<Holding>, String> {
    let mut out = vec![];
    for program in [TOKEN_PROGRAM, TOKEN_2022] {
        let result = call(
            url,
            "getTokenAccountsByOwner",
            json!([owner, {"programId": program}, {"encoding":"jsonParsed","commitment":"confirmed"}]),
        )
        .await?;
        out.extend(holdings(&result));
    }
    Ok(out)
}

pub async fn fetch_largest(url: &str, mint: &str) -> Result<(Vec<Holder>, Option<f64>), String> {
    let total = call(url, "getTokenSupply", json!([mint]))
        .await
        .ok()
        .and_then(|v| supply(&v));
    let result = call(url, "getTokenLargestAccounts", json!([mint])).await?;
    Ok((largest(&result, total), total))
}

#[cfg(test)]
mod tests {
    use super::*;
    const MINT: &str = "So11111111111111111111111111111111111111112";

    fn account(mint: &str, amount: &str, decimals: u64) -> Value {
        json!({"pubkey":"TokenAccountAddress","account":{"data":{"parsed":{"info":{
            "mint":mint,"tokenAmount":{"amount":amount,"decimals":decimals}}}}}})
    }

    #[test]
    fn balances_keep_their_integer_precision() {
        // Beyond f64's exact range; the raw string must survive untouched.
        let v = json!({"value":[account(MINT, "18446744073709551615", 6)]});
        let h = &holdings(&v)[0];
        assert_eq!(h.raw_amount, "18446744073709551615");
        assert_eq!(h.decimals, 6);
    }
    #[test]
    fn empty_accounts_are_not_holdings() {
        let v = json!({"value":[account(MINT,"0",6), account(MINT,"500",2)]});
        let h = holdings(&v);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].amount, 5.0);
    }
    #[test]
    fn malformed_accounts_are_skipped_not_guessed() {
        let v = json!({"value":[
            account("not-an-address","100",6),
            account(MINT,"notanumber",6),
            json!({"pubkey":"x","account":{}}),
            account(MINT,"100",99)
        ]});
        assert!(holdings(&v).is_empty());
        assert!(holdings(&json!({})).is_empty());
    }
    #[test]
    fn holder_shares_need_a_known_supply() {
        let v = json!({"value":[
            {"address":"A","uiAmount":250.0},
            {"address":"B","uiAmount":100.0}
        ]});
        let with = largest(&v, Some(1000.0));
        assert_eq!(with[0].share_pct, Some(25.0));
        // A percentage of an unknown total is not a percentage.
        let without = largest(&v, None);
        assert_eq!(without[0].share_pct, None);
        assert_eq!(without[0].amount, 250.0);
    }
    #[test]
    fn a_share_can_never_exceed_the_whole() {
        // Stale supply against a fresh balance would otherwise read as 340%.
        let v = json!({"value":[{"address":"A","uiAmount":3400.0}]});
        assert_eq!(largest(&v, Some(1000.0))[0].share_pct, Some(100.0));
    }
    #[test]
    fn zero_and_nonfinite_holders_are_dropped() {
        let v = json!({"value":[
            {"address":"A","uiAmount":0.0},
            {"address":"B"},
            {"uiAmount":5.0},
            {"address":"C","uiAmount":5.0}
        ]});
        let h = largest(&v, None);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].address, "C");
    }
    #[test]
    fn supply_is_none_rather_than_zero_when_unreported() {
        assert_eq!(supply(&json!({"value":{"uiAmount":1000.0}})), Some(1000.0));
        assert_eq!(supply(&json!({"value":{"uiAmount":0.0}})), None);
        assert_eq!(supply(&json!({})), None);
    }
}
