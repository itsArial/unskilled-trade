//! Live copy feed: one `logsSubscribe` per followed wallet.
//!
//! Copy trading does not need a global firehose. It needs the wallets a user
//! actually follows, and that is a far smaller stream — tens of thousands of
//! notifications a day rather than millions — which fits inside the free tier of
//! every major Solana RPC provider. Volume scales with the follow list, not with
//! the market, so the cost does not grow when the market gets busy.
//!
//! What this gives up is discovery: a per-wallet subscription cannot find a
//! wallet nobody follows yet. That is deliberate. Finding a wallet an hour late
//! costs nothing; copying it an hour late costs everything. Discovery stays on
//! the free `subscribeNewToken` stream and on batch backfill.
//!
//! Decoding reuses `rpc::decode`, so the same conservative rules apply: one
//! direct Pump/PumpSwap instruction, one changed asset, native SOL on the other
//! side. Anything else is archived, not invented.
use futures_util::{SinkExt, StreamExt};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct Config {
    pub ws_url: String,
    pub rpc_url: String,
    /// Upper bound on concurrent subscriptions, so a large follow list cannot
    /// silently exceed a provider's connection limits.
    pub max_wallets: usize,
}
impl Config {
    pub fn from_env() -> Option<Self> {
        if std::env::var("WATCH_ENABLED").unwrap_or_default() != "true" {
            return None;
        }
        let rpc_url = crate::rpc_url();
        // Most providers serve the websocket on the same host over wss.
        let ws_url = std::env::var("SOLANA_WS_URL").unwrap_or_else(|_| {
            rpc_url
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1)
        });
        Some(Self {
            ws_url,
            rpc_url,
            max_wallets: std::env::var("WATCH_MAX_WALLETS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &usize| (1..=500).contains(v))
                .unwrap_or(50),
        })
    }
}

pub fn set_status(db: &Connection, state: &str, detail: &str, at: i64) {
    let _ = db.execute(
        "INSERT INTO watch_status VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,detail=excluded.detail,updated=excluded.updated",
        params![state, detail, at],
    );
}

/// Wallets worth subscribing to: followed by at least one account that could
/// actually act on the event. Subscribing for a disabled or banned user would
/// spend provider quota on a signal nobody can trade.
pub fn followed(db: &Connection, limit: usize) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = db.prepare(
        "SELECT DISTINCT w.wallet FROM watches w JOIN users u ON u.id=w.user_id WHERE u.enabled=1 AND u.banned=0 ORDER BY w.wallet LIMIT ?1",
    )?;
    stmt.query_map([limit], |r| r.get(0))?.collect()
}

/// Extracts a usable signature from a `logsNotification`.
///
/// A transaction that failed on chain is not a trade. Returning `None` for it
/// keeps a reverted attempt out of the history entirely.
pub fn notification_signature(v: &Value) -> Option<(u64, String)> {
    if v["method"].as_str() != Some("logsNotification") {
        return None;
    }
    let params = &v["params"];
    let subscription = params["subscription"].as_u64()?;
    let value = &params["result"]["value"];
    if !value["err"].is_null() {
        return None;
    }
    let signature = value["signature"].as_str().filter(|s| s.len() <= 128)?;
    Some((subscription, signature.into()))
}

/// Pairs a subscription confirmation with the request that asked for it.
pub fn subscription_result(v: &Value) -> Option<(u64, u64)> {
    let id = v["id"].as_u64()?;
    let subscription = v["result"].as_u64()?;
    Some((id, subscription))
}

fn subscribe_frame(request_id: u64, wallet: &str) -> String {
    json!({"jsonrpc":"2.0","id":request_id,"method":"logsSubscribe",
        "params":[{"mentions":[wallet]},{"commitment":"confirmed"}]})
    .to_string()
}
fn unsubscribe_frame(request_id: u64, subscription: u64) -> String {
    json!({"jsonrpc":"2.0","id":request_id,"method":"logsUnsubscribe","params":[subscription]})
        .to_string()
}

pub async fn run(app: crate::App, cfg: Config) {
    let mut backoff = 1u64;
    loop {
        // With nothing followed there is nothing to subscribe to, and holding
        // an idle socket open only earns periodic resets from the provider.
        // Wait for work instead of reconnecting to do nothing.
        let watching = {
            let db = app.lock().unwrap();
            followed(&db, cfg.max_wallets).map(|w| w.len()).unwrap_or(0)
        };
        if watching == 0 {
            {
                let db = app.lock().unwrap();
                set_status(
                    &db,
                    "idle",
                    "No wallets are followed, so no subscription is open.",
                    crate::now(),
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            continue;
        }
        match session(&app, &cfg).await {
            Ok(()) => backoff = 1,
            Err(e) => {
                // Providers drop idle sockets routinely; the reconnect loop
                // handles it and the detail lands in status, not the console.
                if !e.contains("Connection reset") {
                    eprintln!("watch: {e}");
                }
                let db = app.lock().unwrap();
                set_status(&db, "disconnected", &e, crate::now());
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(120);
    }
}

async fn session(app: &crate::App, cfg: &Config) -> Result<(), String> {
    let (mut socket, _) = tokio_tungstenite::connect_async(&cfg.ws_url)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let mut next_request = 1u64;
    // wallet -> subscription id, and the reverse for routing notifications.
    let mut active: BTreeMap<String, u64> = BTreeMap::new();
    let mut pending: BTreeMap<u64, String> = BTreeMap::new();
    let mut routes: BTreeMap<u64, String> = BTreeMap::new();
    let mut reconcile = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        tokio::select! {
            _ = reconcile.tick() => {
                let wanted = {
                    let db = app.lock().unwrap();
                    followed(&db, cfg.max_wallets).map_err(|e| e.to_string())?
                };
                for wallet in &wanted {
                    if active.contains_key(wallet) || pending.values().any(|w| w == wallet) {
                        continue;
                    }
                    next_request += 1;
                    pending.insert(next_request, wallet.clone());
                    socket
                        .send(subscribe_frame(next_request, wallet).into())
                        .await
                        .map_err(|e| format!("subscribe failed: {e}"))?;
                }
                // Stop paying attention to wallets nobody follows any more.
                let stale: Vec<String> = active
                    .keys()
                    .filter(|w| !wanted.contains(w))
                    .cloned()
                    .collect();
                for wallet in stale {
                    if let Some(subscription) = active.remove(&wallet) {
                        routes.remove(&subscription);
                        next_request += 1;
                        socket
                            .send(unsubscribe_frame(next_request, subscription).into())
                            .await
                            .map_err(|e| format!("unsubscribe failed: {e}"))?;
                    }
                }
                let db = app.lock().unwrap();
                set_status(
                    &db,
                    "connected",
                    &format!("{} of {} followed wallets subscribed", active.len(), wanted.len()),
                    crate::now(),
                );
            }
            message = socket.next() => {
                let Some(message) = message else { return Err("stream closed".into()) };
                let message = message.map_err(|e| format!("stream error: {e}"))?;
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else { continue };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(error) = value.get("error") {
                    let detail = error["message"].as_str().unwrap_or("unknown RPC error");
                    eprintln!("watch: provider rejected a request: {detail}");
                    let db = app.lock().unwrap();
                    set_status(&db, "rejected", detail, crate::now());
                    continue;
                }
                if let Some((request_id, subscription)) = subscription_result(&value)
                    && let Some(wallet) = pending.remove(&request_id)
                {
                    routes.insert(subscription, wallet.clone());
                    active.insert(wallet, subscription);
                    continue;
                }
                let Some((subscription, signature)) = notification_signature(&value) else { continue };
                let Some(wallet) = routes.get(&subscription).cloned() else { continue };
                if let Err(e) = handle(app, cfg, &wallet, &signature).await {
                    eprintln!("watch: {wallet} {signature}: {e}");
                }
            }
        }
    }
}

/// Fetches, decodes and distributes one observed transaction.
///
/// A transaction that the conservative decoder does not recognise is archived
/// and dropped, never guessed at. Duplicate signatures are ignored by the
/// trades primary key, so a replayed notification cannot double-count.
async fn handle(
    app: &crate::App,
    cfg: &Config,
    wallet: &str,
    signature: &str,
) -> Result<(), String> {
    {
        let db = app.lock().unwrap();
        let seen: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM raw_transactions WHERE signature=?1 AND wallet=?2)",
                params![signature, wallet],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if seen {
            return Ok(());
        }
    }
    let raw = crate::rpc::transaction(&cfg.rpc_url, signature).await?;
    let decoded = crate::rpc::decode(wallet, signature, &raw);
    let mut db = app.lock().unwrap();
    let tx = db.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO raw_transactions VALUES(?1,?2,?3,?4)",
        params![signature, wallet, raw.to_string(), crate::now()],
    )
    .map_err(|e| e.to_string())?;
    if let Some(trade) = decoded {
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
        if stored {
            let policy = crate::ticks::Policy::from_env();
            if trade.price_sol > 0.0 {
                crate::ticks::record(&tx, &trade.token, trade.price_sol, trade.timestamp, "watch")
                    .map_err(|e| e.to_string())?;
                let closed = crate::ticks::apply(
                    &tx,
                    &trade.token,
                    trade.price_sol,
                    trade.timestamp,
                    None,
                    &policy,
                )
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
            }
            let (_, opened, closed, _) =
                crate::fanout(&tx, &trade).map_err(|_| "follower evaluation failed".to_string())?;
            if opened + closed > 0 {
                println!(
                    "watch: {wallet} {} -> {opened} opened, {closed} closed",
                    trade.symbol
                );
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_frames_target_one_wallet_each() {
        let frame: Value = serde_json::from_str(&subscribe_frame(7, "WALLET")).unwrap();
        assert_eq!(frame["method"], "logsSubscribe");
        assert_eq!(frame["id"], 7);
        assert_eq!(frame["params"][0]["mentions"][0], "WALLET");
        assert_eq!(frame["params"][0]["mentions"].as_array().unwrap().len(), 1);
        assert_eq!(frame["params"][1]["commitment"], "confirmed");
        let off: Value = serde_json::from_str(&unsubscribe_frame(8, 42)).unwrap();
        assert_eq!(off["method"], "logsUnsubscribe");
        assert_eq!(off["params"][0], 42);
    }
    #[test]
    fn a_confirmation_is_paired_with_its_request() {
        let v = json!({"jsonrpc":"2.0","result":24040,"id":7});
        assert_eq!(subscription_result(&v), Some((7, 24040)));
        // A notification is not a confirmation.
        assert_eq!(
            subscription_result(&json!({"method":"logsNotification","params":{}})),
            None
        );
    }
    fn notification(signature: &str, err: Value) -> Value {
        json!({"jsonrpc":"2.0","method":"logsNotification","params":{
            "subscription":24040,
            "result":{"context":{"slot":1},"value":{"signature":signature,"err":err,"logs":[]}}}})
    }
    #[test]
    fn a_successful_notification_yields_its_signature() {
        assert_eq!(
            notification_signature(&notification("sig123", Value::Null)),
            Some((24040, "sig123".to_string()))
        );
    }
    #[test]
    fn a_reverted_transaction_never_becomes_history() {
        let failed = notification("sig123", json!({"InstructionError":[0,"Custom"]}));
        assert_eq!(notification_signature(&failed), None);
    }
    #[test]
    fn unrelated_frames_are_ignored() {
        assert_eq!(notification_signature(&json!({"result":1,"id":2})), None);
        assert_eq!(notification_signature(&json!({})), None);
        // Missing signature is not a usable notification.
        let mut v = notification("s", Value::Null);
        v["params"]["result"]["value"]["signature"] = Value::Null;
        assert_eq!(notification_signature(&v), None);
    }
    #[test]
    fn only_wallets_someone_can_act_on_are_subscribed() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE users(id TEXT PRIMARY KEY,enabled INTEGER,banned INTEGER);
             CREATE TABLE watches(user_id TEXT,wallet TEXT,PRIMARY KEY(user_id,wallet));",
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO users VALUES('active',1,0),('paused',0,0),('banned',1,1);
             INSERT INTO watches VALUES('active','WATCHED'),('paused','IGNORED'),('banned','ALSO_IGNORED'),('active','SECOND');",
        )
        .unwrap();
        assert_eq!(
            followed(&db, 50).unwrap(),
            vec!["SECOND".to_string(), "WATCHED".to_string()]
        );
        // The cap bounds provider usage regardless of follow-list size.
        assert_eq!(followed(&db, 1).unwrap().len(), 1);
    }
}
