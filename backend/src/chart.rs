//! OHLCV candles for a token, from a free public aggregator.
//!
//! Candles are per *pool*, not per token, because a price only exists inside a
//! market. The deepest pool is charted and the rest are ignored rather than
//! averaged: blending pools of different depth invents a price nobody could
//! trade at.
//!
//! The provider allows roughly 30 requests a minute with no key, so responses
//! are cached and the age is returned with them.
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

pub const DEFAULT_BASE_URL: &str = "https://api.geckoterminal.com/api/v2";

pub struct Config {
    pub base_url: String,
    pub cache_seconds: i64,
}
impl Config {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("CHART_BASE_URL").unwrap_or(DEFAULT_BASE_URL.into()),
            cache_seconds: std::env::var("CHART_CACHE_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &i64| (5..=3600).contains(v))
                .unwrap_or(60),
        }
    }
}

/// Supported ranges. Each maps to the provider's resolution plus how many of
/// those units make one candle.
pub const TIMEFRAMES: [(&str, &str, u32); 4] = [
    ("5m", "minute", 5),
    ("1h", "hour", 1),
    ("4h", "hour", 4),
    ("1d", "day", 1),
];

pub fn timeframe(key: &str) -> Option<(&'static str, u32)> {
    TIMEFRAMES
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, unit, aggregate)| (*unit, *aggregate))
}

#[derive(Debug, Serialize, PartialEq, Clone, Copy)]
pub struct Candle {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

/// Parses the provider's array-of-arrays form, oldest first.
///
/// A candle whose high is below its low, or which carries a nonfinite or
/// negative price, is dropped rather than drawn — a malformed bar is worse than
/// a missing one, because it is indistinguishable from a real move.
pub fn candles(response: &Value) -> Vec<Candle> {
    let Some(list) = response["data"]["attributes"]["ohlcv_list"].as_array() else {
        return vec![];
    };
    let mut out: Vec<Candle> = list
        .iter()
        .filter_map(|row| {
            let r = row.as_array()?;
            let n = |i: usize| r.get(i)?.as_f64().filter(|v| v.is_finite());
            let candle = Candle {
                t: n(0)? as i64,
                o: n(1)?,
                h: n(2)?,
                l: n(3)?,
                c: n(4)?,
                v: n(5).unwrap_or(0.0),
            };
            let sane = candle.t > 0
                && candle.o > 0.0
                && candle.c > 0.0
                && candle.l > 0.0
                && candle.h >= candle.l
                && candle.h >= candle.o.max(candle.c)
                && candle.l <= candle.o.min(candle.c)
                && candle.v >= 0.0;
            sane.then_some(candle)
        })
        .collect();
    out.sort_by_key(|c| c.t);
    out.dedup_by_key(|c| c.t);
    out
}

pub fn cached(
    db: &Connection,
    key: &str,
    cfg: &Config,
    at: i64,
) -> Result<Option<(Value, i64)>, rusqlite::Error> {
    let row = db
        .query_row(
            "SELECT data,fetched FROM chart_cache WHERE key=?1",
            [key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(data, fetched)| {
        (at - fetched <= cfg.cache_seconds)
            .then(|| serde_json::from_str(&data).ok().map(|v| (v, at - fetched)))
            .flatten()
    }))
}

pub fn store(db: &Connection, key: &str, raw: &Value, at: i64) -> Result<(), rusqlite::Error> {
    db.execute(
        "INSERT INTO chart_cache VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET data=excluded.data,fetched=excluded.fetched",
        params![key, raw.to_string(), at],
    )?;
    Ok(())
}

async fn get(cfg: &Config, path: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|_| "Cannot create chart client")?;
    let response = client
        .get(format!("{}{path}", cfg.base_url))
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| "Chart request failed".to_string())?;
    if response.status() == 429 {
        return Err("Chart provider rate limit reached; showing what is cached".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "Chart provider returned HTTP {}",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .await
        .map_err(|_| "Unreadable chart data".into())
}

/// Deepest pool for a mint, as the provider ranks it. Returns the bare pool
/// address, not the network-prefixed id.
pub fn deepest_pool(response: &Value) -> Option<String> {
    let pools = response["data"].as_array()?;
    let best = pools.iter().max_by(|a, b| {
        let key = |p: &Value| {
            p["attributes"]["reserve_in_usd"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(-1.0)
        };
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    best["id"]
        .as_str()?
        .rsplit('_')
        .next()
        .filter(|s| crate::rpc::valid_address(s))
        .map(str::to_owned)
}

pub async fn pools(cfg: &Config, mint: &str) -> Result<Value, String> {
    get(cfg, &format!("/networks/solana/tokens/{mint}/pools?page=1")).await
}

pub async fn ohlcv(cfg: &Config, pool: &str, unit: &str, aggregate: u32) -> Result<Value, String> {
    get(
        cfg,
        &format!("/networks/solana/pools/{pool}/ohlcv/{unit}?aggregate={aggregate}&limit=200"),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const MINT: &str = "So11111111111111111111111111111111111111112";

    fn response(rows: Value) -> Value {
        json!({"data":{"attributes":{"ohlcv_list":rows}}})
    }

    #[test]
    fn candles_parse_oldest_first() {
        // The provider returns newest first; a chart needs the reverse.
        let v = response(json!([
            [1789747200, 0.00434, 0.00438, 0.00425, 0.00428, 250912.0],
            [1789743600, 0.00432, 0.00439, 0.00425, 0.00434, 1191098.0]
        ]));
        let c = candles(&v);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].t, 1789743600, "oldest first");
        assert_eq!(c[1].t, 1789747200);
        assert_eq!(c[1].o, 0.00434);
        assert_eq!(c[1].v, 250912.0);
    }
    #[test]
    fn an_impossible_candle_is_dropped_not_drawn() {
        // high below low, and a negative price: both indistinguishable from a
        // real move once rendered.
        let v = response(json!([
            [100, 1.0, 0.5, 2.0, 1.0, 10.0],
            [200, 1.0, 1.2, 0.9, -1.0, 10.0],
            [300, 1.0, 1.2, 0.9, 1.1, 10.0]
        ]));
        let c = candles(&v);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].t, 300);
    }
    #[test]
    fn a_body_outside_its_wick_is_rejected() {
        // close above the high cannot happen and would render as a broken bar.
        let v = response(json!([[100, 1.0, 1.1, 0.9, 1.5, 1.0]]));
        assert!(candles(&v).is_empty());
    }
    #[test]
    fn missing_volume_is_zero_but_missing_prices_are_fatal() {
        assert_eq!(
            candles(&response(json!([[100, 1.0, 1.2, 0.9, 1.1]])))[0].v,
            0.0
        );
        assert!(candles(&response(json!([[100, 1.0, 1.2, 0.9]]))).is_empty());
        assert!(candles(&response(json!([[100, "1.0", 1.2, 0.9, 1.1, 1.0]]))).is_empty());
    }
    #[test]
    fn duplicate_timestamps_collapse() {
        let v = response(json!([
            [100, 1.0, 1.2, 0.9, 1.1, 1.0],
            [100, 1.0, 1.2, 0.9, 1.1, 1.0]
        ]));
        assert_eq!(candles(&v).len(), 1);
    }
    #[test]
    fn an_empty_or_malformed_response_is_no_candles_not_a_panic() {
        assert!(candles(&json!({})).is_empty());
        assert!(candles(&response(json!([]))).is_empty());
        assert!(candles(&json!({"data":{"attributes":{}}})).is_empty());
    }
    #[test]
    fn only_known_timeframes_are_accepted() {
        assert_eq!(timeframe("1h"), Some(("hour", 1)));
        assert_eq!(timeframe("4h"), Some(("hour", 4)));
        assert_eq!(timeframe("1d"), Some(("day", 1)));
        assert_eq!(timeframe("5m"), Some(("minute", 5)));
        // Anything else would be interpolated into the provider's URL.
        assert_eq!(timeframe("../../etc"), None);
        assert_eq!(timeframe("1y"), None);
    }
    #[test]
    fn the_deepest_pool_wins_and_the_network_prefix_is_stripped() {
        let v = json!({"data":[
            {"id":format!("solana_{}", "11111111111111111111111111111111"),
             "attributes":{"reserve_in_usd":"1000.0"}},
            {"id":format!("solana_{MINT}"),"attributes":{"reserve_in_usd":"22477105.75"}}
        ]});
        assert_eq!(deepest_pool(&v).as_deref(), Some(MINT));
    }
    #[test]
    fn a_pool_id_that_is_not_an_address_is_refused() {
        let v =
            json!({"data":[{"id":"solana_not-an-address","attributes":{"reserve_in_usd":"5"}}]});
        assert_eq!(deepest_pool(&v), None);
        assert_eq!(deepest_pool(&json!({"data":[]})), None);
        assert_eq!(deepest_pool(&json!({})), None);
    }
}
