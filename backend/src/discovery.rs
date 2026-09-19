//! Candidate-wallet discovery from a public trade firehose.
//!
//! Observation only. A wallet appearing here has traded size recently; that is not
//! evidence of skill, and nothing in this module feeds the paper policy directly.
//! Candidates must still be backfilled through `rpc` and scored by `analytics`.
//!
//! The upstream reports `vSolInBondingCurve`, which is a *virtual* reserve, not
//! executable liquidity. It is stored under its own name and never written into
//! `Trade::liquidity_sol`, because selling into a virtual reserve is not possible.
//!
//! # Upstream access
//!
//! `subscribeNewToken` is free. `subscribeTokenTrade` is **not**: the provider
//! refuses it without an API key attached to a funded wallet, and a refusal
//! arrives as an ordinary `{"message": ...}` frame rather than a connection
//! error. Without a key the collector therefore sees creations and no trades,
//! which is indistinguishable from a quiet market unless the refusal is
//! surfaced — so it is recorded and reported, never swallowed.
use futures_util::{SinkExt, StreamExt};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const DEFAULT_URL: &str = "wss://pumpportal.fun/api/data";

/// Every Pump.fun curve is seeded with 30 SOL of *virtual* reserves against
/// roughly 1.073B virtual tokens. The virtual figure is therefore never sellable
/// liquidity: the real SOL in the curve is `virtual − 30`, and that difference is
/// what a follower could actually exit into. Graduation happens near 85 SOL real.
pub const VIRTUAL_SOL_OFFSET: f64 = 30.0;
/// Real curve reserve at which Pump.fun migrates to a constant-product pool.
pub const GRADUATION_SOL: f64 = 85.0;

pub struct Config {
    pub url: String,
    pub min_sol: f64,
    pub tracked_tokens: usize,
    /// Write observed trades into the analyzable history and evaluate followers
    /// against them. Off by default: this turns a passive collector into the
    /// thing that drives paper positions.
    pub ingest_trades: bool,
    /// Required by the provider for trade subscriptions.
    pub api_key: Option<String>,
}
impl Config {
    /// Reads operator configuration. Discovery stays off unless explicitly enabled.
    pub fn from_env() -> Option<Self> {
        if std::env::var("DISCOVERY_ENABLED").unwrap_or_default() != "true" {
            return None;
        }
        Some(Self {
            url: std::env::var("PUMPPORTAL_WS_URL").unwrap_or(DEFAULT_URL.into()),
            min_sol: std::env::var("DISCOVERY_MIN_SOL")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| v.is_finite() && *v > 0.0)
                .unwrap_or(2.0),
            tracked_tokens: std::env::var("DISCOVERY_TRACKED_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &usize| (1..=500).contains(v))
                .unwrap_or(150),
            ingest_trades: std::env::var("DISCOVERY_INGEST_TRADES").unwrap_or_default() == "true",
            api_key: std::env::var("PUMPPORTAL_API_KEY")
                .ok()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty()),
        })
    }
}

#[derive(Debug, PartialEq)]
pub enum Event {
    /// A new mint was created. Its trades are worth watching for a while, and
    /// its metadata carries the social account behind it.
    Created {
        mint: String,
        name: Option<String>,
        symbol: Option<String>,
        uri: Option<String>,
        creator: Option<String>,
    },
    /// A buy or sell observed on a tracked mint.
    Trade(Observation),
}
#[derive(Debug, PartialEq)]
pub struct Observation {
    pub wallet: String,
    pub token: String,
    pub side: String,
    pub sol_amount: f64,
    pub token_amount: f64,
    pub virtual_reserve_sol: f64,
    pub signature: String,
    /// Upstream venue label: `pump` is the bonding curve, anything else is a
    /// migrated pool whose reserves this message does not report.
    pub pool: String,
}
impl Observation {
    /// Effective SOL per token for this fill. `None` when either leg is missing,
    /// because a price cannot be inferred from one side of a trade.
    pub fn price(&self) -> Option<f64> {
        (self.token_amount > 0.0 && self.sol_amount > 0.0)
            .then(|| self.sol_amount / self.token_amount)
            .filter(|p| p.is_finite() && *p > 0.0)
    }
    /// Sellable SOL in the bonding curve, i.e. the virtual reserve less the
    /// seed offset. Returns 0 — meaning *unknown*, which blocks paper entry —
    /// for any venue whose reserves this feed does not report.
    pub fn liquidity_sol(&self) -> f64 {
        if self.pool != "pump" {
            return 0.0;
        }
        (self.virtual_reserve_sol - VIRTUAL_SOL_OFFSET).max(0.0)
    }
    /// Fraction of the way to migration, where that is observable.
    pub fn graduation_progress(&self) -> Option<f64> {
        (self.pool == "pump").then(|| (self.liquidity_sol() / GRADUATION_SOL).clamp(0.0, 1.0))
    }
    /// Normalized form for the analytics and paper engines.
    ///
    /// `fee_sol` is zero because this feed does not report the network fee. That
    /// is "not observed", not "free": the paper policy prices costs separately
    /// through `fees::CostModel`, so the fee is not silently dropped.
    pub fn to_trade(&self, at: i64) -> Option<crate::analytics::Trade> {
        let price = self.price()?;
        let short = format!(
            "{}…{}",
            &self.token[..4],
            &self.token[self.token.len() - 4..]
        );
        let trade = crate::analytics::Trade {
            id: format!("live:{}:{}", self.signature, self.wallet),
            wallet: self.wallet.clone(),
            token: self.token.clone(),
            symbol: short,
            side: self.side.clone(),
            quantity: self.token_amount,
            price_sol: price,
            fee_sol: 0.0,
            timestamp: at,
            liquidity_sol: self.liquidity_sol(),
            // The firehose reports the venue but not how the trader reached it.
            routed: None,
        };
        trade.validate().then_some(trade)
    }
}

/// Strict parse. Anything missing a field, carrying a nonfinite number, or holding
/// an address that is not a real public key is discarded rather than guessed at.
pub fn parse(v: &Value) -> Option<Event> {
    let mint = v["mint"].as_str()?;
    if !crate::rpc::valid_address(mint) {
        return None;
    }
    let kind = v["txType"].as_str()?;
    if kind == "create" {
        let text = |key: &str, max: usize| {
            v[key]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= max)
                .map(str::to_owned)
        };
        return Some(Event::Created {
            mint: mint.into(),
            name: text("name", 200),
            symbol: text("symbol", 40),
            uri: text("uri", 500),
            creator: text("traderPublicKey", 64),
        });
    }
    if !matches!(kind, "buy" | "sell") {
        return None;
    }
    let wallet = v["traderPublicKey"].as_str()?;
    if !crate::rpc::valid_address(wallet) {
        return None;
    }
    let signature = v["signature"].as_str().filter(|s| s.len() <= 128)?;
    let sol_amount = v["solAmount"].as_f64()?;
    let token_amount = v["tokenAmount"].as_f64().unwrap_or(0.0);
    let virtual_reserve_sol = v["vSolInBondingCurve"].as_f64().unwrap_or(0.0);
    let pool = v["pool"].as_str().unwrap_or("unknown");
    if !sol_amount.is_finite()
        || !token_amount.is_finite()
        || !virtual_reserve_sol.is_finite()
        || !(0.0..=1e9).contains(&sol_amount)
        || !(0.0..=1e24).contains(&token_amount)
        || !(0.0..=1e12).contains(&virtual_reserve_sol)
    {
        return None;
    }
    Some(Event::Trade(Observation {
        wallet: wallet.into(),
        token: mint.into(),
        side: kind.into(),
        sol_amount,
        token_amount,
        virtual_reserve_sol,
        signature: signature.into(),
        pool: pool.into(),
    }))
}

/// Accumulates one observation. Counters are monotonic and deduplicated by
/// signature so a reconnect that replays a message cannot inflate a wallet's size.
pub fn record(db: &Connection, o: &Observation, seen: i64) -> Result<bool, rusqlite::Error> {
    if db.execute(
        "INSERT OR IGNORE INTO candidate_events VALUES(?1,?2,?3)",
        params![o.signature, o.wallet, seen],
    )? == 0
    {
        return Ok(false);
    }
    db.execute("INSERT INTO candidate_wallets(wallet,first_seen,last_seen,buys,sells,sol_volume,max_trade_sol,source) VALUES(?1,?2,?2,?3,?4,?5,?5,'pumpportal') ON CONFLICT(wallet) DO UPDATE SET last_seen=?2,buys=buys+?3,sells=sells+?4,sol_volume=sol_volume+?5,max_trade_sol=MAX(max_trade_sol,?5)",
        params![o.wallet, seen, i64::from(o.side == "buy"), i64::from(o.side == "sell"), o.sol_amount])?;
    db.execute(
        "INSERT OR IGNORE INTO candidate_tokens VALUES(?1,?2,?3)",
        params![o.wallet, o.token, seen],
    )?;
    Ok(true)
}

/// Records a newly created mint and, when its metadata names one, the X
/// account behind it.
///
/// This runs on the *free* creation stream, so the reuse ledger grows at no
/// cost. Metadata is fetched separately and may fail; a mint with no resolved
/// socials is recorded without them rather than skipped.
pub fn record_token(
    db: &Connection,
    mint: &str,
    name: Option<&str>,
    symbol: Option<&str>,
    uri: Option<&str>,
    creator: Option<&str>,
    at: i64,
) -> Result<(), rusqlite::Error> {
    db.execute(
        "INSERT OR IGNORE INTO tokens(mint,name,symbol,uri,creator,first_seen) VALUES(?1,?2,?3,?4,?5,?6)",
        params![mint, name, symbol, uri, creator, at],
    )?;
    Ok(())
}

/// Stores fetched off-chain metadata and links the social account.
pub fn record_metadata(
    db: &Connection,
    mint: &str,
    meta: &Value,
    at: i64,
) -> Result<Option<String>, rusqlite::Error> {
    let field = |key: &str| {
        meta[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 500)
    };
    let twitter = field("twitter");
    db.execute(
        "UPDATE tokens SET twitter=?1,telegram=?2,website=?3,metadata_fetched=?4 WHERE mint=?5",
        params![twitter, field("telegram"), field("website"), at, mint],
    )?;
    let handle = twitter.and_then(crate::social::handle);
    if let Some(handle) = &handle {
        crate::social::link(db, mint, handle, at)?;
    }
    Ok(handle)
}

/// Fetches a token's off-chain metadata document. Creators host these anywhere,
/// so the response is size-capped and a failure is an error, never an empty
/// document that would read as "this token has no socials".
pub async fn fetch_metadata(uri: &str) -> Result<Value, String> {
    if !uri.starts_with("https://") {
        return Err("Metadata URI is not https".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "Cannot create metadata client")?;
    let response = client
        .get(uri)
        .send()
        .await
        .map_err(|_| "Metadata fetch failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Metadata returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let body = response.text().await.map_err(|_| "Unreadable metadata")?;
    if body.len() > 200_000 {
        return Err("Metadata document is implausibly large".into());
    }
    serde_json::from_str(&body).map_err(|_| "Metadata is not JSON".into())
}

/// Discards observation and tick rows older than the retention window so an
/// always-on collector cannot grow the research database without bound.
pub fn prune(db: &Connection, before: i64) -> Result<usize, rusqlite::Error> {
    let events = db.execute("DELETE FROM candidate_events WHERE seen<?1", [before])?;
    db.execute("DELETE FROM price_ticks WHERE timestamp<?1", [before])?;
    Ok(events)
}

/// Commits one observation and the exit decisions it implies, atomically.
///
/// Every trade contributes a price tick, including trades below `min_sol`: a
/// small print is poor evidence of a skilled trader but perfectly good evidence
/// of where the token is trading, and exits need the latter.
#[derive(Default)]
pub struct Outcome {
    pub tick_exits: usize,
    pub stored: bool,
    pub opened: usize,
    pub closed: usize,
}

pub fn ingest(
    db: &mut Connection,
    o: &Observation,
    seen: i64,
    cfg: &Config,
    policy: &crate::ticks::Policy,
) -> Result<Outcome, String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let mut outcome = Outcome::default();
    if o.sol_amount >= cfg.min_sol {
        record(&tx, o, seen).map_err(|e| e.to_string())?;
    }
    if let Some(price) = o.price() {
        crate::ticks::record(&tx, &o.token, price, seen, "discovery").map_err(|e| e.to_string())?;
        let closed =
            crate::ticks::apply(&tx, &o.token, price, seen, o.graduation_progress(), policy)
                .map_err(|e| e.to_string())?;
        for c in &closed {
            crate::audit(
                &tx,
                &c.user_id,
                "paper.tick_exit",
                &format!("{}: {}", c.id, c.reason),
            )
            .map_err(|e| e.to_string())?;
        }
        outcome.tick_exits = closed.len();
    }
    // Live history: real observed trades become analyzable events, and eligible
    // followers are evaluated against them exactly as they are for an
    // administrator-supplied event. Still paper: nothing is signed.
    if cfg.ingest_trades
        && let Some(trade) = o.to_trade(seen)
    {
        let stored = tx
            .execute(
                "INSERT OR IGNORE INTO trades VALUES(?1,?2,?3)",
                params![
                    trade.id,
                    trade.timestamp,
                    serde_json::to_string(&trade).unwrap()
                ],
            )
            .map_err(|e| e.to_string())?
            > 0;
        outcome.stored = stored;
        if stored {
            let (_, opened, closed, _) =
                crate::fanout(&tx, &trade).map_err(|_| "follower evaluation failed".to_string())?;
            outcome.opened = opened;
            outcome.closed = closed;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(outcome)
}

fn subscribe(method: &str, keys: &[String]) -> String {
    json!({"method":method,"keys":keys}).to_string()
}

/// Long-running collector. Subscribes to new mints, follows their trades for a
/// bounded window, and records wallets trading at or above the configured size.
///
/// Every failure path reconnects with backoff. A dropped connection loses
/// observations; it never produces invented ones, and the gap is not recorded as
/// an absence of trading.
pub async fn run(app: crate::App, cfg: Config) {
    let mut backoff = 1u64;
    loop {
        match session(&app, &cfg).await {
            Ok(()) => backoff = 1,
            Err(e) => eprintln!("discovery: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(120);
    }
}

/// Records what the collector is actually doing, so an operator can tell a quiet
/// market from a refused subscription.
pub fn set_status(db: &Connection, state: &str, detail: &str, at: i64) {
    let _ = db.execute(
        "INSERT INTO collector_status VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,detail=excluded.detail,updated=excluded.updated",
        params![state, detail, at],
    );
}

fn endpoint(cfg: &Config) -> String {
    match &cfg.api_key {
        Some(key) => format!("{}?api-key={key}", cfg.url),
        None => cfg.url.clone(),
    }
}

async fn session(app: &crate::App, cfg: &Config) -> Result<(), String> {
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(cfg))
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    socket
        .send(json!({"method":"subscribeNewToken"}).to_string().into())
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;
    {
        let db = app.lock().unwrap();
        let detail = if cfg.api_key.is_some() {
            "Connected with an API key; trade subscriptions requested"
        } else {
            "Connected without an API key, so only token creations are collected. Trade subscriptions are not attempted: the provider meters them and refuses them without a funded key. New launches are still recorded."
        };
        set_status(&db, "connected", detail, crate::now());
    }
    let mut tracked: VecDeque<String> = VecDeque::new();
    let mut last_notice: Option<String> = None;
    let mut last_prune = crate::now();
    let policy = crate::ticks::Policy::from_env();
    while let Some(message) = socket.next().await {
        let message = message.map_err(|e| format!("stream error: {e}"))?;
        let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        // Provider notices arrive as ordinary frames. A refused subscription is
        // the difference between "no trades happened" and "we were not allowed
        // to see them", so it is recorded rather than ignored.
        if let Some(notice) = value["message"].as_str() {
            let refused = notice.contains("API key");
            // The provider repeats itself per request; we do not.
            if last_notice.as_deref() != Some(notice) {
                println!("discovery: upstream says: {notice}");
                last_notice = Some(notice.to_string());
            }
            let db = app.lock().unwrap();
            set_status(
                &db,
                if refused { "refused" } else { "connected" },
                notice,
                crate::now(),
            );
            continue;
        }
        match parse(&value) {
            Some(Event::Created {
                mint,
                name,
                symbol,
                uri,
                creator,
            }) => {
                {
                    let db = app.lock().unwrap();
                    if let Err(e) = record_token(
                        &db,
                        &mint,
                        name.as_deref(),
                        symbol.as_deref(),
                        uri.as_deref(),
                        creator.as_deref(),
                        crate::now(),
                    ) {
                        eprintln!("discovery: token record failed: {e}");
                    }
                }
                if let Some(uri) = uri.clone()
                    && let Ok(meta) = fetch_metadata(&uri).await
                {
                    let db = app.lock().unwrap();
                    let _ = record_metadata(&db, &mint, &meta, crate::now());
                }
                // Without a key the provider refuses trade subscriptions, so
                // asking for one per new mint would produce a refusal per mint
                // and nothing else. Creation events are free and still useful.
                if cfg.api_key.is_none() || tracked.contains(&mint) {
                    continue;
                }
                socket
                    .send(subscribe("subscribeTokenTrade", std::slice::from_ref(&mint)).into())
                    .await
                    .map_err(|e| format!("track failed: {e}"))?;
                tracked.push_back(mint);
                if tracked.len() > cfg.tracked_tokens
                    && let Some(old) = tracked.pop_front()
                {
                    socket
                        .send(subscribe("unsubscribeTokenTrade", std::slice::from_ref(&old)).into())
                        .await
                        .map_err(|e| format!("untrack failed: {e}"))?;
                }
            }
            Some(Event::Trade(o)) => {
                let seen = crate::now();
                let mut db = app.lock().unwrap();
                match ingest(&mut db, &o, seen, cfg, &policy) {
                    Ok(r) if r.opened + r.closed + r.tick_exits > 0 => println!(
                        "discovery: {} opened, {} closed, {} tick exits",
                        r.opened, r.closed, r.tick_exits
                    ),
                    Ok(_) => {}
                    Err(e) => eprintln!("discovery: ingest failed: {e}"),
                }
                if seen - last_prune > 3600 {
                    last_prune = seen;
                    let _ = prune(&db, seen - 604800);
                }
            }
            _ => {}
        }
    }
    Err("stream closed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const WALLET: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
    const MINT: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
    fn trade(sig: &str, side: &str, sol: f64) -> Value {
        json!({"signature":sig,"mint":MINT,"traderPublicKey":WALLET,"txType":side,"solAmount":sol,"tokenAmount":sol*1000.0,"vSolInBondingCurve":31.5,"pool":"pump"})
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::DISCOVERY_SCHEMA).unwrap();
        db.execute_batch(crate::TICK_SCHEMA).unwrap();
        db
    }
    #[test]
    fn parses_creation_and_trades() {
        assert_eq!(
            parse(&json!({"mint":MINT,"txType":"create","name":"Coin","symbol":"CN"})),
            Some(Event::Created {
                mint: MINT.into(),
                name: Some("Coin".into()),
                symbol: Some("CN".into()),
                uri: None,
                creator: None
            })
        );
        let Some(Event::Trade(o)) = parse(&trade("sig", "buy", 4.5)) else {
            panic!("expected a trade")
        };
        assert_eq!(o.wallet, WALLET);
        assert_eq!(o.sol_amount, 4.5);
        assert_eq!(o.virtual_reserve_sol, 31.5);
        assert_eq!(o.price(), Some(0.001));
    }
    #[test]
    fn malformed_messages_are_discarded_not_defaulted() {
        assert!(parse(&json!({"txType":"buy"})).is_none());
        assert!(parse(&json!({"mint":"notanaddress","txType":"create"})).is_none());
        assert!(parse(&json!({"mint":MINT,"txType":"subscribed"})).is_none());
        let mut v = trade("sig", "buy", 1.0);
        v["traderPublicKey"] = json!("short");
        assert!(parse(&v).is_none());
        v = trade("sig", "buy", 1.0);
        v["solAmount"] = json!("4.5");
        assert!(parse(&v).is_none());
    }
    #[test]
    fn virtual_reserves_are_never_reported_as_sellable_liquidity() {
        let Some(Event::Trade(o)) = parse(&trade("s", "buy", 1.0)) else {
            panic!()
        };
        // 31.5 virtual less the 30 SOL seed leaves 1.5 SOL genuinely in the curve.
        assert_eq!(o.virtual_reserve_sol, 31.5);
        assert_eq!(o.liquidity_sol(), 1.5);
        assert!((o.graduation_progress().unwrap() - 1.5 / 85.0).abs() < 1e-12);
    }
    #[test]
    fn a_brand_new_curve_has_no_liquidity_at_all() {
        let mut v = trade("s", "buy", 1.0);
        v["vSolInBondingCurve"] = json!(30.0);
        let Some(Event::Trade(o)) = parse(&v) else {
            panic!()
        };
        assert_eq!(o.liquidity_sol(), 0.0, "the seed offset is not real money");
        // Never negative, however the feed reports it.
        v["vSolInBondingCurve"] = json!(5.0);
        let Some(Event::Trade(o)) = parse(&v) else {
            panic!()
        };
        assert_eq!(o.liquidity_sol(), 0.0);
    }
    #[test]
    fn a_migrated_pool_reports_unknown_liquidity_rather_than_a_guess() {
        let mut v = trade("s", "buy", 1.0);
        v["pool"] = json!("pump-amm");
        let Some(Event::Trade(o)) = parse(&v) else {
            panic!()
        };
        // This message does not carry migrated-pool reserves, and 0 means
        // unknown, which blocks paper entry.
        assert_eq!(o.liquidity_sol(), 0.0);
        assert_eq!(o.graduation_progress(), None);
        assert_eq!(o.to_trade(100).unwrap().liquidity_sol, 0.0);
    }
    #[test]
    fn a_live_observation_becomes_a_normalized_trade() {
        let mut v = trade("sig1", "buy", 2.0);
        v["vSolInBondingCurve"] = json!(80.0);
        let Some(Event::Trade(o)) = parse(&v) else {
            panic!()
        };
        let t = o.to_trade(1_700_000_000).unwrap();
        assert_eq!(t.id, format!("live:sig1:{WALLET}"));
        assert_eq!(t.wallet, WALLET);
        assert_eq!(t.side, "buy");
        assert_eq!(t.quantity, 2000.0);
        assert_eq!(t.price_sol, 0.001);
        assert_eq!(t.liquidity_sol, 50.0);
        // The feed does not report a network fee, so none is invented.
        assert_eq!(t.fee_sol, 0.0);
        assert_eq!(t.timestamp, 1_700_000_000);
        assert!(t.validate());
    }
    #[test]
    fn an_observation_without_both_legs_cannot_become_a_trade() {
        let mut v = trade("sig", "buy", 1.0);
        v["tokenAmount"] = json!(0.0);
        let Some(Event::Trade(o)) = parse(&v) else {
            panic!()
        };
        assert_eq!(o.price(), None);
        assert!(o.to_trade(100).is_none(), "a price cannot be half observed");
    }
    #[test]
    fn replayed_signatures_do_not_inflate_volume() {
        let db = db();
        let Some(Event::Trade(o)) = parse(&trade("dup", "buy", 3.0)) else {
            panic!()
        };
        assert!(record(&db, &o, 100).unwrap());
        assert!(!record(&db, &o, 100).unwrap());
        let (buys, volume): (i64, f64) = db
            .query_row(
                "SELECT buys,sol_volume FROM candidate_wallets WHERE wallet=?1",
                [WALLET],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(buys, 1);
        assert_eq!(volume, 3.0);
    }
    #[test]
    fn counters_accumulate_and_prune_is_bounded() {
        let db = db();
        for (i, (side, sol)) in [("buy", 2.0), ("sell", 5.0), ("buy", 1.0)]
            .into_iter()
            .enumerate()
        {
            let Some(Event::Trade(o)) = parse(&trade(&format!("s{i}"), side, sol)) else {
                panic!()
            };
            record(&db, &o, 100 + i as i64).unwrap();
        }
        let (buys, sells, volume, max): (i64, i64, f64, f64) = db
            .query_row(
                "SELECT buys,sells,sol_volume,max_trade_sol FROM candidate_wallets",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((buys, sells), (2, 1));
        assert_eq!((volume, max), (8.0, 5.0));
        assert_eq!(prune(&db, 102).unwrap(), 2);
    }
}

#[cfg(test)]
mod upstream_tests {
    use super::*;
    fn cfg(key: Option<&str>) -> Config {
        Config {
            url: "wss://example.test/api/data".into(),
            min_sol: 1.0,
            tracked_tokens: 10,
            ingest_trades: false,
            api_key: key.map(str::to_owned),
        }
    }
    #[test]
    fn the_api_key_is_attached_to_the_endpoint_when_present() {
        assert_eq!(endpoint(&cfg(None)), "wss://example.test/api/data");
        assert_eq!(
            endpoint(&cfg(Some("abc123"))),
            "wss://example.test/api/data?api-key=abc123"
        );
    }
    #[test]
    fn a_refused_subscription_is_recorded_rather_than_swallowed() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::DISCOVERY_SCHEMA).unwrap();
        // Exactly what the provider sends when the key is missing or unfunded.
        let notice = "'subscribeTokenTrade' and 'subscribeAccountTrade' methods are only available when connecting with an API key funded with at least 0.02 SOL.";
        set_status(&db, "refused", notice, 1000);
        let (state, detail): (String, String) = db
            .query_row("SELECT state,detail FROM collector_status", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(state, "refused");
        assert!(detail.contains("API key"));
        // Status is a single row: a later report replaces it.
        set_status(&db, "connected", "fine now", 2000);
        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM collector_status", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }
    #[test]
    fn a_provider_notice_is_never_mistaken_for_a_trade() {
        let notice = json!({"message":"Successfully subscribed to token creation events."});
        assert_eq!(parse(&notice), None);
    }
}
