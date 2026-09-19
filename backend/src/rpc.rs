//! Conservative native-SOL swap extraction. Never classifies arbitrary transfers
//! as trades. Raw responses remain available for later protocol-specific replay.
//!
//! # Direct and routed swaps
//!
//! Most real Pump volume does not call the program directly — it arrives through
//! Jupiter or a bot router, with the Pump instruction nested inside. Requiring a
//! direct top-level call rejected the majority of live trades.
//!
//! Both are decoded now, and which one it was is recorded on the trade. That
//! distinction is itself a signal: routing through an aggregator versus invoking
//! the program directly is the published proxy for bot-attributed flow.
//!
//! The measurement is the wallet's **net position change**, not the route: if an
//! address spent X SOL and ended up holding Y of one token, the effective price
//! is X/Y however many pools were crossed. Aggregator and platform fees are
//! inside that number, which makes the recorded price slightly worse than the
//! pool price — the conservative direction.
//!
//! The limitation that remains: a transaction that both swaps *and* moves the
//! same token to a third party nets to a single delta, so the implied price
//! would be wrong. Such transactions are not currently detected.
use crate::analytics::Trade;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const PUMP_AMM: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const WSOL: &str = "So11111111111111111111111111111111111111112";
pub fn valid_address(s: &str) -> bool {
    s.len() >= 32 && s.len() <= 44 && bs58::decode(s).into_vec().is_ok_and(|v| v.len() == 32)
}
pub struct Snapshot {
    pub signature: String,
    pub data: Value,
    pub trade: Option<Trade>,
}
pub struct Page {
    pub snapshots: Vec<Snapshot>,
    pub next_before: Option<String>,
    pub scanned: usize,
    pub failed: usize,
}
async fn call(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    for attempt in 0..3u64 {
        let response = client
            .post(url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .map_err(|_| "RPC connection failed".to_string())?;
        if response.status() == 429 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("RPC returned HTTP {}", response.status()));
        }
        let v: Value = response.json().await.map_err(|_| "Invalid RPC response")?;
        if v.get("error").is_some() {
            return Err(
                "RPC rejected the request; inspect provider limits and transaction-version support"
                    .into(),
            );
        }
        return Ok(v["result"].clone());
    }
    Err("RPC rate limit reached; retry later".into())
}
pub async fn fetch(url: &str, wallet: &str, before: Option<&str>) -> Result<Page, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot create RPC client")?;
    let mut config = json!({"limit":20,"commitment":"finalized"});
    if let Some(b) = before {
        config["before"] = json!(b);
    }
    let list = call(
        &client,
        url,
        "getSignaturesForAddress",
        json!([wallet, config]),
    )
    .await?;
    let signatures = list.as_array().ok_or("Missing signature list")?;
    let next_before = signatures
        .last()
        .and_then(|v| v["signature"].as_str())
        .map(str::to_owned);
    let mut snapshots = vec![];
    let mut failed = 0;
    for item in signatures {
        let Some(signature) = item["signature"].as_str() else {
            failed += 1;
            continue;
        };
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        let raw=call(&client,url,"getTransaction",json!([signature,{"encoding":"jsonParsed","commitment":"finalized","maxSupportedTransactionVersion":0}])).await?;
        if raw.is_null() {
            failed += 1;
            continue;
        }
        let trade = decode(wallet, signature, &raw);
        snapshots.push(Snapshot {
            signature: signature.into(),
            data: raw,
            trade,
        });
    }
    Ok(Page {
        snapshots,
        next_before,
        scanned: signatures.len(),
        failed,
    })
}
/// Infers the pool's SOL balance after this swap.
///
/// The counterparty of a native-SOL swap is the account that received what the
/// trader paid, or paid what the trader received — a bonding curve or an AMM's
/// SOL vault. It is found by lamport movement rather than by address, because
/// the program's account layout differs across Pump versions and a wrong index
/// would be a wrong number rather than a missing one.
///
/// Protocol fees go to a separate recipient, so the counterparty's movement is
/// close to, not equal to, the trader's. A tolerance band absorbs that. The
/// match must be **unique**: if two accounts moved a similar amount the pool is
/// ambiguous, and ambiguous means unknown, which blocks paper entry.
fn pool_balance(meta: &Value, owned: &BTreeSet<usize>, quote_lamports: i128) -> Option<f64> {
    let pre = meta["preBalances"].as_array()?;
    let post = meta["postBalances"].as_array()?;
    let paid = quote_lamports.unsigned_abs() as f64;
    if paid <= 0.0 {
        return None;
    }
    // The trader bought, so the counterparty gained; or sold, so it lost.
    let want_gain = quote_lamports < 0;
    let mut found: Option<f64> = None;
    for index in 0..pre.len().min(post.len()) {
        if owned.contains(&index) {
            continue;
        }
        let (Some(before), Some(after)) = (pre[index].as_u64(), post[index].as_u64()) else {
            continue;
        };
        let moved = after as i128 - before as i128;
        if (moved > 0) != want_gain {
            continue;
        }
        let ratio = moved.unsigned_abs() as f64 / paid;
        if !(0.9..=1.1).contains(&ratio) {
            continue;
        }
        if found.is_some() {
            return None; // ambiguous counterparty
        }
        found = Some(after as f64 / 1e9);
    }
    // A pool holding more SOL than plausibly exists in one curve or pool is a
    // sign the wrong account matched.
    found.filter(|sol| sol.is_finite() && *sol > 0.0 && *sol < 1e7)
}

/// Parsed account state, used to check that a configured account is what the
/// operator thinks it is.
pub async fn account_info(url: &str, address: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|_| "Cannot create RPC client")?;
    call(
        &client,
        url,
        "getAccountInfo",
        json!([address, {"encoding":"jsonParsed","commitment":"confirmed"}]),
    )
    .await
}

/// One finalized transaction, for the live watcher.
pub async fn transaction(url: &str, signature: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot create RPC client")?;
    let raw = call(
        &client,
        url,
        "getTransaction",
        json!([signature, {"encoding":"jsonParsed","commitment":"confirmed","maxSupportedTransactionVersion":0}]),
    )
    .await?;
    if raw.is_null() {
        return Err("Transaction not available yet".into());
    }
    Ok(raw)
}

fn instructions(v: &Value) -> Vec<&Value> {
    let mut out = vec![];
    if let Some(a) = v["transaction"]["message"]["instructions"].as_array() {
        out.extend(a);
    }
    if let Some(a) = v["meta"]["innerInstructions"].as_array() {
        for group in a {
            if let Some(i) = group["instructions"].as_array() {
                out.extend(i);
            }
        }
    }
    out
}
pub fn decode(wallet: &str, signature: &str, v: &Value) -> Option<Trade> {
    let meta = &v["meta"];
    if meta.is_null() || !meta["err"].is_null() {
        return None;
    }
    let ins = instructions(v);
    let is_pump = |i: &&Value| matches!(i["programId"].as_str(), Some(PUMP) | Some(PUMP_AMM));
    // The venue must appear somewhere. Without it this is some other protocol's
    // trade, or not a trade at all.
    if !ins.iter().any(is_pump) {
        return None;
    }
    let outer = v["transaction"]["message"]["instructions"].as_array()?;
    // Direct invocation means the signer called Pump themselves. When the
    // instruction only appears nested, the trade was routed.
    let routed = !outer.iter().any(|i| is_pump(&i));
    let keys = v["transaction"]["message"]["accountKeys"].as_array()?;
    let wi = keys
        .iter()
        .position(|k| k["pubkey"].as_str() == Some(wallet))?;
    if keys[wi]["signer"] != true {
        return None;
    }
    let mut owned = BTreeSet::from([wi]);
    let mut deltas: BTreeMap<String, f64> = BTreeMap::new();
    for (field, sign) in [("preTokenBalances", -1.0), ("postTokenBalances", 1.0)] {
        for balance in meta[field].as_array()? {
            if balance["owner"].as_str() != Some(wallet) {
                continue;
            }
            let index = balance["accountIndex"].as_u64()? as usize;
            owned.insert(index);
            let mint = balance["mint"].as_str()?;
            let decimals = balance["uiTokenAmount"]["decimals"].as_i64()?;
            if !(0..=18).contains(&decimals) {
                return None;
            }
            let raw = balance["uiTokenAmount"]["amount"]
                .as_str()?
                .parse::<u64>()
                .ok()?;
            *deltas.entry(mint.into()).or_default() +=
                sign * raw as f64 / 10f64.powi(decimals as i32);
        }
    }
    let changed: Vec<_> = deltas
        .iter()
        .filter(|(mint, q)| mint.as_str() != WSOL && q.abs() > 1e-12)
        .collect();
    if changed.len() != 1 {
        return None;
    }
    // Reject unusual delegate changes, burns, minting, or ownership changes.
    if ins.iter().any(|i| {
        matches!(
            i["parsed"]["type"].as_str(),
            Some("mintTo") | Some("burn") | Some("setAuthority") | Some("approve")
        )
    }) {
        return None;
    }
    let (mint, delta) = changed[0];
    if !valid_address(mint) {
        return None;
    }
    let pre = meta["preBalances"].as_array()?;
    let post = meta["postBalances"].as_array()?;
    let owned_indices = owned.clone();
    let mut quote_lamports: i128 = 0;
    for index in owned {
        quote_lamports += post.get(index)?.as_u64()? as i128 - pre.get(index)?.as_u64()? as i128;
    }
    let fee = if wi == 0 { meta["fee"].as_u64()? } else { 0 };
    quote_lamports += fee as i128;
    let quote = quote_lamports as f64 / 1e9;
    if delta.signum() == quote.signum() || quote == 0.0 {
        return None;
    }
    let liquidity_sol = pool_balance(meta, &owned_indices, quote_lamports).unwrap_or(0.0);
    let trade = Trade {
        id: format!("rpc:{signature}:{wallet}"),
        wallet: wallet.into(),
        token: mint.clone(),
        symbol: format!("{}…{}", &mint[..4], &mint[mint.len() - 4..]),
        side: if *delta > 0.0 { "buy" } else { "sell" }.into(),
        quantity: delta.abs(),
        price_sol: quote.abs() / delta.abs(),
        fee_sol: fee as f64 / 1e9,
        timestamp: v["blockTime"].as_i64()?,
        liquidity_sol,
        routed: Some(routed),
    };
    trade.validate().then_some(trade)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn raw() -> Value {
        json!({"blockTime":1700000000,"transaction":{"message":{"accountKeys":[{"pubkey":"wallet","signer":true},{"pubkey":"tokenaccount","signer":false}],"instructions":[{"programId":PUMP}]}},"meta":{"err":null,"fee":5000,"preBalances":[3000000000u64,0],"postBalances":[1997955720u64,2039280],"preTokenBalances":[],"postTokenBalances":[{"accountIndex":1,"owner":"wallet","mint":"6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P","uiTokenAmount":{"amount":"100000000","decimals":6}}],"innerInstructions":[]}})
    }
    #[test]
    fn rent_is_not_trading_loss() {
        let t = decode("wallet", "sig", &raw()).unwrap();
        assert_eq!(t.quantity, 100.0);
        assert!((t.price_sol - 0.01).abs() < 1e-10);
        assert_eq!(t.fee_sol, 0.000005);
        assert_eq!(t.liquidity_sol, 0.0);
    }
    /// A buy where the curve account is visible: trader pays ~1.002 SOL, the
    /// curve gains ~0.99 of it after protocol fees and ends holding 41 SOL.
    fn raw_with_curve() -> Value {
        let mut v = raw();
        v["transaction"]["message"]["accountKeys"]
            .as_array_mut()
            .unwrap()
            .push(json!({"pubkey":"bondingcurve","signer":false}));
        v["meta"]["preBalances"] = json!([3000000000u64, 0, 40008000000u64]);
        v["meta"]["postBalances"] = json!([1997955720u64, 2039280, 41000000000u64]);
        v
    }
    #[test]
    fn the_counterparty_balance_becomes_observed_liquidity() {
        let t = decode("wallet", "sig", &raw_with_curve()).unwrap();
        assert_eq!(t.liquidity_sol, 41.0);
        assert_eq!(t.side, "buy");
    }
    #[test]
    fn an_ambiguous_counterparty_reports_unknown_rather_than_a_guess() {
        let mut v = raw_with_curve();
        // A second account moved a similar amount: which one is the pool?
        v["transaction"]["message"]["accountKeys"]
            .as_array_mut()
            .unwrap()
            .push(json!({"pubkey":"lookalike","signer":false}));
        v["meta"]["preBalances"] = json!([3000000000u64, 0, 40008000000u64, 0]);
        v["meta"]["postBalances"] = json!([1997955720u64, 2039280, 41000000000u64, 995000000u64]);
        let t = decode("wallet", "sig", &v).unwrap();
        assert_eq!(t.liquidity_sol, 0.0, "ambiguous means unknown, not a guess");
    }
    #[test]
    fn an_unrelated_account_movement_is_not_mistaken_for_a_pool() {
        let mut v = raw_with_curve();
        // Movement far outside the tolerance band: not the counterparty.
        v["meta"]["preBalances"] = json!([3000000000u64, 0, 40008000000u64]);
        v["meta"]["postBalances"] = json!([1997955720u64, 2039280, 40009000000u64]);
        assert_eq!(decode("wallet", "sig", &v).unwrap().liquidity_sol, 0.0);
    }
    #[test]
    fn a_sell_finds_the_pool_that_paid_out() {
        let mut v = raw_with_curve();
        // Reverse the direction: trader receives SOL, curve pays it.
        v["meta"]["preTokenBalances"] = v["meta"]["postTokenBalances"].clone();
        v["meta"]["postTokenBalances"] = json!([]);
        v["meta"]["preBalances"] = json!([1000000000u64, 2039280, 41000000000u64]);
        v["meta"]["postBalances"] = json!([1999995000u64, 0, 40000000000u64]);
        let t = decode("wallet", "sig", &v).unwrap();
        assert_eq!(t.side, "sell");
        assert_eq!(t.liquidity_sol, 40.0);
    }
    #[test]
    fn transfers_and_failed_swaps_are_not_trades() {
        let mut v = raw();
        v["transaction"]["message"]["instructions"] =
            json!([{ "programId":"11111111111111111111111111111111"}]);
        assert!(decode("wallet", "sig", &v).is_none());
        v = raw();
        v["meta"]["err"] = json!({"InstructionError":[0,"error"]});
        assert!(decode("wallet", "sig", &v).is_none());
    }
    /// The same economic trade, reached through an aggregator: the Pump
    /// instruction is nested and the top level is a router.
    fn raw_routed() -> Value {
        let mut v = raw_with_curve();
        v["transaction"]["message"]["instructions"] = json!([
            {"programId":"ComputeBudget111111111111111111111111111111"},
            {"programId":"JUP6LkbZbjS7jKqbAUknHmbUbFBLVWtcBgJKBQ5QLpe"}
        ]);
        v["meta"]["innerInstructions"] = json!([{"index":1,"instructions":[{"programId":PUMP}]}]);
        v
    }
    #[test]
    fn a_routed_swap_is_decoded_and_marked_as_routed() {
        let t = decode("wallet", "sig", &raw_routed()).unwrap();
        assert_eq!(t.routed, Some(true));
        assert_eq!(t.side, "buy");
        assert_eq!(t.quantity, 100.0);
        // Price is the wallet's own net cost, whatever route produced it.
        assert!((t.price_sol - 0.01).abs() < 1e-10);
    }
    #[test]
    fn a_direct_call_is_marked_as_direct() {
        assert_eq!(decode("wallet", "sig", &raw()).unwrap().routed, Some(false));
    }
    #[test]
    fn a_route_that_never_touches_pump_is_not_a_trade() {
        let mut v = raw_routed();
        // Same shape, but the nested venue is some other protocol.
        v["meta"]["innerInstructions"] =
            json!([{"index":1,"instructions":[{"programId":"11111111111111111111111111111111"}]}]);
        assert!(decode("wallet", "sig", &v).is_none());
    }
    #[test]
    fn a_routed_swap_still_refuses_to_guess_at_multiple_assets() {
        let mut v = raw_routed();
        v["meta"]["postTokenBalances"].as_array_mut().unwrap().push(
            json!({"accountIndex":1,"owner":"wallet","mint":"secondmint12345","uiTokenAmount":{"amount":"5","decimals":0}}),
        );
        assert!(decode("wallet", "sig", &v).is_none());
    }
    #[test]
    fn multiple_assets_are_rejected() {
        let mut v = raw();
        v["meta"]["postTokenBalances"].as_array_mut().unwrap().push(json!({"accountIndex":1,"owner":"wallet","mint":"differentmint1234","uiTokenAmount":{"amount":"1234","decimals":6}}));
        assert!(decode("wallet", "sig", &v).is_none());
    }
    #[test]
    fn validates_actual_public_key_length() {
        assert!(valid_address(PUMP));
        assert!(!valid_address("hello"));
    }
}

#[cfg(test)]
mod real_world {
    use super::*;
    /// A real Jupiter-routed Pump swap captured from mainnet. Kept as a fixture
    /// because synthetic shapes did not catch what live data does.
    #[test]
    fn a_real_routed_mainnet_swap_decodes() {
        let raw: Value =
            serde_json::from_str(include_str!("../../fixtures/routed-swap.json")).unwrap();
        let wallet = raw["wallet"].as_str().unwrap();
        let signature = raw["signature"].as_str().unwrap();
        let decoded = decode(wallet, signature, &raw["transaction"]);
        let t = decoded.expect("a real routed swap must decode");
        assert_eq!(t.wallet, wallet);
        assert_eq!(t.routed, Some(true));
        assert!(t.quantity > 0.0);
        assert!(t.price_sol > 0.0);
        assert!(t.validate());
    }
}
