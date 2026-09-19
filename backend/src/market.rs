//! Market data for a token: price, depth, turnover and the links attached to it.
//!
//! A token can trade in several pools at once. The one that matters for a
//! trader is the deepest, because that is where an order of any size actually
//! executes, so the pool with the most liquidity is chosen and the rest are
//! counted but not merged. Summing liquidity across pools would overstate what
//! a single order can reach.
//!
//! Every field is optional. A provider that does not report a market cap gets
//! `None`, not zero — the difference between "this token has no value" and "we
//! were not told" is the whole point.
//!
//! Responses are cached per mint. The age is returned with the data so the
//! interface can say how stale it is instead of implying it is live.
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

pub const DEFAULT_BASE_URL: &str = "https://api.dexscreener.com";

pub struct Config {
    pub base_url: String,
    /// Seconds a cached quote is served before it is refetched.
    pub cache_seconds: i64,
}
impl Config {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("MARKET_BASE_URL").unwrap_or(DEFAULT_BASE_URL.into()),
            cache_seconds: std::env::var("MARKET_CACHE_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &i64| (1..=3600).contains(v))
                .unwrap_or(20),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct Market {
    pub pair_address: Option<String>,
    pub dex: Option<String>,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub price_usd: Option<f64>,
    /// Price in the quote asset, usually SOL.
    pub price_native: Option<f64>,
    pub liquidity_usd: Option<f64>,
    pub volume_h24: Option<f64>,
    pub market_cap: Option<f64>,
    pub fdv: Option<f64>,
    pub price_change_h24: Option<f64>,
    pub buys_h24: Option<i64>,
    pub sells_h24: Option<i64>,
    /// Unix seconds the pool was created, where reported.
    pub pair_created_at: Option<i64>,
    pub image_url: Option<String>,
    /// `(kind, url)`, e.g. `("twitter", "https://x.com/…")`.
    pub socials: Vec<(String, String)>,
    pub websites: Vec<String>,
    /// Pools seen for this mint. More than one means depth is split.
    pub pools: usize,
    pub warnings: Vec<String>,
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}
fn text(v: &Value, max: usize) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= max)
        .map(str::to_owned)
}

/// Picks the deepest pool in which `mint` is the **base** token, and reads it.
///
/// The provider returns every pool the mint appears in, on either side. In a
/// pool where it is the quote asset, `baseToken`, `priceUsd` and `marketCap`
/// all describe the *other* token — reading those would put a different
/// token's price on this one's page. So pools where the mint is the quote are
/// discarded rather than reinterpreted.
///
/// Pools with unknown liquidity cannot be compared, so they only win when
/// nothing else is available — an unmeasurable pool is not evidence of depth.
pub fn best_pair(response: &Value, mint: &str) -> Option<Market> {
    let all = response["pairs"].as_array()?;
    let pairs: Vec<&Value> = all
        .iter()
        .filter(|p| p["baseToken"]["address"].as_str() == Some(mint))
        .collect();
    if pairs.is_empty() {
        return None;
    }
    let deepest = pairs.iter().max_by(|a, b| {
        let key = |p: &Value| number(&p["liquidity"]["usd"]).unwrap_or(-1.0);
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let info = &deepest["info"];
    let socials: Vec<(String, String)> = info["socials"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|s| Some((text(&s["type"], 40)?.to_lowercase(), text(&s["url"], 500)?)))
                .take(8)
                .collect()
        })
        .unwrap_or_default();
    let websites: Vec<String> = info["websites"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|w| text(&w["url"], 500))
                .take(4)
                .collect()
        })
        .unwrap_or_default();
    let liquidity_usd = number(&deepest["liquidity"]["usd"]);
    let mut warnings = vec![];
    if liquidity_usd.is_none() {
        warnings.push("Liquidity was not reported for this pool".into());
    }
    if liquidity_usd.is_some_and(|l| l < 5_000.0) {
        warnings.push(format!(
            "Only ${:.0} of liquidity: an ordinary order will move this price",
            liquidity_usd.unwrap_or(0.0)
        ));
    }
    if pairs.len() > 1 {
        warnings.push(format!(
            "Trades across {} pools; only the deepest is shown, so total depth is not this number",
            pairs.len()
        ));
    }
    if socials.is_empty() && websites.is_empty() {
        warnings.push("No website or social account is attached to this token".into());
    }
    Some(Market {
        pair_address: text(&deepest["pairAddress"], 64),
        dex: text(&deepest["dexId"], 40),
        name: text(&deepest["baseToken"]["name"], 200),
        symbol: text(&deepest["baseToken"]["symbol"], 40),
        price_usd: number(&deepest["priceUsd"]),
        price_native: number(&deepest["priceNative"]),
        liquidity_usd,
        volume_h24: number(&deepest["volume"]["h24"]),
        market_cap: number(&deepest["marketCap"]),
        fdv: number(&deepest["fdv"]),
        price_change_h24: number(&deepest["priceChange"]["h24"]),
        buys_h24: number(&deepest["txns"]["h24"]["buys"]).map(|n| n as i64),
        sells_h24: number(&deepest["txns"]["h24"]["sells"]).map(|n| n as i64),
        // The provider reports milliseconds; everything else here is seconds.
        pair_created_at: number(&deepest["pairCreatedAt"]).map(|ms| (ms / 1000.0) as i64),
        image_url: text(&info["imageUrl"], 500),
        socials,
        websites,
        pools: pairs.len(),
        warnings,
    })
}

/// The X handle attached to this market, if one is.
pub fn twitter_handle(market: &Market) -> Option<String> {
    market
        .socials
        .iter()
        .find(|(kind, _)| kind == "twitter" || kind == "x")
        .and_then(|(_, url)| crate::social::handle(url))
}

pub fn cached(
    db: &Connection,
    mint: &str,
    cfg: &Config,
    at: i64,
) -> Result<Option<(Value, i64)>, rusqlite::Error> {
    let row = db
        .query_row(
            "SELECT data,fetched FROM market_cache WHERE mint=?1",
            [mint],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(data, fetched)| {
        (at - fetched <= cfg.cache_seconds)
            .then(|| serde_json::from_str(&data).ok().map(|v| (v, at - fetched)))
            .flatten()
    }))
}

pub fn store(db: &Connection, mint: &str, raw: &Value, at: i64) -> Result<(), rusqlite::Error> {
    db.execute(
        "INSERT INTO market_cache VALUES(?1,?2,?3) ON CONFLICT(mint) DO UPDATE SET data=excluded.data,fetched=excluded.fetched",
        params![mint, raw.to_string(), at],
    )?;
    Ok(())
}

async fn get(cfg: &Config, path: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|_| "Cannot create market client")?;
    let response = client
        .get(format!("{}{path}", cfg.base_url))
        .send()
        .await
        .map_err(|_| "Market data request failed".to_string())?;
    // The provider publishes no rate-limit headers, so throttling is expected
    // rather than exceptional and is reported as such.
    if response.status() == 429 {
        return Err("Market data rate limit reached; the cached value is what we have".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "Market provider returned HTTP {}",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .await
        .map_err(|_| "Unreadable market data".into())
}

pub async fn fetch(cfg: &Config, mint: &str) -> Result<Value, String> {
    get(cfg, &format!("/latest/dex/tokens/{mint}")).await
}

/// Several mints in one request. The provider accepts a comma-separated list,
/// which keeps a whole discovery page inside a single call rather than one per
/// card — the difference between fitting the rate limit and not.
pub async fn fetch_many(cfg: &Config, mints: &[String]) -> Result<Value, String> {
    if mints.is_empty() {
        return Ok(Value::Null);
    }
    let joined = mints
        .iter()
        .take(30)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");
    get(cfg, &format!("/latest/dex/tokens/{joined}")).await
}

/// Deepest pool per base mint, from a batch response.
pub fn best_per_mint(
    response: &Value,
    mints: &[String],
) -> std::collections::BTreeMap<String, Market> {
    mints
        .iter()
        .filter_map(|m| best_pair(response, m).map(|market| (m.clone(), market)))
        .collect()
}

/// Recently profiled tokens, for the discovery list.
pub async fn latest_profiles(cfg: &Config) -> Result<Vec<Value>, String> {
    let v = get(cfg, "/token-profiles/latest/v1").await?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const MINT: &str = "So11111111111111111111111111111111111111112";

    /// Shape captured from the live provider.
    fn response() -> Value {
        json!({"pairs":[
            {"chainId":"solana","dexId":"raydium","pairAddress":"POOL_SHALLOW",
             "baseToken":{"address":MINT,"name":"Coin","symbol":"CN"},
             "priceUsd":"1.0003","priceNative":"0.0052","liquidity":{"usd":1000.0},
             "volume":{"h24":500.0},"marketCap":10000.0,"fdv":12000.0,
             "priceChange":{"h24":-4.2},"txns":{"h24":{"buys":10,"sells":4}},
             "pairCreatedAt":1700000000000u64,"info":{}},
            {"chainId":"solana","dexId":"orca","pairAddress":"POOL_DEEP",
             "baseToken":{"address":MINT,"name":"Coin","symbol":"CN"},
             "priceUsd":"1.0010","priceNative":"0.0053","liquidity":{"usd":1556191.19},
             "volume":{"h24":1896662.4},"marketCap":73838129336.0,"fdv":73838129336.0,
             "priceChange":{"h24":0.31},"txns":{"h24":{"buys":900,"sells":700}},
             "pairCreatedAt":1700000000000u64,
             "info":{"imageUrl":"https://img/x.png",
                     "socials":[{"type":"twitter","url":"https://x.com/somecoin"},
                                {"type":"telegram","url":"https://t.me/g"}],
                     "websites":[{"label":"Site","url":"https://example.com"}]}}
        ]})
    }

    #[test]
    fn the_deepest_pool_is_the_one_reported() {
        let m = best_pair(&response(), MINT).unwrap();
        assert_eq!(m.pair_address.as_deref(), Some("POOL_DEEP"));
        assert_eq!(m.dex.as_deref(), Some("orca"));
        assert_eq!(m.liquidity_usd, Some(1_556_191.19));
        assert_eq!(m.price_usd, Some(1.0010));
        assert_eq!(m.volume_h24, Some(1_896_662.4));
        assert_eq!(m.buys_h24, Some(900));
        assert_eq!(m.pools, 2);
        // Depth is split, and saying so matters more than the headline number.
        assert!(m.warnings.iter().any(|w| w.contains("across 2 pools")));
    }
    #[test]
    fn milliseconds_become_seconds_like_every_other_timestamp() {
        assert_eq!(
            best_pair(&response(), MINT).unwrap().pair_created_at,
            Some(1_700_000_000)
        );
    }
    #[test]
    fn missing_numbers_are_none_never_zero() {
        let mut v = response();
        for pair in v["pairs"].as_array_mut().unwrap() {
            pair["marketCap"] = Value::Null;
            pair["liquidity"] = json!({});
            pair["priceUsd"] = Value::Null;
        }
        let m = best_pair(&v, MINT).unwrap();
        assert_eq!(m.market_cap, None, "unreported is not worthless");
        assert_eq!(m.price_usd, None);
        assert_eq!(m.liquidity_usd, None);
        assert!(
            m.warnings
                .iter()
                .any(|w| w.contains("Liquidity was not reported"))
        );
    }
    #[test]
    fn an_unmeasurable_pool_does_not_beat_a_measured_one() {
        let mut v = response();
        v["pairs"][1]["liquidity"] = json!({}); // the deep pool stops reporting
        let m = best_pair(&v, MINT).unwrap();
        assert_eq!(m.pair_address.as_deref(), Some("POOL_SHALLOW"));
    }
    #[test]
    fn thin_liquidity_is_called_out_in_money_terms() {
        let mut v = response();
        v["pairs"] = json!([v["pairs"][0].clone()]);
        let m = best_pair(&v, MINT).unwrap();
        assert!(m.warnings.iter().any(|w| w.contains("$1000 of liquidity")));
    }
    #[test]
    fn socials_are_read_and_the_x_handle_extracted() {
        let m = best_pair(&response(), MINT).unwrap();
        assert_eq!(m.socials.len(), 2);
        assert_eq!(twitter_handle(&m).as_deref(), Some("somecoin"));
        assert_eq!(m.websites, vec!["https://example.com".to_string()]);
    }
    #[test]
    fn a_token_with_no_links_is_flagged_and_has_no_handle() {
        let mut v = response();
        v["pairs"][1]["info"] = json!({});
        let m = best_pair(&v, MINT).unwrap();
        assert!(twitter_handle(&m).is_none());
        assert!(
            m.warnings
                .iter()
                .any(|w| w.contains("No website or social"))
        );
    }
    #[test]
    fn an_empty_or_absent_pair_list_is_not_a_market() {
        assert!(best_pair(&json!({"pairs":[]}), MINT).is_none());
        assert!(best_pair(&json!({}), MINT).is_none());
        assert!(best_pair(&json!({"pairs":Value::Null}), MINT).is_none());
    }
    #[test]
    fn a_pool_where_the_mint_is_the_quote_asset_is_not_this_tokens_market() {
        // Live bug: asking for USDC returned PUMP, because the deepest pool
        // containing USDC has USDC as the *quote* and another token as base.
        let other = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let v = json!({"pairs":[
            {"pairAddress":"QUOTE_SIDE","baseToken":{"address":other,"symbol":"OTHER"},
             "quoteToken":{"address":MINT},"priceUsd":"999.0","liquidity":{"usd":99999999.0},"info":{}},
            {"pairAddress":"BASE_SIDE","baseToken":{"address":MINT,"symbol":"MINE"},
             "priceUsd":"1.5","liquidity":{"usd":1000.0},"info":{}}
        ]});
        let m = best_pair(&v, MINT).unwrap();
        assert_eq!(m.pair_address.as_deref(), Some("BASE_SIDE"));
        assert_eq!(m.symbol.as_deref(), Some("MINE"));
        assert_eq!(m.price_usd, Some(1.5), "never another token's price");
        assert_eq!(m.pools, 1, "only pools where this mint is the base count");
        // A mint that appears solely as a quote asset has no market of its own.
        assert!(best_pair(&v, "11111111111111111111111111111111").is_none());
    }
    #[test]
    fn the_cache_expires_and_reports_its_age() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::MARKET_SCHEMA).unwrap();
        let cfg = Config {
            base_url: DEFAULT_BASE_URL.into(),
            cache_seconds: 20,
        };
        store(&db, "mint", &response(), 1000).unwrap();
        let (_, age) = cached(&db, "mint", &cfg, 1015).unwrap().unwrap();
        assert_eq!(age, 15);
        assert!(
            cached(&db, "mint", &cfg, 1021).unwrap().is_none(),
            "stale is not served"
        );
        assert!(cached(&db, "other", &cfg, 1000).unwrap().is_none());
    }
}
