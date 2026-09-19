mod analytics;
mod chart;
mod cluster;
mod custody;
mod discovery;
mod fees;
mod holdings;
mod market;
mod risk_token;
mod rpc;
mod social;
mod swap;
mod ticks;
mod transfer;
mod velocity;
mod wallet_auth;
mod watch;
use analytics::{Trade, analyze, copyability};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

type App = Arc<Mutex<Connection>>;
type ApiResult = Result<Json<Value>, ApiError>;
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        eprintln!("database error: {e}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database operation failed".into(),
        )
    }
}
fn bad(s: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, s.into())
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
/// Under the commission model access is not sold: the platform earns from a cut
/// of results instead, so an account with no subscription is still entitled.
/// Minimum supplied liquidity for a paper entry.
///
/// The 100 SOL default predates live curve data and is structurally
/// unreachable for a pre-graduation Pump.fun token, which migrates at about 85
/// SOL of real reserve. Lower it deliberately to trade the curve, and read the
/// capacity warning in `METHODOLOGY.md` before doing so.
fn min_liquidity_sol() -> f64 {
    std::env::var("MIN_LIQUIDITY_SOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &f64| v.is_finite() && *v > 0.0)
        .unwrap_or(100.0)
}
/// Copy trading is a separate product from swapping, and an operator may not
/// want to be in it. When it is off the feature is genuinely unavailable —
/// every path refuses — rather than merely hidden in the interface.
fn copy_trading_enabled(db: &Connection) -> bool {
    db.query_row(
        "SELECT value FROM platform_settings WHERE key='copy_trading'",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(|v| v != "off")
    .unwrap_or(true)
}
fn require_copy_trading(db: &Connection) -> Result<(), ApiError> {
    if copy_trading_enabled(db) {
        return Ok(());
    }
    Err(ApiError(
        StatusCode::CONFLICT,
        "Copy trading is switched off on this platform. Swapping is unaffected.".into(),
    ))
}
fn require_subscription(u: &User, what: &str) -> Result<(), ApiError> {
    if entitled(u) {
        return Ok(());
    }
    Err(ApiError(
        StatusCode::PAYMENT_REQUIRED,
        format!("An active subscription is required for {what}"),
    ))
}
fn commission_access() -> bool {
    std::env::var("ACCESS_MODEL").unwrap_or_default() == "commission"
}
fn entitled(u: &User) -> bool {
    commission_access() || u.expires > now()
}
fn audit(db: &Connection, user: &str, action: &str, detail: &str) -> Result<(), rusqlite::Error> {
    db.execute(
        "INSERT INTO logs(user_id,action,detail,created) VALUES(?1,?2,?3,?4)",
        params![user, action, detail, now()],
    )?;
    Ok(())
}
#[derive(Clone)]
struct User {
    id: String,
    email: String,
    admin: bool,
    expires: i64,
}
fn auth(db: &Connection, h: &HeaderMap, admin: bool) -> Result<User, ApiError> {
    let cookie = h
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = cookie
        .split(';')
        .find_map(|v| v.trim().strip_prefix("session="));
    let bearer = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let key = token.or(bearer).ok_or(ApiError(
        StatusCode::UNAUTHORIZED,
        "Sign in to continue".into(),
    ))?;
    let u=db.query_row("SELECT u.id,u.email,u.admin,u.expires FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.token=?1 AND s.expires>?2 AND u.banned=0 AND (?3=0 OR s.kind='session')",params![hash(key),now(),admin],|r|Ok(User{id:r.get(0)?,email:r.get(1)?,admin:r.get(2)?,expires:r.get(3)?})).optional()?.ok_or(ApiError(StatusCode::UNAUTHORIZED,"Session expired or access revoked".into()))?;
    if admin && !u.admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator access required".into(),
        ));
    }
    Ok(u)
}
fn trades(db: &Connection) -> Result<Vec<Trade>, ApiError> {
    let mut s = db.prepare("SELECT data FROM trades ORDER BY timestamp,id")?;
    let strings = s
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    strings
        .into_iter()
        .map(|s| serde_json::from_str(&s).map_err(|_| bad("Stored trade could not be decoded")))
        .collect()
}
fn rows(db: &Connection, sql: &str) -> Result<Vec<Value>, ApiError> {
    let mut stmt = db.prepare(sql)?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let result = stmt
        .query_map([], |row| {
            let mut v = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                let val = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Real(n) => json!(n),
                    rusqlite::types::ValueRef::Text(s) => json!(String::from_utf8_lossy(s)),
                    _ => Value::Null,
                };
                v.insert(name.clone(), val);
            }
            Ok(Value::Object(v))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(result)
}
// Additive-only schema. Discovery tables are separated so tests can build the
// collector's storage without the full application schema.
const DISCOVERY_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS candidate_wallets(wallet TEXT PRIMARY KEY,first_seen INTEGER NOT NULL,last_seen INTEGER NOT NULL,buys INTEGER NOT NULL DEFAULT 0,sells INTEGER NOT NULL DEFAULT 0,sol_volume REAL NOT NULL DEFAULT 0,max_trade_sol REAL NOT NULL DEFAULT 0,source TEXT NOT NULL,promoted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS candidate_tokens(wallet TEXT NOT NULL,token TEXT NOT NULL,first_seen INTEGER NOT NULL,PRIMARY KEY(wallet,token));
    CREATE TABLE IF NOT EXISTS candidate_events(signature TEXT PRIMARY KEY,wallet TEXT NOT NULL,seen INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS candidate_events_seen ON candidate_events(seen);
    CREATE TABLE IF NOT EXISTS collector_status(id INTEGER PRIMARY KEY CHECK(id=1),state TEXT NOT NULL,detail TEXT NOT NULL,updated INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS watch_status(id INTEGER PRIMARY KEY CHECK(id=1),state TEXT NOT NULL,detail TEXT NOT NULL,updated INTEGER NOT NULL);";

// A row here is a decision record with a collection time, not a permanent verdict.
// `blocked_reason` NULL means the token cleared the gate when it was checked.
// Money owed, in integer lamports. `reference` is the idempotency key: a
// replayed settlement or event cannot bill the same thing twice.
const FEE_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS fee_ledger(id TEXT PRIMARY KEY,user_id TEXT NOT NULL,kind TEXT NOT NULL,lamports INTEGER NOT NULL,basis_lamports INTEGER NOT NULL,reference TEXT UNIQUE NOT NULL,created INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS fee_ledger_user ON fee_ledger(user_id);
    CREATE TABLE IF NOT EXISTS fee_state(user_id TEXT PRIMARY KEY,high_water_lamports INTEGER NOT NULL DEFAULT 0,last_settled INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS auth_challenges(nonce TEXT PRIMARY KEY,address TEXT NOT NULL,chain TEXT NOT NULL,expires INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS withdrawals(id TEXT PRIMARY KEY,user_id TEXT NOT NULL,destination TEXT NOT NULL,lamports INTEGER NOT NULL,signature TEXT,status TEXT NOT NULL,detail TEXT,created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS wallet_identities(address TEXT NOT NULL,chain TEXT NOT NULL,user_id TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(address,chain));";

// Token identity and the social account behind it. `token_socials` is the
// zero-cost reuse ledger: it grows from our own observations, and a handle that
// appears against many mints is visible without any third-party call.
const SOCIAL_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS tokens(mint TEXT PRIMARY KEY,name TEXT,symbol TEXT,uri TEXT,creator TEXT,twitter TEXT,telegram TEXT,website TEXT,first_seen INTEGER NOT NULL,metadata_fetched INTEGER);
    CREATE TABLE IF NOT EXISTS token_socials(mint TEXT NOT NULL,handle TEXT NOT NULL,first_seen INTEGER NOT NULL,PRIMARY KEY(mint,handle));
    CREATE INDEX IF NOT EXISTS token_socials_handle ON token_socials(handle);
    CREATE TABLE IF NOT EXISTS x_accounts(user_id TEXT PRIMARY KEY,handle TEXT NOT NULL,followers INTEGER,account_created INTEGER,first_seen INTEGER NOT NULL,last_seen INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS x_accounts_handle ON x_accounts(handle);
    CREATE TABLE IF NOT EXISTS swaps(id TEXT PRIMARY KEY,user_id TEXT NOT NULL,taker TEXT NOT NULL,input_mint TEXT NOT NULL,output_mint TEXT NOT NULL,in_amount INTEGER NOT NULL,out_amount INTEGER NOT NULL,minimum_out INTEGER NOT NULL,platform_fee INTEGER NOT NULL DEFAULT 0,slippage_bps INTEGER NOT NULL,status TEXT NOT NULL,signature TEXT,created INTEGER NOT NULL,settled INTEGER);
    CREATE INDEX IF NOT EXISTS swaps_user ON swaps(user_id);
    CREATE TABLE IF NOT EXISTS x_handle_history(user_id TEXT NOT NULL,handle TEXT NOT NULL,first_seen INTEGER NOT NULL,PRIMARY KEY(user_id,handle));";

// Cached market quotes. Age is served with the data so the interface can say
// how stale it is rather than implying it is live.
// Operator switches that belong in the admin panel rather than the environment,
// because they are turned on and off in response to how the product is doing.
const SETTINGS_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS platform_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL,updated INTEGER NOT NULL);";

const MARKET_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS market_cache(mint TEXT PRIMARY KEY,data TEXT NOT NULL,fetched INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS chart_cache(key TEXT PRIMARY KEY,data TEXT NOT NULL,fetched INTEGER NOT NULL);";

// Managed wallets. `secret` holds an authenticated ciphertext, never a key, and
// is emptied on release so custody genuinely ends.
const CUSTODY_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS managed_wallets(user_id TEXT PRIMARY KEY,address TEXT UNIQUE NOT NULL,secret TEXT NOT NULL,created INTEGER NOT NULL,released INTEGER NOT NULL DEFAULT 0,exports INTEGER NOT NULL DEFAULT 0,last_export INTEGER);";

// Funding edges are observations extracted from archived transactions. An absent
// edge means "not observed", never "did not happen".
const CLUSTER_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS funding_edges(signature TEXT NOT NULL,source TEXT NOT NULL,destination TEXT NOT NULL,lamports INTEGER NOT NULL,timestamp INTEGER NOT NULL,PRIMARY KEY(signature,source,destination));
    CREATE INDEX IF NOT EXISTS funding_edges_destination ON funding_edges(destination);
    CREATE INDEX IF NOT EXISTS funding_edges_source ON funding_edges(source);";

// Ticks are an independent price path, so an exit no longer depends on the
// leader trading again. One observation per token per second is retained.
const TICK_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS price_ticks(token TEXT NOT NULL,timestamp INTEGER NOT NULL,price_sol REAL NOT NULL,source TEXT NOT NULL,PRIMARY KEY(token,timestamp));";

const RISK_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS token_risk(mint TEXT PRIMARY KEY,score REAL,blocked_reason TEXT,data TEXT NOT NULL,checked INTEGER NOT NULL);";

fn setup(path: &str, email: &str, password: &str) -> Connection {
    let db = Connection::open(path).expect("open database");
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
    CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY,email TEXT UNIQUE NOT NULL,password TEXT NOT NULL,admin INTEGER NOT NULL DEFAULT 0,banned INTEGER NOT NULL DEFAULT 0,expires INTEGER NOT NULL DEFAULT 0,enabled INTEGER NOT NULL DEFAULT 0,budget REAL NOT NULL DEFAULT 0.1,created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS sessions(token TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),expires INTEGER NOT NULL,kind TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS logs(id INTEGER PRIMARY KEY,user_id TEXT,action TEXT,detail TEXT,created INTEGER);
    CREATE TABLE IF NOT EXISTS raw_transactions(signature TEXT,wallet TEXT,data TEXT,created INTEGER,PRIMARY KEY(signature,wallet));
    CREATE TABLE IF NOT EXISTS rpc_syncs(wallet TEXT PRIMARY KEY,last_attempt INTEGER);
    CREATE TABLE IF NOT EXISTS trades(id TEXT PRIMARY KEY,timestamp INTEGER NOT NULL,data TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS watches(user_id TEXT REFERENCES users(id),wallet TEXT,PRIMARY KEY(user_id,wallet));
    CREATE TABLE IF NOT EXISTS positions(id TEXT PRIMARY KEY,user_id TEXT REFERENCES users(id),wallet TEXT,token TEXT,symbol TEXT,quantity REAL,cost REAL,entry REAL,exit REAL,pnl REAL,status TEXT,reason TEXT,created INTEGER,closed INTEGER);
    CREATE TABLE IF NOT EXISTS processed(user_id TEXT,event_id TEXT,PRIMARY KEY(user_id,event_id));
    CREATE TABLE IF NOT EXISTS plans(id TEXT PRIMARY KEY,name TEXT,price_cents INTEGER,days INTEGER,active INTEGER);
    CREATE TABLE IF NOT EXISTS payments(id TEXT PRIMARY KEY,user_id TEXT,plan_id TEXT,amount_cents INTEGER,reference TEXT UNIQUE,created INTEGER);
    CREATE TABLE IF NOT EXISTS login_attempts(email TEXT PRIMARY KEY,attempts INTEGER,reset INTEGER);
    INSERT OR IGNORE INTO plans VALUES('observer','Observer',1900,30,1),('operator','Operator',4900,30,1),('api','API access',9900,30,1);").unwrap();
    db.execute_batch(DISCOVERY_SCHEMA).unwrap();
    db.execute_batch(RISK_SCHEMA).unwrap();
    db.execute_batch(TICK_SCHEMA).unwrap();
    db.execute_batch(CLUSTER_SCHEMA).unwrap();
    db.execute_batch(CUSTODY_SCHEMA).unwrap();
    db.execute_batch(FEE_SCHEMA).unwrap();
    db.execute_batch(SOCIAL_SCHEMA).unwrap();
    db.execute_batch(MARKET_SCHEMA).unwrap();
    db.execute_batch(SETTINGS_SCHEMA).unwrap();
    // Additive column for the trailing stop. Already-present is the expected
    // outcome on an existing database and is not an error.
    let _ = db.execute("ALTER TABLE positions ADD COLUMN peak REAL", []);
    let _ = db.execute(
        "ALTER TABLE managed_wallets ADD COLUMN exports INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = db.execute(
        "ALTER TABLE managed_wallets ADD COLUMN last_export INTEGER",
        [],
    );
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)",
            [email],
            |r| r.get(0),
        )
        .unwrap();
    if exists {
        let is_admin: bool = db
            .query_row("SELECT admin FROM users WHERE email=?1", [email], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            is_admin,
            "ADMIN_EMAIL belongs to a non-admin account; choose a new administrator email"
        );
    }
    if !exists {
        let pw = Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .unwrap()
            .to_string();
        db.execute(
            "INSERT INTO users(id,email,password,admin,expires,created) VALUES(?1,?2,?3,1,?4,?5)",
            params![
                Uuid::new_v4().to_string(),
                email,
                pw,
                now() + 315360000,
                now()
            ],
        )
        .unwrap();
    }
    db
}
#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
}
async fn register(State(app): State<App>, Json(c): Json<Credentials>) -> ApiResult {
    let email = c.email.trim().to_lowercase();
    if email.len() > 254 || !email.contains('@') || c.password.len() < 12 || c.password.len() > 128
    {
        return Err(bad("Use a valid email and a password of 12–128 characters"));
    }
    let pw = Argon2::default()
        .hash_password(c.password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map_err(|_| bad("Password could not be secured"))?
        .to_string();
    let db = app.lock().unwrap();
    let id = Uuid::new_v4().to_string();
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)",
        [&email],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(bad("An account with this email already exists"));
    }
    db.execute(
        "INSERT INTO users(id,email,password,created) VALUES(?1,?2,?3,?4)",
        params![id, email, pw, now()],
    )?;
    audit(&db, &id, "account.created", "User registration")?;
    let address = provision_wallet(&db, &id)?;
    Ok(Json(json!({"ok":true,"managed_wallet":address})))
}
async fn login(State(app): State<App>, Json(c): Json<Credentials>) -> Result<Response, ApiError> {
    if c.password.len() > 128 || c.email.len() > 254 {
        return Err(bad("Invalid credentials"));
    }
    let db = app.lock().unwrap();
    let email = c.email.trim().to_lowercase();
    let attempt = db
        .query_row(
            "SELECT attempts,reset FROM login_attempts WHERE email=?1",
            [&email],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    if attempt.is_some_and(|(a, t)| a >= 8 && t > now()) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts; retry in 15 minutes".into(),
        ));
    }
    db.execute("INSERT INTO login_attempts VALUES(?1,1,?2) ON CONFLICT(email) DO UPDATE SET attempts=CASE WHEN reset<?3 THEN 1 ELSE attempts+1 END,reset=CASE WHEN reset<?3 THEN ?2 ELSE reset END",params![email,now()+900,now()])?;
    let row = db
        .query_row(
            "SELECT id,password,banned FROM users WHERE email=?1",
            [&email],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((id, pw, banned)) = row else {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Invalid credentials".into(),
        ));
    };
    if banned
        || !PasswordHash::new(&pw).ok().is_some_and(|p| {
            Argon2::default()
                .verify_password(c.password.as_bytes(), &p)
                .is_ok()
        })
    {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Invalid credentials".into(),
        ));
    }
    db.execute("DELETE FROM login_attempts WHERE email=?1", [email])?;
    let token = secret();
    db.execute(
        "INSERT INTO sessions VALUES(?1,?2,?3,'session')",
        params![hash(&token), id, now() + 86400],
    )?;
    audit(&db, &id, "session.created", "Browser login")?;
    let secure = if std::env::var("COOKIE_SECURE").unwrap_or_default() == "true" {
        "; Secure"
    } else {
        ""
    };
    Ok((
        [(
            header::SET_COOKIE,
            format!("session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400{secure}"),
        )],
        Json(json!({"ok":true})),
    )
        .into_response())
}
async fn logout(State(app): State<App>, h: HeaderMap) -> Result<Response, ApiError> {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    db.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND kind='session'",
        [u.id],
    )?;
    Ok((
        [(
            header::SET_COOKIE,
            "session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        )],
        Json(json!({"ok":true})),
    )
        .into_response())
}
async fn me(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let (enabled, budget) = db.query_row(
        "SELECT enabled,budget FROM users WHERE id=?1",
        [&u.id],
        |r| Ok((r.get::<_, bool>(0)?, r.get::<_, f64>(1)?)),
    )?;
    Ok(Json(
        json!({"copy_trading":copy_trading_enabled(&db),"access_model":if commission_access() { "commission" } else { "subscription" },"id":u.id,"email":u.email,"admin":u.admin,"subscription_expires":u.expires,"subscribed":u.expires>now(),"enabled":enabled,"budget":budget}),
    ))
}
async fn overview(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let all = trades(&db)?;
    let wallets: std::collections::BTreeSet<_> = all.iter().map(|t| t.wallet.clone()).collect();
    let mut analyses: Vec<_> = wallets.iter().map(|w| analyze(w, &all)).collect();
    analyses.sort_by_key(|a| std::cmp::Reverse(a.score));
    let mut s = db.prepare("SELECT wallet FROM watches WHERE user_id=?1")?;
    let watches = s
        .query_map([&u.id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut p=db.prepare("SELECT id,wallet,symbol,cost,pnl,status,reason,created FROM positions WHERE user_id=?1 ORDER BY created DESC")?;
    let positions=p.query_map([&u.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"wallet":r.get::<_,String>(1)?,"symbol":r.get::<_,String>(2)?,"cost":r.get::<_,f64>(3)?,"pnl":r.get::<_,Option<f64>>(4)?,"status":r.get::<_,String>(5)?,"reason":r.get::<_,String>(6)?,"created":r.get::<_,i64>(7)?})))?.collect::<Result<Vec<_>,_>>()?;
    if !entitled(&u) {
        for a in &mut analyses {
            a.history.clear();
            a.curve.clear();
        }
    }
    let feed: Vec<_> = all.iter().rev().take(40).collect();
    Ok(Json(
        json!({"mode":"paper","source":"Imported history / optional RPC snapshots","wallets":analyses,"feed":feed,"watches":watches,"positions":positions,"last_event":all.last().map(|t|t.timestamp)}),
    ))
}
async fn wallet(State(app): State<App>, h: HeaderMap, Path(address): Path<String>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "An active subscription is required for wallet analysis".into(),
        ));
    }
    let all = trades(&db)?;
    if !all.iter().any(|t| t.wallet == address) {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No imported history for this wallet".into(),
        ));
    }
    Ok(Json(serde_json::to_value(analyze(&address, &all)).unwrap()))
}
#[derive(Deserialize)]
struct Watch {
    wallet: String,
}
async fn watch(State(app): State<App>, h: HeaderMap, Json(w): Json<Watch>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if w.wallet.is_empty() || w.wallet.len() > 128 {
        return Err(bad("Invalid wallet"));
    }
    db.execute(
        "INSERT OR IGNORE INTO watches VALUES(?1,?2)",
        params![u.id, w.wallet],
    )?;
    audit(&db, &u.id, "wallet.followed", &w.wallet)?;
    Ok(Json(json!({"ok":true})))
}
async fn unwatch(State(app): State<App>, h: HeaderMap, Path(w): Path<String>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    db.execute(
        "DELETE FROM watches WHERE user_id=?1 AND wallet=?2",
        params![u.id, w],
    )?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Settings {
    enabled: bool,
    budget: f64,
    mode: String,
}
async fn settings(State(app): State<App>, h: HeaderMap, Json(s): Json<Settings>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if s.enabled {
        require_copy_trading(&db)?;
    }
    if s.mode != "paper" {
        return Err(ApiError(StatusCode::CONFLICT,"Live execution requires a supported provider authorization; no live adapter is configured".into()));
    }
    if !s.budget.is_finite() || s.budget < 0.01 || s.budget > 10.0 {
        return Err(bad("Paper order size must be 0.01–10 SOL"));
    }
    if s.enabled && !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "An active subscription is required".into(),
        ));
    }
    db.execute(
        "UPDATE users SET enabled=?1,budget=?2 WHERE id=?3",
        params![s.enabled, s.budget, u.id],
    )?;
    audit(
        &db,
        &u.id,
        "execution.settings",
        &format!("paper enabled={}, size={}", s.enabled, s.budget),
    )?;
    Ok(Json(json!({"ok":true})))
}
async fn connectors() -> Json<Value> {
    Json(json!([
    {"id":"axiom","name":"Axiom","status":"Unavailable","description":"Official delegated trading authorization has not been verified. No passwords or session cookies collected.","url":"https://docs.axiom.trade/"},
    {"id":"fomo","name":"Fomo","status":"Unavailable","description":"No supported account-linking API established from the public documentation reviewed.","url":"https://fomo.family/"},
    {"id":"pump","name":"Pump.fun","status":"Protocol available","description":"Official program instructions are public. Live execution still needs RPC access and authorized transaction signing.","url":"https://github.com/pump-fun/pump-public-docs"}
    ]))
}
async fn plans(State(app): State<App>) -> ApiResult {
    let db = app.lock().unwrap();
    Ok(Json(json!(rows(
        &db,
        "SELECT * FROM plans WHERE active=1 ORDER BY price_cents"
    )?)))
}
async fn api_key(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "An active subscription is required".into(),
        ));
    }
    let key = format!("usk_{}", secret());
    db.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND kind='api'",
        [&u.id],
    )?;
    db.execute(
        "INSERT INTO sessions VALUES(?1,?2,?3,'api')",
        params![hash(&key), u.id, now() + 2592000],
    )?;
    audit(&db, &u.id, "api_key.rotated", "Previous API key revoked")?;
    Ok(Json(json!({"key":key,"expires":now()+2592000})))
}
async fn revoke_key(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    db.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND kind='api'",
        [&u.id],
    )?;
    Ok(Json(json!({"ok":true})))
}
async fn admin_overview(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    auth(&db, &h, true)?;
    Ok(Json(
        json!({"users":rows(&db,"SELECT id,email,admin,banned,expires,enabled,created FROM users ORDER BY created DESC")?,"logs":rows(&db,"SELECT * FROM logs ORDER BY id DESC LIMIT 100")?,"plans":rows(&db,"SELECT * FROM plans ORDER BY price_cents")?,"payments":rows(&db,"SELECT * FROM payments ORDER BY created DESC LIMIT 100")?,"revenue_cents":db.query_row("SELECT COALESCE(SUM(amount_cents),0) FROM payments",[],|r|r.get::<_,i64>(0))?}),
    ))
}
#[derive(Deserialize)]
struct Ban {
    banned: bool,
}
async fn ban(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<Ban>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let n = db.execute(
        "UPDATE users SET banned=?1,enabled=0 WHERE id=?2 AND admin=0",
        params![b.banned, id],
    )?;
    if n == 0 {
        return Err(bad("User not found or is an administrator"));
    }
    if b.banned {
        db.execute("DELETE FROM sessions WHERE user_id=?1", [&id])?;
    }
    audit(
        &db,
        &u.id,
        "user.ban_changed",
        &format!("{id}: {}", b.banned),
    )?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Plan {
    id: String,
    name: String,
    price_cents: i64,
    days: i64,
    active: bool,
}
async fn save_plan(State(app): State<App>, h: HeaderMap, Json(p): Json<Plan>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    if p.id.is_empty()
        || p.id.len() > 40
        || p.name.is_empty()
        || p.name.len() > 80
        || p.price_cents < 0
        || p.price_cents > 10000000
        || p.days < 1
        || p.days > 365
    {
        return Err(bad("Invalid plan values"));
    }
    db.execute("INSERT INTO plans VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET name=excluded.name,price_cents=excluded.price_cents,days=excluded.days,active=excluded.active",params![p.id,p.name,p.price_cents,p.days,p.active])?;
    audit(&db, &u.id, "plan.saved", &p.id)?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Grant {
    user_id: String,
    plan_id: String,
    reference: String,
    paid: bool,
}
async fn grant(State(app): State<App>, h: HeaderMap, Json(g): Json<Grant>) -> ApiResult {
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    if g.reference.trim().is_empty() || g.reference.len() > 128 {
        return Err(bad(
            "A unique payment or complimentary grant reference is required",
        ));
    }
    let tx = db.transaction()?;
    let (price, days) = tx
        .query_row(
            "SELECT price_cents,days FROM plans WHERE id=?1 AND active=1",
            [&g.plan_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
        .ok_or(bad("Active plan not found"))?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM payments WHERE reference=?1)",
        [&g.reference],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(bad("This reference has already been recorded"));
    }
    let n = tx.execute(
        "UPDATE users SET expires=MAX(expires,?1)+?2 WHERE id=?3 AND banned=0",
        params![now(), days * 86400, g.user_id],
    )?;
    if n == 0 {
        return Err(bad("Active user not found"));
    }
    tx.execute(
        "INSERT INTO payments VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            Uuid::new_v4().to_string(),
            g.user_id,
            g.plan_id,
            if g.paid { price } else { 0 },
            g.reference,
            now()
        ],
    )?;
    audit(&tx, &u.id, "subscription.granted", &g.user_id)?;
    tx.commit()?;
    Ok(Json(json!({"ok":true})))
}
// Imported events are historical. No automatic execution is triggered by imports.
async fn import(State(app): State<App>, h: HeaderMap, Json(batch): Json<Vec<Trade>>) -> ApiResult {
    if batch.is_empty()
        || batch.len() > 10000
        || batch
            .iter()
            .any(|t| !t.validate() || t.timestamp > now() + 60)
    {
        return Err(bad(
            "Provide 1–10,000 valid trade events with nonfuture timestamps",
        ));
    }
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let tx = db.transaction()?;
    let mut count = 0;
    for t in batch {
        count += tx.execute(
            "INSERT OR IGNORE INTO trades VALUES(?1,?2,?3)",
            params![t.id, t.timestamp, serde_json::to_string(&t).unwrap()],
        )?;
    }
    audit(&tx, &u.id, "history.imported", &format!("{count} events"))?;
    tx.commit()?;
    Ok(Json(json!({"inserted":count})))
}
async fn close_position(State(app): State<App>, h: HeaderMap, Path(id): Path<String>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let (token,q,cost)=db.query_row("SELECT token,quantity,cost FROM positions WHERE id=?1 AND user_id=?2 AND status='open'",params![id,u.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?,r.get::<_,f64>(2)?))).optional()?.ok_or(bad("Open paper position not found"))?;
    let all = trades(&db)?;
    let price = all
        .iter()
        .rev()
        .find(|t| t.token == token)
        .ok_or(bad("No reference price"))?
        .price_sol;
    let model = fees::CostModel::from_env();
    let gross = q * price;
    let proceeds = gross - model.charge(gross);
    fees::charge_trade(
        &db,
        &u.id,
        &format!("{id}:exit"),
        gross,
        now(),
        &fees::Schedule::from_env(),
    )?;
    db.execute("UPDATE positions SET status='closed',exit=?1,pnl=?2,reason='Manual paper exit; fixed-plus-proportional cost model',closed=?3 WHERE id=?4",params![price,proceeds-cost,now(),id])?;
    audit(&db, &u.id, "paper.closed", &id)?;
    Ok(Json(json!({"ok":true})))
}
/// Evaluates one newly stored event against every eligible follower.
///
/// Shared by the administrator ingestion route and the live collector so both
/// paths apply exactly the same eligibility, gates and refusal accounting.
/// Returns the number of followers considered, positions opened and closed, and
/// the named refusals.
fn fanout(
    db: &Connection,
    event: &Trade,
) -> Result<(usize, usize, usize, BTreeMap<String, usize>), ApiError> {
    // The switch is checked here too, not only at the routes: the live
    // collector reaches this function without passing through one.
    if !copy_trading_enabled(db) {
        return Ok((0, 0, 0, BTreeMap::new()));
    }
    let subscription_floor = if commission_access() { 0 } else { now() };
    let users = {
        let mut stmt=db.prepare("SELECT u.id,u.budget FROM users u JOIN watches w ON w.user_id=u.id WHERE u.enabled=1 AND u.banned=0 AND u.expires>?1 AND w.wallet=?2")?;
        stmt.query_map(params![subscription_floor, event.wallet], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    if users.is_empty() {
        return Ok((0, 0, 0, BTreeMap::new()));
    }
    let all = trades(db)?;
    let (mut opened, mut closed) = (0, 0);
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    for (id, budget) in &users {
        let (o, c, reasons) = process_paper(db, id, *budget, &all, Some(&event.id))?;
        opened += o;
        closed += c;
        for (reason, count) in reasons {
            *refusals.entry(reason).or_default() += count;
        }
        audit(
            db,
            id,
            "paper.event_evaluated",
            &format!("{}: {o} opened, {c} closed", event.id),
        )?;
    }
    Ok((users.len(), opened, closed, refusals))
}

fn process_paper(
    db: &Connection,
    user_id: &str,
    budget: f64,
    all: &[Trade],
    only_event: Option<&str>,
) -> Result<(usize, usize, BTreeMap<String, usize>), ApiError> {
    let watermark:i64=db.query_row("SELECT COALESCE(MAX(t.timestamp),0) FROM processed p JOIN trades t ON t.id=p.event_id WHERE p.user_id=?1",[&user_id],|r|r.get(0))?;
    let mut opened = 0;
    let mut closed = 0;
    // Named refusals: a user is told why a followed wallet's buy was not copied.
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    let risk = risk_token::Config::from_env();
    let authenticity = cluster::Config::from_env();
    let costs = fees::CostModel::from_env();
    let schedule = fees::Schedule::from_env();
    for (i, t) in all.iter().enumerate() {
        if only_event.is_some_and(|id| id != t.id) {
            continue;
        }
        if !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM watches WHERE user_id=?1 AND wallet=?2)",
            params![user_id, t.wallet],
            |r| r.get::<_, bool>(0),
        )? {
            continue;
        }
        if db.execute(
            "INSERT OR IGNORE INTO processed VALUES(?1,?2)",
            params![user_id, t.id],
        )? == 0
        {
            continue;
        }
        if t.timestamp <= watermark {
            continue;
        }
        let open=db.query_row("SELECT id,quantity,cost,entry FROM positions WHERE user_id=?1 AND wallet=?2 AND token=?3 AND status='open'",params![user_id,t.wallet,t.token],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?,r.get::<_,f64>(2)?,r.get::<_,f64>(3)?))).optional()?;
        if let Some((id, q, cost, entry)) = open {
            let reason = if t.side == "sell" {
                Some("Leader sell detected")
            } else if t.price_sol < entry * 0.8 {
                Some("20% stop threshold observed")
            } else if t.liquidity_sol < min_liquidity_sol() {
                Some("Liquidity below threshold")
            } else {
                None
            };
            if let Some(reason) = reason {
                let gross = q * t.price_sol;
                let proceeds = gross - costs.charge(gross);
                fees::charge_trade(
                    db,
                    user_id,
                    &format!("{id}:exit"),
                    gross,
                    t.timestamp,
                    &schedule,
                )?;
                db.execute("UPDATE positions SET status='closed',exit=?1,pnl=?2,reason=?3,closed=?4 WHERE id=?5",params![t.price_sol,proceeds-cost,reason,t.timestamp,id])?;
                closed += 1;
            }
            continue;
        }
        if t.side != "buy" {
            continue;
        }
        let mut refuse = |reason: &str| {
            *refusals.entry(reason.to_string()).or_default() += 1;
        };
        let floor = min_liquidity_sol();
        if t.liquidity_sol < floor {
            refuse(&format!(
                "Supplied liquidity is unknown or below {floor} SOL"
            ));
            continue;
        }
        if budget > t.liquidity_sol * 0.001 {
            refuse("Order size exceeds 0.1% of supplied liquidity");
            continue;
        }
        if let Some(reason) = risk_token::check(db, &t.token, &risk, t.timestamp) {
            refuse(&reason);
            continue;
        }
        let a = analyze(&t.wallet, &all[..i]);
        if a.matched_sells < 20 {
            refuse("Source wallet has fewer than 20 prior matched sells");
            continue;
        }
        if a.unmatched_quantity > 1e-9 {
            refuse("Source wallet has incomplete cost basis in prior history");
            continue;
        }
        if a.score < 65 {
            refuse("Source wallet ranking heuristic is below 65");
            continue;
        }
        if let Some(reason) = cluster::refuse(&t.wallet, &all[..i], &authenticity) {
            refuse(&reason);
            continue;
        }
        if authenticity.enforce_deployer_funding
            && let Some(reason) =
                cluster::deployer_funded(db, &t.wallet, &t.token, t.timestamp, &authenticity)?
        {
            refuse(&reason);
            continue;
        }
        let exposure: f64 = db.query_row(
            "SELECT COALESCE(SUM(cost),0) FROM positions WHERE user_id=?1 AND status='open'",
            [&user_id],
            |r| r.get(0),
        )?;
        if exposure + budget > budget * 5.0 {
            refuse("Open exposure cap reached");
            continue;
        }
        let loss:f64=db.query_row("SELECT COALESCE(SUM(pnl),0) FROM positions WHERE user_id=?1 AND status='closed' AND closed>=?2",params![user_id,t.timestamp-t.timestamp%86400],|r|r.get(0))?;
        if loss < -budget * 2.0 {
            refuse("Daily realized loss guard is active");
            continue;
        }
        // The fixed component is consumed whatever the size, so it does not buy
        // tokens. Charging it proportionally would understate small orders.
        let spendable = budget - costs.charge(budget);
        if spendable <= 0.0 {
            refuse("Order is smaller than the cost of placing it");
            continue;
        }
        let q = spendable / t.price_sol;
        let position = Uuid::new_v4().to_string();
        db.execute("INSERT INTO positions(id,user_id,wallet,token,symbol,quantity,cost,entry,status,reason,created) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'open','Paper simulation; fixed-plus-proportional costs each side',?9)",params![position,user_id,t.wallet,t.token,t.symbol,q,budget,t.price_sol,t.timestamp])?;
        fees::charge_trade(
            db,
            user_id,
            &format!("{position}:entry"),
            budget,
            t.timestamp,
            &schedule,
        )?;
        opened += 1;
    }
    Ok((opened, closed, refusals))
}

// Explicit replay uses only history before each event. It is an illustrative
// simulation: fixed cost assumptions do not model actual liquidity or latency.
async fn replay(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "Subscription required".into(),
        ));
    }
    let (enabled, budget) = db.query_row(
        "SELECT enabled,budget FROM users WHERE id=?1",
        [&u.id],
        |r| Ok((r.get::<_, bool>(0)?, r.get::<_, f64>(1)?)),
    )?;
    require_copy_trading(&db)?;
    if !enabled {
        return Err(bad("Enable paper execution first"));
    }
    let all = trades(&db)?;
    let tx = db.transaction()?;
    let (opened, closed, refusals) = process_paper(&tx, &u.id, budget, &all, None)?;
    audit(
        &tx,
        &u.id,
        "paper.replayed",
        &format!("{opened} opened, {closed} closed"),
    )?;
    tx.commit()?;
    Ok(Json(
        json!({"opened":opened,"closed":closed,"refusals":refusals,"note":"Historical replay only; fixed 2% costs per side, no latency model"}),
    ))
}

// Trusted normalized event boundary. This operates on paper balances only.
async fn ingest_event(State(app): State<App>, h: HeaderMap, Json(event): Json<Trade>) -> ApiResult {
    let mut db = app.lock().unwrap();
    let admin = auth(&db, &h, true)?;
    if !event.validate() || event.timestamp < now() - 120 || event.timestamp > now() + 5 {
        return Err(bad(
            "Event must be valid and no more than 120 seconds old or 5 seconds in the future",
        ));
    }
    let tx = db.transaction()?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM trades WHERE id=?1)",
        [&event.id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(Json(
            json!({"duplicate":true,"users_evaluated":0,"opened":0,"closed":0}),
        ));
    }
    tx.execute(
        "INSERT INTO trades VALUES(?1,?2,?3)",
        params![
            event.id,
            event.timestamp,
            serde_json::to_string(&event).unwrap()
        ],
    )?;
    let (evaluated, opened, closed, refusals) = fanout(&tx, &event)?;
    audit(&tx, &admin.id, "event.ingested", &event.id)?;
    tx.commit()?;
    Ok(Json(
        json!({"duplicate":false,"users_evaluated":evaluated,"opened":opened,"closed":closed,"refusals":refusals,"execution":"paper"}),
    ))
}

#[derive(Deserialize)]
struct SyncRequest {
    wallet: String,
    before: Option<String>,
}
async fn sync_rpc(State(app): State<App>, h: HeaderMap, Json(req): Json<SyncRequest>) -> ApiResult {
    if !rpc::valid_address(&req.wallet) || req.before.as_ref().is_some_and(|v| v.len() > 100) {
        return Err(bad(
            "Provide a valid Solana wallet and optional pagination signature",
        ));
    }
    {
        let db = app.lock().unwrap();
        let u = auth(&db, &h, true)?;
        let last = db
            .query_row(
                "SELECT last_attempt FROM rpc_syncs WHERE wallet=?1",
                [&req.wallet],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        if now() - last < 30 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Wait 30 seconds between requests for this wallet".into(),
            ));
        }
        db.execute("INSERT INTO rpc_syncs VALUES(?1,?2) ON CONFLICT(wallet) DO UPDATE SET last_attempt=excluded.last_attempt",params![req.wallet,now()])?;
        audit(&db, &u.id, "rpc.sync_started", &req.wallet)?;
    }
    let url =
        std::env::var("SOLANA_RPC_URL").unwrap_or("https://api.mainnet-beta.solana.com".into());
    let page = rpc::fetch(&url, &req.wallet, req.before.as_deref())
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let tx = db.transaction()?;
    let mut imported = 0;
    let mut decoded = 0;
    for snap in &page.snapshots {
        tx.execute(
            "INSERT OR IGNORE INTO raw_transactions VALUES(?1,?2,?3,?4)",
            params![snap.signature, req.wallet, snap.data.to_string(), now()],
        )?;
        if let Some(t) = &snap.trade {
            decoded += 1;
            imported += tx.execute(
                "INSERT OR IGNORE INTO trades VALUES(?1,?2,?3)",
                params![t.id, t.timestamp, serde_json::to_string(t).unwrap()],
            )?;
        }
    }
    audit(
        &tx,
        &u.id,
        "rpc.history_collected",
        &format!(
            "{}: {} scanned, {} decoded, {} inserted",
            req.wallet, page.scanned, decoded, imported
        ),
    )?;
    tx.commit()?;
    Ok(Json(
        json!({"scanned":page.scanned,"archived":page.snapshots.len(),"decoded":decoded,"inserted":imported,"unavailable":page.failed,"next_before":page.next_before,"coverage":"Partial: single direct native-SOL Pump/PumpSwap swaps only. Liquidity unknown; paper entries blocked. No global stream or execution."}),
    ))
}

// Discovery surfaces wallets that recently traded size. Size is not skill: nothing
// here grants eligibility, and a candidate reaches the paper policy only after
// history collection and the normal analytics gates.
async fn discovery_overview(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    auth(&db, &h, true)?;
    let candidates = rows(
        &db,
        "SELECT c.wallet,c.first_seen,c.last_seen,c.buys,c.sells,c.sol_volume,c.max_trade_sol,c.promoted,c.source,(SELECT COUNT(*) FROM candidate_tokens t WHERE t.wallet=c.wallet) AS distinct_tokens FROM candidate_wallets c ORDER BY c.sol_volume DESC LIMIT 100",
    )?;
    let observations: i64 =
        db.query_row("SELECT COUNT(*) FROM candidate_events", [], |r| r.get(0))?;
    // An empty candidate list can mean a quiet market or a refused upstream
    // subscription. Report which.
    let status = db
        .query_row(
            "SELECT state,detail,updated FROM collector_status WHERE id=1",
            [],
            |r| {
                Ok(
                    json!({"state":r.get::<_,String>(0)?,"detail":r.get::<_,String>(1)?,"updated":r.get::<_,i64>(2)?}),
                )
            },
        )
        .optional()?
        .unwrap_or(json!({"state":"never_started","detail":"The collector has not reported in.","updated":0}));
    let watch = db
        .query_row(
            "SELECT state,detail,updated FROM watch_status WHERE id=1",
            [],
            |r| {
                Ok(
                    json!({"state":r.get::<_,String>(0)?,"detail":r.get::<_,String>(1)?,"updated":r.get::<_,i64>(2)?}),
                )
            },
        )
        .optional()?
        .unwrap_or(json!({"state":"never_started","detail":"The live copy feed has not reported in.","updated":0}));
    Ok(Json(
        json!({"running":discovery::Config::from_env().is_some(),"status":status,"watch":watch,"watch_running":watch::Config::from_env().is_some(),"observations":observations,"candidates":candidates,"note":"Recent large-size observations from a public firehose. Reserves reported upstream are virtual and are not treated as executable liquidity. Ranking by volume is not a performance claim."}),
    ))
}
#[derive(Deserialize)]
struct Promote {
    wallet: String,
}
async fn promote(State(app): State<App>, h: HeaderMap, Json(p): Json<Promote>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    if !rpc::valid_address(&p.wallet) {
        return Err(bad("Provide a valid Solana wallet"));
    }
    if db.execute(
        "UPDATE candidate_wallets SET promoted=1 WHERE wallet=?1",
        [&p.wallet],
    )? == 0
    {
        return Err(bad("Candidate not found"));
    }
    audit(&db, &u.id, "discovery.promoted", &p.wallet)?;
    Ok(Json(
        json!({"ok":true,"next":"Collect history with /api/admin/rpc/sync before this wallet can be analyzed"}),
    ))
}

#[derive(Deserialize)]
struct MintRequest {
    mint: String,
}
// Collection is administrator-only and server-configured: the provider URL is never
// a request field, so this cannot be pointed at an arbitrary host.
async fn collect_token_risk(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<MintRequest>,
) -> ApiResult {
    if !rpc::valid_address(&req.mint) {
        return Err(bad("Provide a valid token mint address"));
    }
    {
        let db = app.lock().unwrap();
        auth(&db, &h, true)?;
    }
    let cfg = risk_token::Config::from_env();
    let raw = risk_token::fetch(&cfg.base_url, &req.mint)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let assessment = risk_token::assess(&req.mint, &raw)
        .ok_or_else(|| ApiError(StatusCode::BAD_GATEWAY, "Unreadable risk report".into()))?;
    let blocked = risk_token::gate(&assessment, &cfg);
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    risk_token::store(&db, &assessment, blocked.as_ref(), &raw, now())?;
    audit(
        &db,
        &u.id,
        "token_risk.collected",
        &format!("{}: {}", req.mint, blocked.as_deref().unwrap_or("cleared")),
    )?;
    Ok(Json(risk_token::summary(&assessment, blocked.as_ref())))
}
async fn token_risk(State(app): State<App>, h: HeaderMap, Path(mint): Path<String>) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "An active subscription is required for token risk detail".into(),
        ));
    }
    let (data, blocked, checked) = db
        .query_row(
            "SELECT data,blocked_reason,checked FROM token_risk WHERE mint=?1",
            [&mint],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(ApiError(
            StatusCode::NOT_FOUND,
            "No risk assessment has been collected for this mint".into(),
        ))?;
    let raw: Value = serde_json::from_str(&data).map_err(|_| bad("Stored report is unreadable"))?;
    let assessment = risk_token::assess(&mint, &raw)
        .ok_or_else(|| bad("Stored report no longer parses; collect it again"))?;
    let mut out = risk_token::summary(&assessment, blocked.as_ref());
    out["checked"] = json!(checked);
    out["stale"] = json!(now() - checked > risk_token::Config::from_env().max_age);
    Ok(Json(out))
}

// Copyability answers the question a leaderboard cannot: what would this user,
// at this order size, at this delay, actually have realized. The leader's own
// profit is reported beside it so the two are never confused.
async fn wallet_copyability(
    State(app): State<App>,
    h: HeaderMap,
    Path(address): Path<String>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "An active subscription is required for copyability analysis".into(),
        ));
    }
    let all = trades(&db)?;
    if !all.iter().any(|t| t.wallet == address) {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No imported history for this wallet".into(),
        ));
    }
    let budget: f64 = db.query_row("SELECT budget FROM users WHERE id=?1", [&u.id], |r| {
        r.get(0)
    })?;
    let number = |key: &str, fallback: f64| {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f64| v.is_finite() && *v >= 0.0)
            .unwrap_or(fallback)
    };
    let report = copyability(
        &address,
        &all,
        &analytics::DEFAULT_DELAYS,
        budget,
        &fees::CostModel::from_env(),
        number("COPY_MAX_WAIT_SECONDS", 3600.0) as i64,
    );
    Ok(Json(serde_json::to_value(report).unwrap()))
}

#[derive(Deserialize)]
struct Tick {
    token: String,
    price_sol: f64,
    timestamp: i64,
}
// Independent price observations. Unlike `/admin/events` these never open a
// position: a tick can only manage or close exposure that already exists.
async fn ingest_ticks(
    State(app): State<App>,
    h: HeaderMap,
    Json(batch): Json<Vec<Tick>>,
) -> ApiResult {
    if batch.is_empty() || batch.len() > 1000 {
        return Err(bad("Provide 1–1,000 price observations"));
    }
    if batch.iter().any(|t| {
        t.token.is_empty()
            || t.token.len() > 128
            || !t.price_sol.is_finite()
            || t.price_sol <= 0.0
            || t.price_sol > 1e12
            || t.timestamp <= 0
            || t.timestamp > now() + 5
    }) {
        return Err(bad(
            "Each observation needs a token, a positive finite price and a nonfuture timestamp",
        ));
    }
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let policy = ticks::Policy::from_env();
    let tx = db.transaction()?;
    let mut closed = vec![];
    for t in &batch {
        ticks::record(&tx, &t.token, t.price_sol, t.timestamp, "admin")?;
        // An administrator tick carries no curve state, so migration proximity
        // is unknown here rather than assumed distant.
        closed.extend(ticks::apply(
            &tx,
            &t.token,
            t.price_sol,
            t.timestamp,
            None,
            &policy,
        )?);
    }
    for c in &closed {
        audit(
            &tx,
            &c.user_id,
            "paper.tick_exit",
            &format!("{}: {}", c.id, c.reason),
        )?;
    }
    audit(
        &tx,
        &u.id,
        "ticks.ingested",
        &format!(
            "{} observations, {} positions closed",
            batch.len(),
            closed.len()
        ),
    )?;
    tx.commit()?;
    Ok(Json(
        json!({"recorded":batch.len(),"closed":closed,"note":"Observed trade prices, not executable quotes. No price impact for our own size is modeled."}),
    ))
}

// Structural checks on the wallet itself: who funded it, who it shares funding
// with, and whether it is systematically present before anyone could follow it.
async fn wallet_authenticity(
    State(app): State<App>,
    h: HeaderMap,
    Path(address): Path<String>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if !entitled(&u) {
        return Err(ApiError(
            StatusCode::PAYMENT_REQUIRED,
            "An active subscription is required for authenticity analysis".into(),
        ));
    }
    let all = trades(&db)?;
    if !all.iter().any(|t| t.wallet == address) {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No imported history for this wallet".into(),
        ));
    }
    let cfg = cluster::Config::from_env();
    let report = cluster::assess(&db, &address, &all, &cfg)?;
    let mut out = serde_json::to_value(report).unwrap();
    out["blocked"] = json!(cluster::refuse(&address, &all, &cfg));
    // Deployer funding is reported for every buy regardless of whether it is
    // being enforced, so an operator can see what enabling it would cost.
    let deployer_hits: Vec<Value> = all
        .iter()
        .filter(|t| t.wallet == address && t.side == "buy")
        .filter_map(|t| {
            cluster::deployer_funded(&db, &t.wallet, &t.token, t.timestamp, &cfg)
                .ok()
                .flatten()
                .map(|reason| json!({"token": t.token, "reason": reason}))
        })
        .take(20)
        .collect();
    out["deployer_funded"] = json!(deployer_hits);
    out["deployer_funding_enforced"] = json!(cfg.enforce_deployer_funding);
    Ok(Json(out))
}
// Builds funding edges from transactions already archived by RPC collection.
// It performs no network access of its own.
async fn index_funding(State(app): State<App>, h: HeaderMap, Json(w): Json<Watch>) -> ApiResult {
    if !rpc::valid_address(&w.wallet) {
        return Err(bad("Provide a valid Solana wallet"));
    }
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let written = cluster::index(&db, &w.wallet)?;
    audit(
        &db,
        &u.id,
        "cluster.indexed",
        &format!("{}: {written} edges", w.wallet),
    )?;
    Ok(Json(
        json!({"edges":written,"note":"Derived from archived raw transactions only. Collect more history with /api/admin/rpc/sync to widen coverage."}),
    ))
}

fn rpc_url() -> String {
    std::env::var("SOLANA_RPC_URL").unwrap_or("https://api.mainnet-beta.solana.com".into())
}
fn origin_domain(h: &HeaderMap) -> String {
    h.get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 253 && !v.is_empty())
        .unwrap_or("unskilled.local")
        .to_string()
}
/// Issues a managed wallet when custody is configured. A provisioning failure
/// must not silently produce an account with no wallet, so it is surfaced.
fn provision_wallet(db: &Connection, user_id: &str) -> Result<Option<String>, ApiError> {
    let Some(custody) = custody::Custody::from_env() else {
        return Ok(None);
    };
    custody::provision(db, &custody, user_id, now())
        .map(Some)
        .map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Managed wallet could not be created".into(),
            )
        })
}
fn issue_session(db: &Connection, user_id: &str) -> Result<String, ApiError> {
    let token = secret();
    db.execute(
        "INSERT INTO sessions VALUES(?1,?2,?3,'session')",
        params![hash(&token), user_id, now() + 86400],
    )?;
    Ok(token)
}
fn session_cookie(token: &str) -> String {
    let secure = if std::env::var("COOKIE_SECURE").unwrap_or_default() == "true" {
        "; Secure"
    } else {
        ""
    };
    format!("session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400{secure}")
}

#[derive(Deserialize)]
struct ChallengeRequest {
    address: String,
    chain: String,
}
// Step one of wallet sign-in. The nonce is server-issued and single-use; the
// message it goes into states that signing authorizes no transaction.
async fn wallet_challenge(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<ChallengeRequest>,
) -> ApiResult {
    let chain = wallet_auth::Chain::parse(&req.chain).ok_or(bad("Unsupported chain"))?;
    let address =
        wallet_auth::normalize(chain, &req.address).ok_or(bad("Invalid wallet address"))?;
    let nonce = secret();
    let issued = now();
    let db = app.lock().unwrap();
    db.execute("DELETE FROM auth_challenges WHERE expires<?1", [issued])?;
    db.execute(
        "INSERT INTO auth_challenges VALUES(?1,?2,?3,?4)",
        params![
            nonce,
            address,
            chain.name(),
            issued + wallet_auth::CHALLENGE_TTL_SECONDS
        ],
    )?;
    Ok(Json(json!({
        "nonce": nonce,
        "message": wallet_auth::message(&origin_domain(&h), chain, &address, &nonce, issued),
        "expires_in": wallet_auth::CHALLENGE_TTL_SECONDS
    })))
}

#[derive(Deserialize)]
struct VerifyRequest {
    address: String,
    chain: String,
    nonce: String,
    signature: String,
}
// Step two. The challenge row is deleted as part of verification, so one
// signature can be redeemed exactly once even under concurrent submission.
async fn wallet_verify(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<VerifyRequest>,
) -> Result<Response, ApiError> {
    let chain = wallet_auth::Chain::parse(&req.chain).ok_or(bad("Unsupported chain"))?;
    let address =
        wallet_auth::normalize(chain, &req.address).ok_or(bad("Invalid wallet address"))?;
    if req.nonce.len() > 128 || req.signature.len() > 256 {
        return Err(bad("Malformed challenge response"));
    }
    let mut db = app.lock().unwrap();
    let tx = db.transaction()?;
    let issued: i64 = tx
        .query_row(
            "SELECT expires FROM auth_challenges WHERE nonce=?1 AND address=?2 AND chain=?3",
            params![req.nonce, address, chain.name()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(ApiError(
            StatusCode::UNAUTHORIZED,
            "Unknown or already used challenge".into(),
        ))?;
    // Consume first: an invalid signature must still burn the nonce.
    tx.execute("DELETE FROM auth_challenges WHERE nonce=?1", [&req.nonce])?;
    if issued < now() {
        tx.commit()?;
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Challenge expired; request a new one".into(),
        ));
    }
    let expected = wallet_auth::message(
        &origin_domain(&h),
        chain,
        &address,
        &req.nonce,
        issued - wallet_auth::CHALLENGE_TTL_SECONDS,
    );
    if !wallet_auth::verify(chain, &address, &expected, &req.signature) {
        tx.commit()?;
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Signature did not match this address".into(),
        ));
    }
    let existing: Option<String> = tx
        .query_row(
            "SELECT user_id FROM wallet_identities WHERE address=?1 AND chain=?2",
            params![address, chain.name()],
            |r| r.get(0),
        )
        .optional()?;
    let user_id = match existing {
        Some(id) => {
            if tx.query_row("SELECT banned FROM users WHERE id=?1", [&id], |r| {
                r.get::<_, bool>(0)
            })? {
                return Err(ApiError(
                    StatusCode::UNAUTHORIZED,
                    "Invalid credentials".into(),
                ));
            }
            id
        }
        None => {
            let id = Uuid::new_v4().to_string();
            // Wallet accounts have no password. The stored hash is empty, which
            // no password can ever verify against, so the email form of this
            // identity cannot be used to sign in.
            tx.execute(
                "INSERT INTO users(id,email,password,created) VALUES(?1,?2,'',?3)",
                params![id, format!("{}:{address}", chain.name()), now()],
            )?;
            tx.execute(
                "INSERT INTO wallet_identities VALUES(?1,?2,?3,?4)",
                params![address, chain.name(), id, now()],
            )?;
            audit(&tx, &id, "account.created", "Wallet sign-in")?;
            id
        }
    };
    provision_wallet(&tx, &user_id)?;
    let token = issue_session(&tx, &user_id)?;
    audit(&tx, &user_id, "session.created", "Wallet sign-in")?;
    tx.commit()?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&token))],
        Json(json!({"ok":true,"address":address,"chain":chain.name()})),
    )
        .into_response())
}

// The funds widget. Balance is fetched live; a provider failure is reported as
// unavailable rather than rendered as a zero balance.
async fn funds(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let (user, address, charged, realized) = {
        let db = app.lock().unwrap();
        let u = auth(&db, &h, false)?;
        let address = custody::address(&db, &u.id)?;
        let charged = fees::total_charged(&db, &u.id)?;
        let realized: f64 = db.query_row(
            "SELECT COALESCE(SUM(pnl),0) FROM positions WHERE user_id=?1 AND status='closed'",
            [&u.id],
            |r| r.get(0),
        )?;
        (u, address, charged, realized)
    };
    let schedule = fees::Schedule::from_env();
    let (mut balance, mut balance_error) = (None, None);
    if let Some(address) = &address {
        match custody::balance(&rpc_url(), address).await {
            Ok(lamports) => balance = Some(lamports),
            Err(e) => balance_error = Some(e),
        }
    }
    let owed = charged.max(0) as u64;
    Ok(Json(json!({
        "address": address,
        "custody_enabled": custody::Custody::from_env().is_some(),
        "deposit_address": address,
        "deposit_note": "Send SOL to this address from any wallet or exchange. It credits automatically once the network confirms it; there is nothing else to click.",
        "withdrawals_enabled": transfer::enabled(),
        "commission_owed_lamports": owed,
        "withdrawable_lamports": balance.map(|b| transfer::withdrawable(b, owed)),
        "reserved_lamports": transfer::RENT_EXEMPT_LAMPORTS + transfer::FEE_RESERVE_LAMPORTS,
        "balance_lamports": balance,
        "balance_sol": balance.map(|l| l as f64 / fees::LAMPORTS_PER_SOL),
        "balance_error": balance_error,
        "realized_pnl_sol": realized,
        "commission_charged_lamports": charged,
        "commission_charged_sol": charged as f64 / fees::LAMPORTS_PER_SOL,
        "fee_model": schedule.model,
        "conflict_note": schedule.model.conflict_note(),
        "subscribed": user.expires > now(),
        "access_model": if commission_access() { "commission" } else { "subscription" },
        "note": "Paper execution only. No order has been signed or broadcast, and commission figures are accrued against simulated results."
    })))
}

async fn fee_history(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let schedule = fees::Schedule::from_env();
    let ledger = {
        let mut stmt = db.prepare("SELECT kind,lamports,basis_lamports,reference,created FROM fee_ledger WHERE user_id=?1 ORDER BY created DESC LIMIT 100")?;
        stmt.query_map([&u.id], |r| {
            Ok(json!({"kind":r.get::<_,String>(0)?,"lamports":r.get::<_,i64>(1)?,"basis_lamports":r.get::<_,i64>(2)?,"reference":r.get::<_,String>(3)?,"created":r.get::<_,i64>(4)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let (high_water, last_settled) = db
        .query_row(
            "SELECT high_water_lamports,last_settled FROM fee_state WHERE user_id=?1",
            [&u.id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
        .unwrap_or((0, 0));
    Ok(Json(json!({
        "schedule": schedule,
        "conflict_note": schedule.model.conflict_note(),
        "cost_model": fees::CostModel::from_env(),
        "high_water_lamports": high_water,
        "last_settled": last_settled,
        "total_charged_lamports": fees::total_charged(&db, &u.id)?,
        "ledger": ledger
    })))
}

async fn settle_fees(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let mut db = app.lock().unwrap();
    let admin = auth(&db, &h, true)?;
    let schedule = fees::Schedule::from_env();
    let at = now();
    let tx = db.transaction()?;
    let users = {
        let mut stmt = tx.prepare("SELECT id FROM users WHERE banned=0")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut charged = 0u64;
    let mut settled = vec![];
    for id in &users {
        let s = fees::settle(&tx, id, at, &schedule)?;
        if s.charged_lamports > 0 {
            charged += s.charged_lamports;
            audit(
                &tx,
                id,
                "fee.settled",
                &format!("{} lamports: {}", s.charged_lamports, s.reason),
            )?;
            settled.push(json!({"user_id":id,"settlement":s}));
        }
    }
    audit(
        &tx,
        &admin.id,
        "fee.settlement_run",
        &format!("{} accounts charged {charged} lamports", settled.len()),
    )?;
    tx.commit()?;
    Ok(Json(
        json!({"accounts_considered":users.len(),"accounts_charged":settled.len(),"charged_lamports":charged,"settlements":settled}),
    ))
}

#[derive(Deserialize)]
struct CloseAccount {
    confirm: String,
}
// Ends custody. The key is returned exactly once, in one response, and is never
// written to the audit log. Afterwards the service cannot sign for that wallet.
async fn close_account(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<CloseAccount>,
) -> ApiResult {
    if req.confirm != "CLOSE" {
        return Err(bad(
            "Send confirm:\"CLOSE\" to close the account and release the wallet key",
        ));
    }
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let Some(custody) = custody::Custody::from_env() else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Custody is not configured on this server".into(),
        ));
    };
    let open: i64 = db.query_row(
        "SELECT COUNT(*) FROM positions WHERE user_id=?1 AND status='open'",
        [&u.id],
        |r| r.get(0),
    )?;
    if open > 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!("Close your {open} open positions before closing the account"),
        ));
    }
    let at = now();
    let tx = db.transaction()?;
    let released = custody::release(&tx, &custody, &u.id, at)?;
    tx.execute("UPDATE users SET enabled=0 WHERE id=?1", [&u.id])?;
    tx.execute("DELETE FROM sessions WHERE user_id=?1", [&u.id])?;
    audit(
        &tx,
        &u.id,
        "account.closed",
        "Managed wallet key released to owner",
    )?;
    tx.commit()?;
    let Some((address, secret)) = released else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "No managed wallet key is available to release".into(),
        ));
    };
    Ok(Json(json!({
        "address": address,
        "secret_key": secret,
        "format": "base58-encoded 64-byte keypair, importable into Phantom or the Solana CLI",
        "warning": "This is shown once and is not recoverable. Anyone holding it controls the funds. Your sessions have been revoked and this service can no longer sign for this wallet."
    })))
}

/// Commission owed but not yet collected, in lamports. Charges are positive rows
/// and collections are negative ones, so the sum is the outstanding balance.
fn outstanding_fees(db: &Connection, user_id: &str) -> Result<u64, ApiError> {
    Ok(fees::total_charged(db, user_id)?.max(0) as u64)
}
/// Settles commission immediately, ignoring the epoch clock.
///
/// This is why custody exists: a user withdrawing mid-strategy would otherwise
/// leave without paying for the profit already realized.
fn settle_before_withdrawal(db: &Connection, user_id: &str, at: i64) -> Result<(), ApiError> {
    let mut schedule = fees::Schedule::from_env();
    schedule.epoch_seconds = 1;
    schedule.min_settlement_lamports = 0;
    fees::settle(db, user_id, at, &schedule)?;
    Ok(())
}

#[derive(Deserialize)]
struct Withdraw {
    destination: String,
    lamports: u64,
}
// Moves real funds. Narrow by construction: one System transfer to one address.
async fn withdraw(State(app): State<App>, h: HeaderMap, Json(req): Json<Withdraw>) -> ApiResult {
    if !transfer::enabled() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Withdrawals are not enabled on this server".into(),
        ));
    }
    if !rpc::valid_address(&req.destination) {
        return Err(bad("Provide a valid Solana destination address"));
    }
    if req.lamports == 0 {
        return Err(bad("Amount must be greater than zero"));
    }
    let Some(custody) = custody::Custody::from_env() else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Custody is not configured on this server".into(),
        ));
    };
    let at = now();
    let (user_id, address, secret, owed) = {
        let mut db = app.lock().unwrap();
        let u = auth(&db, &h, false)?;
        let tx = db.transaction()?;
        settle_before_withdrawal(&tx, &u.id, at)?;
        let owed = outstanding_fees(&tx, &u.id)?;
        let address = custody::address(&tx, &u.id)?.ok_or(ApiError(
            StatusCode::CONFLICT,
            "This account has no managed wallet".into(),
        ))?;
        if address == req.destination {
            return Err(bad("Destination is this account's own managed wallet"));
        }
        let sealed: String = tx.query_row(
            "SELECT secret FROM managed_wallets WHERE user_id=?1 AND released=0",
            [&u.id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        let secret = custody.open(&sealed).ok_or(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Managed wallet key could not be opened".into(),
        ))?;
        (u.id, address, secret, owed)
    };
    let balance = custody::balance(&rpc_url(), &address)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let available = transfer::withdrawable(balance, owed);
    if req.lamports > available {
        return Err(bad(&format!(
            "Requested {} lamports but only {available} are available: balance {balance}, commission owed {owed}, reserves {} withheld",
            req.lamports,
            transfer::RENT_EXEMPT_LAMPORTS + transfer::FEE_RESERVE_LAMPORTS
        )));
    }
    let from: [u8; 32] = secret[32..]
        .try_into()
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Malformed key".into()))?;
    let to: [u8; 32] = bs58::decode(&req.destination)
        .into_vec()
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
        .ok_or(bad("Destination is not a 32-byte address"))?;
    let blockhash = transfer::latest_blockhash(&rpc_url())
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let message = transfer::build_message(&from, &to, req.lamports, &blockhash);
    let wire = transfer::sign(&message, &secret).ok_or(ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Withdrawal could not be signed".into(),
    ))?;
    // Recorded before broadcasting: a send that times out may still land, so an
    // unrecorded attempt would be an untracked payment.
    let id = Uuid::new_v4().to_string();
    {
        let db = app.lock().unwrap();
        db.execute(
            "INSERT INTO withdrawals(id,user_id,destination,lamports,status,created) VALUES(?1,?2,?3,?4,'submitted',?5)",
            params![id, user_id, req.destination, req.lamports as i64, at],
        )?;
        audit(&db, &user_id, "withdrawal.submitted", &id)?;
    }
    let result = transfer::send(&rpc_url(), &wire).await;
    let db = app.lock().unwrap();
    match result {
        Ok(signature) => {
            db.execute(
                "UPDATE withdrawals SET status='sent',signature=?1 WHERE id=?2",
                params![signature, id],
            )?;
            audit(&db, &user_id, "withdrawal.sent", &signature)?;
            Ok(Json(json!({
                "id": id, "signature": signature, "lamports": req.lamports,
                "destination": req.destination, "commission_withheld_lamports": owed,
                "note": "Submitted to the network. Confirm the signature on chain before treating it as final."
            })))
        }
        Err(e) => {
            // Unknown, not failed: the transaction may still confirm.
            db.execute(
                "UPDATE withdrawals SET status='unknown',detail=?1 WHERE id=?2",
                params![e, id],
            )?;
            audit(
                &db,
                &user_id,
                "withdrawal.unconfirmed",
                &format!("{id}: {e}"),
            )?;
            Err(ApiError(
                StatusCode::BAD_GATEWAY,
                format!(
                    "{e}. The withdrawal is recorded as unconfirmed and may still land; check before retrying, because retrying sends a second payment."
                ),
            ))
        }
    }
}

async fn withdrawal_history(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let mut stmt = db.prepare("SELECT id,destination,lamports,signature,status,detail,created FROM withdrawals WHERE user_id=?1 ORDER BY created DESC LIMIT 50")?;
    let rows = stmt
        .query_map([&u.id], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"destination":r.get::<_,String>(1)?,"lamports":r.get::<_,i64>(2)?,"signature":r.get::<_,Option<String>>(3)?,"status":r.get::<_,String>(4)?,"detail":r.get::<_,Option<String>>(5)?,"created":r.get::<_,i64>(6)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(
        json!({"withdrawals":rows,"enabled":transfer::enabled()}),
    ))
}

#[derive(Deserialize)]
struct Backfill {
    wallet: String,
    pages: Option<u32>,
}
// Walks a wallet's history backwards so it can reach the paper policy's
// minimum-history gate without a person clicking through pages.
//
// Bounded on purpose: a shared RPC endpoint is rate limited, and the decoder
// still only recognises direct native-SOL Pump/PumpSwap swaps. Pages that
// decode nothing are archived, not invented.
async fn backfill_rpc(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<Backfill>,
) -> ApiResult {
    if !rpc::valid_address(&req.wallet) {
        return Err(bad("Provide a valid Solana wallet"));
    }
    let pages = req.pages.unwrap_or(5).clamp(1, 25);
    {
        let db = app.lock().unwrap();
        let u = auth(&db, &h, true)?;
        let last: i64 = db
            .query_row(
                "SELECT last_attempt FROM rpc_syncs WHERE wallet=?1",
                [&req.wallet],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if now() - last < 30 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Wait 30 seconds between collections for this wallet".into(),
            ));
        }
        db.execute("INSERT INTO rpc_syncs VALUES(?1,?2) ON CONFLICT(wallet) DO UPDATE SET last_attempt=excluded.last_attempt", params![req.wallet, now()])?;
        audit(
            &db,
            &u.id,
            "rpc.backfill_started",
            &format!("{}: {pages} pages", req.wallet),
        )?;
    }
    let url = rpc_url();
    let (mut scanned, mut archived, mut decoded, mut inserted, mut unavailable) = (0, 0, 0, 0, 0);
    let mut before: Option<String> = None;
    let mut completed = 0;
    let mut stopped = "Requested pages collected";
    for _ in 0..pages {
        let page = match rpc::fetch(&url, &req.wallet, before.as_deref()).await {
            Ok(page) => page,
            Err(e) => {
                // Partial history is still history. Report where it stopped
                // rather than discarding what was already collected.
                stopped = "Collection stopped early; the provider returned an error";
                eprintln!("backfill: {e}");
                break;
            }
        };
        scanned += page.scanned;
        unavailable += page.failed;
        {
            let mut db = app.lock().unwrap();
            let tx = db.transaction()?;
            for snap in &page.snapshots {
                archived += tx.execute(
                    "INSERT OR IGNORE INTO raw_transactions VALUES(?1,?2,?3,?4)",
                    params![snap.signature, req.wallet, snap.data.to_string(), now()],
                )?;
                if let Some(t) = &snap.trade {
                    decoded += 1;
                    inserted += tx.execute(
                        "INSERT OR IGNORE INTO trades VALUES(?1,?2,?3)",
                        params![t.id, t.timestamp, serde_json::to_string(t).unwrap()],
                    )?;
                }
            }
            tx.commit()?;
        }
        completed += 1;
        if page.scanned == 0 || page.next_before.is_none() {
            stopped = "Reached the end of the history this provider retains";
            break;
        }
        before = page.next_before;
    }
    let (matched_sells, score) = {
        let db = app.lock().unwrap();
        let u = auth(&db, &h, true)?;
        let all = trades(&db)?;
        let a = analyze(&req.wallet, &all);
        audit(
            &db,
            &u.id,
            "rpc.backfilled",
            &format!("{}: {completed} pages, {inserted} new trades", req.wallet),
        )?;
        let _ = cluster::index(&db, &req.wallet);
        (a.matched_sells, a.score)
    };
    Ok(Json(json!({
        "pages": completed, "scanned": scanned, "archived": archived, "decoded": decoded,
        "inserted": inserted, "unavailable": unavailable, "next_before": before, "stopped": stopped,
        "matched_sells": matched_sells, "score": score,
        "ready_for_paper": matched_sells >= 20 && score >= 65,
        "coverage": "Direct native-SOL Pump/PumpSwap swaps only; routed and multi-asset trades are archived but not decoded. Liquidity is unknown for RPC observations, which blocks paper entries for them."
    })))
}

// Re-runs the current decoder over everything already archived.
//
// The archive exists so that a better decoder can revisit history: a
// transaction stored when only direct calls were understood becomes a trade
// once routed swaps are. Existing trade rows are never rewritten, only added.
async fn redecode(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let mut db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let archived = {
        let mut stmt = db.prepare("SELECT signature,wallet,data FROM raw_transactions")?;
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let tx = db.transaction()?;
    let (mut decoded, mut inserted, mut routed) = (0, 0, 0);
    for (signature, wallet, data) in &archived {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let Some(trade) = rpc::decode(wallet, signature, &value) else {
            continue;
        };
        decoded += 1;
        if trade.routed == Some(true) {
            routed += 1;
        }
        inserted += tx.execute(
            "INSERT OR IGNORE INTO trades VALUES(?1,?2,?3)",
            params![
                trade.id,
                trade.timestamp,
                serde_json::to_string(&trade).unwrap()
            ],
        )?;
    }
    audit(
        &tx,
        &u.id,
        "rpc.redecoded",
        &format!(
            "{} archived, {decoded} decoded, {inserted} new",
            archived.len()
        ),
    )?;
    tx.commit()?;
    Ok(Json(json!({
        "archived": archived.len(), "decoded": decoded, "routed": routed,
        "inserted": inserted,
        "note": "Existing trades are never rewritten. A transaction the decoder still does not recognise stays archived for a future attempt."
    })))
}

// The token page's identity and social panel.
//
// Reuse is derived from our own record and costs nothing. Identity resolution
// is metered by X, so it happens once per account and is cached; a page view
// never triggers it.
async fn token_page(State(app): State<App>, h: HeaderMap, Path(mint): Path<String>) -> ApiResult {
    if !rpc::valid_address(&mint) {
        return Err(bad("Provide a valid token mint address"));
    }
    let db = app.lock().unwrap();
    auth(&db, &h, false)?;
    let token = db
        .query_row(
            "SELECT name,symbol,uri,creator,twitter,telegram,website,first_seen,metadata_fetched FROM tokens WHERE mint=?1",
            [&mint],
            |r| {
                Ok(json!({"name":r.get::<_,Option<String>>(0)?,"symbol":r.get::<_,Option<String>>(1)?,
                    "uri":r.get::<_,Option<String>>(2)?,"creator":r.get::<_,Option<String>>(3)?,
                    "twitter":r.get::<_,Option<String>>(4)?,"telegram":r.get::<_,Option<String>>(5)?,
                    "website":r.get::<_,Option<String>>(6)?,"first_seen":r.get::<_,i64>(7)?,
                    "metadata_fetched":r.get::<_,Option<i64>>(8)?}))
            },
        )
        .optional()?;
    let handle = token
        .as_ref()
        .and_then(|t| t["twitter"].as_str())
        .and_then(social::handle);
    let social = match &handle {
        Some(handle) => {
            Some(serde_json::to_value(social::reputation(&db, handle, &mint)?).unwrap())
        }
        None => None,
    };
    let risk = db
        .query_row(
            "SELECT blocked_reason,score,checked FROM token_risk WHERE mint=?1",
            [&mint],
            |r| {
                Ok(
                    json!({"blocked":r.get::<_,Option<String>>(0)?,"score":r.get::<_,Option<f64>>(1)?,"checked":r.get::<_,i64>(2)?}),
                )
            },
        )
        .optional()?;
    let last_price: Option<f64> = db
        .query_row(
            "SELECT price_sol FROM price_ticks WHERE token=?1 ORDER BY timestamp DESC LIMIT 1",
            [&mint],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Json(json!({
        "mint": mint, "token": token, "social": social, "risk": risk,
        "last_price_sol": last_price,
        "note": "Social reuse is derived from mints this service has observed itself, so it is a lower bound. A token absent from our record is unknown, not new."
    })))
}

#[derive(Deserialize)]
struct ResolveRequest {
    handle: String,
}
// Metered: one X read per account, cached forever afterwards. Administrator
// only so a page view can never spend the budget.
async fn resolve_social(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<ResolveRequest>,
) -> ApiResult {
    let Some(handle) = social::handle(&req.handle) else {
        return Err(bad("Not a usable X account handle"));
    };
    {
        let db = app.lock().unwrap();
        auth(&db, &h, true)?;
    }
    let bearer = std::env::var("X_API_BEARER").ok().filter(|b| !b.is_empty());
    let Some(bearer) = bearer else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "X identity resolution is not configured. Reuse across mints still works without it; rename detection does not.".into(),
        ));
    };
    let (user_id, followers, created) = social::resolve(&bearer, &handle)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    social::record_identity(&db, &user_id, &handle, followers, created, now())?;
    audit(
        &db,
        &u.id,
        "social.resolved",
        &format!("{handle} -> {user_id}"),
    )?;
    Ok(Json(
        json!({"handle":handle,"user_id":user_id,"followers":followers,"account_created":created}),
    ))
}

// Newly observed mints, newest first.
//
// One deployer minting the same name over and over is the most common thing on
// this stream — twelve identical "YO / LAST COIN 3.3 MILL" mints from a single
// address inside forty seconds is a real observation, not duplicated rows. Left
// as-is it buries every other launch, so a batch collapses to its newest mint
// and carries the count. The count is the interesting part: a deployer who
// mints the same coin twelve times is telling you something about the coin.
//
// Grouping is by (creator, symbol, name). A mint whose creator is unknown
// groups only with itself, because without a deployer two coins sharing a name
// are just two coins sharing a name.
const TOKEN_GROUP: &str = "COALESCE(t.creator,t.mint)||char(31)||LOWER(COALESCE(t.symbol,''))||char(31)||LOWER(COALESCE(t.name,''))";

async fn token_list(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    auth(&db, &h, false)?;
    let tokens = rows(
        &db,
        &format!(
            "SELECT mint,name,symbol,twitter,first_seen,creator,copies,handle_tokens FROM (
               SELECT t.mint,t.name,t.symbol,t.twitter,t.first_seen,t.creator,
                 COUNT(*) OVER (PARTITION BY {TOKEN_GROUP}) AS copies,
                 ROW_NUMBER() OVER (PARTITION BY {TOKEN_GROUP} ORDER BY t.first_seen DESC,t.mint) AS rn,
                 (SELECT COUNT(*) FROM token_socials s2 WHERE s2.handle=(SELECT handle FROM token_socials s1 WHERE s1.mint=t.mint LIMIT 1)) AS handle_tokens
               FROM tokens t
             ) WHERE rn=1 ORDER BY first_seen DESC LIMIT 100"
        ),
    )?;
    Ok(Json(
        json!({"tokens":tokens,"note":"Mints observed by this service since the collector was last running. Not a complete list of what launched. Repeat mints of the same name by the same deployer are shown once, with the count."}),
    ))
}

#[derive(Deserialize)]
struct SwapRequest {
    input_mint: String,
    output_mint: String,
    amount: u64,
    slippage_bps: u16,
    /// Address that will sign. Omitted means "use the wallet this account
    /// signed in with, or its managed wallet".
    taker: Option<String>,
}
/// The signer for this account: the wallet they authenticated with if there is
/// one, otherwise the managed wallet. Never an address the caller supplied for
/// someone else.
fn resolve_taker(
    db: &Connection,
    user: &User,
    requested: Option<&str>,
) -> Result<String, ApiError> {
    if let Some(address) = requested {
        let owned: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM wallet_identities WHERE user_id=?1 AND address=?2 AND chain='solana')",
            params![user.id, address],
            |r| r.get(0),
        )?;
        let managed = custody::address(db, &user.id)?;
        if !owned && managed.as_deref() != Some(address) {
            return Err(bad(
                "That address is not linked to this account. Sign in with it first.",
            ));
        }
        return Ok(address.to_string());
    }
    if let Some(address) = db
        .query_row(
            "SELECT address FROM wallet_identities WHERE user_id=?1 AND chain='solana' ORDER BY created LIMIT 1",
            [&user.id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(address);
    }
    // Accounts created before custody was configured, including the bootstrap
    // administrator, have no managed wallet yet. Issue one on first use rather
    // than leaving the account unable to trade.
    if let Some(address) = custody::address(db, &user.id)?.or(provision_wallet(db, &user.id)?) {
        return Ok(address);
    }
    Err(bad(
        "No Solana wallet is linked to this account. Sign in with Phantom to trade.",
    ))
}

// A price, with the cost of taking it stated plainly.
async fn swap_quote(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<SwapRequest>,
) -> ApiResult {
    {
        let db = app.lock().unwrap();
        auth(&db, &h, false)?;
    }
    let cfg = swap::Config::from_env();
    let request = swap::validate(
        &req.input_mint,
        &req.output_mint,
        req.amount,
        req.slippage_bps,
        &cfg,
    )
    .map_err(|e| bad(&e))?;
    let quote = swap::quote(&cfg, &request)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let summary = swap::summarize(&quote, &cfg).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            "Router returned an unreadable quote".into(),
        )
    })?;
    Ok(Json(json!({
        "quote": summary,
        "note": "Indicative. The transaction is quoted again at build time, and the price you get is whatever the network fills at within your slippage."
    })))
}

// Builds an unsigned transaction for the user's own wallet to sign.
//
// The quote is fetched here rather than accepted from the caller: it fixes the
// route, the amounts and the fee, so it has to be ours.
async fn swap_build(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<SwapRequest>,
) -> ApiResult {
    let (user_id, taker) = {
        let db = app.lock().unwrap();
        let u = auth(&db, &h, false)?;
        let taker = resolve_taker(&db, &u, req.taker.as_deref())?;
        (u.id, taker)
    };
    let cfg = swap::Config::from_env();
    let request = swap::validate(
        &req.input_mint,
        &req.output_mint,
        req.amount,
        req.slippage_bps,
        &cfg,
    )
    .map_err(|e| bad(&e))?;
    let quote = swap::quote(&cfg, &request)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let summary = swap::summarize(&quote, &cfg).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            "Router returned an unreadable quote".into(),
        )
    })?;
    let unsigned = swap::build(&cfg, &quote, &taker)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let id = Uuid::new_v4().to_string();
    {
        let db = app.lock().unwrap();
        db.execute("INSERT INTO swaps(id,user_id,taker,input_mint,output_mint,in_amount,out_amount,minimum_out,platform_fee,slippage_bps,status,created) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'built',?11)",
            params![id, user_id, taker, request.input_mint, request.output_mint, summary.in_amount as i64,
                summary.out_amount as i64, summary.minimum_out as i64, summary.platform_fee as i64,
                request.slippage_bps, now()])?;
        audit(&db, &user_id, "swap.built", &id)?;
    }
    Ok(Json(json!({
        "id": id, "transaction": unsigned.transaction, "taker": taker,
        "last_valid_block_height": unsigned.last_valid_block_height,
        "prioritization_fee_lamports": unsigned.prioritization_fee_lamports,
        "quote": summary,
        "note": "Unsigned. Your wallet signs and broadcasts it; this service never holds the key that signs this transaction."
    })))
}

#[derive(Deserialize)]
struct SwapResult {
    id: String,
    signature: Option<String>,
    /// False when the wallet rejected or the broadcast failed outright.
    submitted: bool,
}
// The client reports what its wallet did. A swap we never hear back about stays
// `built`, which is honest: we do not know whether it landed.
async fn swap_submitted(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<SwapResult>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    if req.signature.as_ref().is_some_and(|s| s.len() > 128) {
        return Err(bad("Malformed signature"));
    }
    let status = if req.submitted {
        "submitted"
    } else {
        "abandoned"
    };
    let changed = db.execute(
        "UPDATE swaps SET status=?1,signature=?2,settled=?3 WHERE id=?4 AND user_id=?5 AND status='built'",
        params![status, req.signature, now(), req.id, u.id],
    )?;
    if changed == 0 {
        return Err(bad("No pending swap with that id for this account"));
    }
    audit(&db, &u.id, &format!("swap.{status}"), &req.id)?;
    Ok(Json(
        json!({"ok":true,"status":status,"note":"Confirm the signature on chain before treating the trade as final."}),
    ))
}

async fn swap_history(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let mut stmt = db.prepare("SELECT id,input_mint,output_mint,in_amount,out_amount,platform_fee,slippage_bps,status,signature,created FROM swaps WHERE user_id=?1 ORDER BY created DESC LIMIT 100")?;
    let rows = stmt
        .query_map([&u.id], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"input_mint":r.get::<_,String>(1)?,"output_mint":r.get::<_,String>(2)?,
                "in_amount":r.get::<_,i64>(3)?,"out_amount":r.get::<_,i64>(4)?,"platform_fee":r.get::<_,i64>(5)?,
                "slippage_bps":r.get::<_,i64>(6)?,"status":r.get::<_,String>(7)?,"signature":r.get::<_,Option<String>>(8)?,
                "created":r.get::<_,i64>(9)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let cfg = swap::Config::from_env();
    Ok(Json(json!({
        "swaps": rows,
        "platform_fee_bps": cfg.effective_fee_bps(),
        "fee_collected": cfg.fee_account.is_some(),
        "note": "A swap left as 'built' was never reported back by the wallet; whether it landed is unknown."
    })))
}

/// Market data for one mint, cached, with its age.
///
/// Also feeds the social ledger: a token's X handle learned here counts toward
/// reuse exactly as one learned from a launch does, which extends the signal
/// to tokens that never passed through the creation stream.
async fn market_for(app: &App, mint: &str) -> Result<(Value, i64), ApiError> {
    let cfg = market::Config::from_env();
    let at = now();
    if let Some((raw, age)) = {
        let db = app.lock().unwrap();
        market::cached(&db, mint, &cfg, at)?
    } {
        return Ok((raw, age));
    }
    let raw = market::fetch(&cfg, mint)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let db = app.lock().unwrap();
    market::store(&db, mint, &raw, at)?;
    if let Some(m) = market::best_pair(&raw, mint)
        && let Some(handle) = market::twitter_handle(&m)
    {
        db.execute(
            "INSERT OR IGNORE INTO tokens(mint,name,symbol,first_seen) VALUES(?1,?2,?3,?4)",
            params![mint, m.name, m.symbol, at],
        )?;
        social::link(&db, mint, &handle, at)?;
    }
    Ok((raw, 0))
}

async fn market_page(State(app): State<App>, h: HeaderMap, Path(mint): Path<String>) -> ApiResult {
    if !rpc::valid_address(&mint) {
        return Err(bad("Provide a valid token mint address"));
    }
    {
        let db = app.lock().unwrap();
        auth(&db, &h, false)?;
    }
    let (raw, age) = market_for(&app, &mint).await?;
    let Some(m) = market::best_pair(&raw, &mint) else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No market has been found for this mint. It may not be trading yet.".into(),
        ));
    };
    Ok(Json(json!({
        "mint": mint, "market": m, "age_seconds": age,
        "note": "Deepest pool only. Figures are as of the age shown, not live ticks."
    })))
}

// The discovery list. Newly profiled tokens from the market provider, merged
// with mints this service saw launch itself.
async fn discover(State(app): State<App>, h: HeaderMap) -> ApiResult {
    {
        let db = app.lock().unwrap();
        auth(&db, &h, false)?;
    }
    let cfg = market::Config::from_env();
    let profiles = market::latest_profiles(&cfg).await.unwrap_or_default();
    let listed: Vec<Value> = profiles
        .iter()
        .filter(|p| p["chainId"].as_str() == Some("solana"))
        .filter_map(|p| {
            let mint = p["tokenAddress"].as_str()?;
            rpc::valid_address(mint).then(|| {
                json!({"mint":mint,"icon":p["icon"],"description":p["description"],"links":p["links"]})
            })
        })
        .take(40)
        .collect();
    // One batched request prices the whole page; per-card lookups would not fit
    // the provider's rate limit.
    let mints: Vec<String> = listed
        .iter()
        .filter_map(|t| t["mint"].as_str().map(str::to_owned))
        .collect();
    let priced = match market::fetch_many(&cfg, &mints).await {
        Ok(raw) => market::best_per_mint(&raw, &mints),
        Err(_) => Default::default(),
    };
    let listed: Vec<Value> = listed
        .into_iter()
        .map(|mut t| {
            let mint = t["mint"].as_str().unwrap_or_default().to_string();
            // A token with no pool carries an explicitly absent market, so the
            // interface says "not trading yet" rather than drawing a blank row.
            t["market"] = priced
                .get(&mint)
                .and_then(|m| serde_json::to_value(m).ok())
                .unwrap_or(Value::Null);
            t
        })
        .collect();
    let db = app.lock().unwrap();
    let observed = rows(
        &db,
        "SELECT mint,name,symbol,twitter,first_seen FROM tokens ORDER BY first_seen DESC LIMIT 40",
    )?;
    Ok(Json(json!({
        "profiled": listed, "observed": observed,
        "note": "Profiled tokens come from the market provider and are not an endorsement; observed tokens are launches this service saw itself. Neither list is complete."
    })))
}

#[derive(Deserialize)]
struct CandleQuery {
    #[serde(default)]
    tf: String,
}
// Candles for the deepest pool. The pool lookup is cached separately from the
// candles, because it changes far less often than the price does.
async fn token_candles(
    State(app): State<App>,
    h: HeaderMap,
    Path(mint): Path<String>,
    axum::extract::Query(q): axum::extract::Query<CandleQuery>,
) -> ApiResult {
    if !rpc::valid_address(&mint) {
        return Err(bad("Provide a valid token mint address"));
    }
    let key = if q.tf.is_empty() { "1h" } else { &q.tf };
    let Some((unit, aggregate)) = chart::timeframe(key) else {
        return Err(bad("Unsupported timeframe"));
    };
    {
        let db = app.lock().unwrap();
        auth(&db, &h, false)?;
    }
    let cfg = chart::Config::from_env();
    let at = now();
    let pool_key = format!("pool:{mint}");
    let cached_pool = {
        let db = app.lock().unwrap();
        chart::cached(&db, &pool_key, &cfg, at)?
    };
    let pool = match cached_pool {
        Some((v, _)) => v.as_str().map(str::to_owned),
        None => {
            let found = chart::pools(&cfg, &mint)
                .await
                .ok()
                .as_ref()
                .and_then(chart::deepest_pool);
            if let Some(p) = &found {
                let db = app.lock().unwrap();
                chart::store(&db, &pool_key, &json!(p), at)?;
            }
            found
        }
    };
    let Some(pool) = pool else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No pool was found for this mint, so there is nothing to chart.".into(),
        ));
    };
    let candles_key = format!("ohlcv:{pool}:{key}");
    let cached_candles = {
        let db = app.lock().unwrap();
        chart::cached(&db, &candles_key, &cfg, at)?
    };
    let (raw, age) = match cached_candles {
        Some((v, age)) => (v, age),
        None => {
            let raw = chart::ohlcv(&cfg, &pool, unit, aggregate)
                .await
                .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
            let db = app.lock().unwrap();
            chart::store(&db, &candles_key, &raw, at)?;
            (raw, 0)
        }
    };
    let candles = chart::candles(&raw);
    Ok(Json(json!({
        "mint": mint, "pool": pool, "timeframe": key, "age_seconds": age,
        "candles": candles,
        "timeframes": chart::TIMEFRAMES.map(|(k, _, _)| k),
        "note": "Deepest pool only. Candles from other pools are not blended in, because a price nobody could trade at is not a price."
    })))
}

/// Prices a set of mints from the market cache, refreshing what is missing.
/// A mint we cannot price stays unpriced rather than counting as zero.
async fn price_mints(app: &App, mints: &[String]) -> BTreeMap<String, f64> {
    let mut prices = BTreeMap::new();
    for mint in mints.iter().take(25) {
        if let Ok((raw, _)) = market_for(app, mint).await
            && let Some(m) = market::best_pair(&raw, mint)
            && let Some(price) = m.price_usd
        {
            prices.insert(mint.clone(), price);
        }
    }
    prices
}

// What the account actually holds on chain, priced where possible.
async fn portfolio(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let (user, address) = {
        let db = app.lock().unwrap();
        let u = auth(&db, &h, false)?;
        let address = db
            .query_row(
                "SELECT address FROM wallet_identities WHERE user_id=?1 AND chain='solana' ORDER BY created LIMIT 1",
                [&u.id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .or(custody::address(&db, &u.id)?);
        (u, address)
    };
    let Some(address) = address else {
        return Err(bad(
            "No Solana wallet is linked to this account. Sign in with Phantom to see a portfolio.",
        ));
    };
    let url = rpc_url();
    let sol = custody::balance(&url, &address).await;
    let tokens = holdings::fetch_holdings(&url, &address).await;
    let (tokens, holdings_error) = match tokens {
        Ok(t) => (t, None),
        Err(e) => (vec![], Some(e)),
    };
    let mints: Vec<String> = tokens.iter().map(|t| t.mint.clone()).collect();
    let prices = price_mints(&app, &mints).await;
    let mut valued = 0.0;
    let mut unpriced = 0;
    let rows: Vec<Value> = tokens
        .iter()
        .map(|t| {
            let price = prices.get(&t.mint).copied();
            match price {
                Some(p) => valued += p * t.amount,
                None => unpriced += 1,
            }
            json!({"mint":t.mint,"amount":t.amount,"raw_amount":t.raw_amount,"decimals":t.decimals,
                "price_usd":price,"value_usd":price.map(|p| p * t.amount)})
        })
        .collect();
    Ok(Json(json!({
        "address": address,
        "sol_lamports": sol.as_ref().ok(),
        "sol_error": sol.as_ref().err(),
        "tokens": rows,
        "valued_usd": valued,
        "unpriced_tokens": unpriced,
        "holdings_error": holdings_error,
        "subscribed": user.expires > now(),
        "note": "Token value excludes anything we could not price; that count is reported rather than folded into the total as zero."
    })))
}

// Who holds the token, and who has traded it well in our own record.
async fn token_holders(
    State(app): State<App>,
    h: HeaderMap,
    Path(mint): Path<String>,
) -> ApiResult {
    if !rpc::valid_address(&mint) {
        return Err(bad("Provide a valid token mint address"));
    }
    {
        let db = app.lock().unwrap();
        auth(&db, &h, false)?;
    }
    let (holders, supply) = holdings::fetch_largest(&rpc_url(), &mint)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let db = app.lock().unwrap();
    // Traders we have actually observed on this token, ranked by realized
    // result. This is our own record, not a global leaderboard.
    let all = trades(&db)?;
    let wallets: std::collections::BTreeSet<String> = all
        .iter()
        .filter(|t| t.token == mint)
        .map(|t| t.wallet.clone())
        .collect();
    let mut traders: Vec<Value> = wallets
        .iter()
        .map(|w| {
            let a = analyze(w, &all);
            json!({"wallet":w,"realized_pnl_sol":a.realized_pnl_sol,"round_trips":a.round_trips,
                "round_trip_win_rate":a.round_trip_win_rate,"score":a.score})
        })
        .collect();
    traders.sort_by(|a, b| {
        b["realized_pnl_sol"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["realized_pnl_sol"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    traders.truncate(20);
    Ok(Json(json!({
        "mint": mint, "supply": supply, "holders": holders, "top_traders": traders,
        "note": "Holders are token accounts, not people: one owner can hold through several, and the largest is usually a pool rather than a person. Top traders covers only wallets this service has observed, so it is not a global ranking."
    })))
}

// The social feed: what the wallets you follow just did, and what it did to you.
async fn follow_feed(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let mut stmt = db.prepare(
        "SELECT t.data FROM trades t JOIN watches w ON w.wallet=json_extract(t.data,'$.wallet') WHERE w.user_id=?1 ORDER BY t.timestamp DESC LIMIT 60",
    )?;
    let events: Vec<Trade> = stmt
        .query_map([&u.id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect();
    let mut positions = db.prepare(
        "SELECT wallet,token,symbol,quantity,cost,entry,exit,pnl,status,reason,created FROM positions WHERE user_id=?1 ORDER BY created DESC LIMIT 100",
    )?;
    let mine: Vec<Value> = positions
        .query_map([&u.id], |r| {
            Ok(json!({"wallet":r.get::<_,String>(0)?,"token":r.get::<_,String>(1)?,"symbol":r.get::<_,String>(2)?,
                "quantity":r.get::<_,f64>(3)?,"cost":r.get::<_,f64>(4)?,"entry":r.get::<_,f64>(5)?,
                "exit":r.get::<_,Option<f64>>(6)?,"pnl":r.get::<_,Option<f64>>(7)?,"status":r.get::<_,String>(8)?,
                "reason":r.get::<_,String>(9)?,"created":r.get::<_,i64>(10)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    // Mark open paper positions against the latest observed tick, flagged as
    // unrealized so it is never confused with a booked result.
    let marked: Vec<Value> = mine
        .into_iter()
        .map(|mut p| {
            if p["status"] == "open" {
                let token = p["token"].as_str().unwrap_or_default().to_string();
                let last: Option<f64> = db
                    .query_row(
                        "SELECT price_sol FROM price_ticks WHERE token=?1 ORDER BY timestamp DESC LIMIT 1",
                        [&token],
                        |r| r.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten();
                p["mark_price"] = json!(last);
                p["unrealized_pnl_sol"] = json!(last.map(|price| {
                    p["quantity"].as_f64().unwrap_or(0.0) * price - p["cost"].as_f64().unwrap_or(0.0)
                }));
            }
            p
        })
        .collect();
    Ok(Json(json!({
        "events": events, "positions": marked,
        "note": "Events are trades by wallets you follow, from this service's own record. Open positions are marked against the last observed tick, which is unrealized and not a booked result; a position with no tick shows no mark rather than a stale one."
    })))
}

#[derive(Deserialize)]
struct ExportRequest {
    confirm: String,
}
// Exporting is what the incumbents do at signup: the owner can always take
// their key. It does not end custody, and the response refuses to imply it does.
async fn export_key(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<ExportRequest>,
) -> ApiResult {
    if req.confirm != "EXPORT" {
        return Err(bad("Send confirm:\"EXPORT\" to reveal the private key"));
    }
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    let Some(custody) = custody::Custody::from_env() else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Custody is not configured on this server".into(),
        ));
    };
    let Some((address, secret)) = custody::export(&db, &custody, &u.id)? else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No managed wallet key is available to export".into(),
        ));
    };
    // The key itself is never written to the audit log, only the fact of it.
    audit(&db, &u.id, "wallet.exported", &address)?;
    Ok(Json(json!({
        "address": address,
        "secret_key": secret,
        "format": "base58-encoded 64-byte keypair, importable into Phantom or the Solana CLI",
        "custody_ended": false,
        "warning": "This is a copy, not a handover. This service still holds this key and can still sign for this wallet. For sole control, move the funds to a wallet this service never generated, or close the account, which destroys our copy."
    })))
}

// Verifies the configured fee account before it silently collects nothing.
//
// A fee account must be a real token account whose mint is one side of the
// trades being charged. A wrong one does not error at quote time: Jupiter
// simply refuses the fee, so revenue quietly stays at zero.
async fn verify_fee_account(State(app): State<App>, h: HeaderMap) -> ApiResult {
    {
        let db = app.lock().unwrap();
        auth(&db, &h, true)?;
    }
    let cfg = swap::Config::from_env();
    let Some(account) = cfg.fee_account.clone() else {
        return Ok(Json(json!({
            "configured": false, "collecting": false,
            "detail": "PLATFORM_FEE_ACCOUNT is not set, so no fee is requested on any swap and every quote reports 0 bps.",
            "how_to_fix": "Create a wrapped-SOL token account you control and set it here. One WSOL account covers every SOL-paired trade in both directions."
        })));
    };
    if !rpc::valid_address(&account) {
        return Ok(Json(json!({
            "configured": true, "collecting": false,
            "detail": "PLATFORM_FEE_ACCOUNT is not a valid Solana address."
        })));
    }
    let raw = rpc::account_info(&rpc_url(), &account)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    let exists = !raw["value"].is_null();
    let parsed = &raw["value"]["data"]["parsed"];
    let mint = parsed["info"]["mint"].as_str();
    let is_token_account = parsed["type"].as_str() == Some("account");
    let wsol = mint == Some("So11111111111111111111111111111111111111112");
    Ok(Json(json!({
        "configured": true,
        "collecting": is_token_account,
        "account": account,
        "mint": mint,
        "owner": parsed["info"]["owner"].as_str(),
        "is_wrapped_sol": wsol,
        "platform_fee_bps": cfg.platform_fee_bps,
        "exists": exists,
        "detail": if !exists {
            // Two very different situations look identical from here, so say
            // both rather than guessing: it has simply never been used yet, or
            // it was deleted by a close-based sweep.
            "This account does not exist on chain yet. If no swap has happened since you configured it, that is expected — the router creates it on the first trade. If trades have happened, it was probably deleted by sweeping with a close, and every swap since collected nothing; sweep with UnwrapLamports instead, which leaves the account open."
        } else if !is_token_account {
            "This address exists but is not a token account, so the router will refuse the fee and you will collect nothing."
        } else if wsol {
            "Valid wrapped-SOL token account. Fees are collected on every SOL-paired swap."
        } else {
            "Valid token account, but not wrapped SOL. Fees are only collected on swaps where this mint is one side."
        },
        "peers": "Comparable platforms charge 0.5% to 1% per swap: Photon 0.5%, Axiom about 1% falling to 0.75% at its top tier, GMGN 1% flat."
    })))
}

// EVIDENCE C5 and C9: how fast a token filled, and how much money the strategy
// can actually carry. Both are per-caller, because capacity depends on size.
async fn wallet_capacity(
    State(app): State<App>,
    h: HeaderMap,
    Path(address): Path<String>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    require_subscription(&u, "capacity analysis")?;
    let all = trades(&db)?;
    if !all.iter().any(|t| t.wallet == address) {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "No imported history for this wallet".into(),
        ));
    }
    let budget: f64 = db.query_row("SELECT budget FROM users WHERE id=?1", [&u.id], |r| {
        r.get(0)
    })?;
    let report = velocity::capacity(
        &address,
        &all,
        budget,
        min_liquidity_sol(),
        &fees::CostModel::from_env(),
        &[1, 10, 100, 1000],
    );
    Ok(Json(json!({
        "capacity": report,
        "regime": velocity::wallet_regime(&address, &all),
        "note": "Order sizes are the ones this policy would really have used, not an idealised size. Per-follower exposure assumes everyone takes the same size at the same moment, which is the crowding case, not the average one."
    })))
}

async fn token_velocity(
    State(app): State<App>,
    h: HeaderMap,
    Path(mint): Path<String>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, false)?;
    require_subscription(&u, "velocity analysis")?;
    let all = trades(&db)?;
    Ok(Json(json!({
        "velocity": velocity::velocity(&mint, &all),
        "thresholds_sol": velocity::THRESHOLDS,
        "note": "Published work found that reaching a given depth in fewer trades predicted graduation better than any other variable tested. That was measured on complete data; this is measured on ours."
    })))
}

#[derive(Deserialize)]
struct PlatformSetting {
    copy_trading: bool,
}
// The operator's switch for copy trading. Turning it off stops paper execution
// everywhere, including the live collector, and refuses re-enabling per user.
async fn platform_settings(
    State(app): State<App>,
    h: HeaderMap,
    Json(req): Json<PlatformSetting>,
) -> ApiResult {
    let db = app.lock().unwrap();
    let u = auth(&db, &h, true)?;
    let value = if req.copy_trading { "on" } else { "off" };
    db.execute("INSERT INTO platform_settings VALUES('copy_trading',?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated=excluded.updated", params![value, now()])?;
    if !req.copy_trading {
        // Leaving accounts marked enabled would silently resume the moment the
        // switch flipped back, which is not what "off" should mean.
        db.execute("UPDATE users SET enabled=0", [])?;
    }
    audit(&db, &u.id, "platform.copy_trading", value)?;
    Ok(Json(json!({
        "copy_trading": req.copy_trading,
        "note": if req.copy_trading { "Copy trading is available again. Users must opt in individually." }
                else { "Copy trading is off. Paper execution is disabled for every account and the live collector will not open positions. Swapping is unaffected." }
    })))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}
// One box for everything a person might paste or half-remember.
//
// A mint or wallet address is answered directly rather than searched for:
// somebody pasting 44 base58 characters knows what they want.
async fn search(
    State(app): State<App>,
    h: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> ApiResult {
    let term = query.q.trim();
    if term.is_empty() || term.len() > 128 {
        return Err(bad("Search for a token, a wallet, or paste an address"));
    }
    let db = app.lock().unwrap();
    auth(&db, &h, false)?;
    if rpc::valid_address(term) {
        // Distinguish the two kinds of address we can be handed, so the caller
        // can open the right page instead of guessing.
        let traded: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM trades WHERE json_extract(data,'$.wallet')=?1)",
            [term],
            |r| r.get(0),
        )?;
        return Ok(Json(json!({
            "exact": {"address": term, "kind": if traded { "wallet" } else { "token" }},
            "tokens": [], "wallets": [], "handles": [],
            "note": if traded { "This address has traded in our record, so it opens as a wallet." }
                    else { "Opened as a token. If it is a wallet we have never seen trade, its page will be empty." }
        })));
    }
    let like = format!("%{term}%");
    let tokens = {
        let mut stmt = db.prepare(
            "SELECT mint,name,symbol,twitter FROM tokens WHERE symbol LIKE ?1 OR name LIKE ?1 ORDER BY first_seen DESC LIMIT 12",
        )?;
        stmt.query_map([&like], |r| {
            Ok(
                json!({"mint":r.get::<_,String>(0)?,"name":r.get::<_,Option<String>>(1)?,
                "symbol":r.get::<_,Option<String>>(2)?,"twitter":r.get::<_,Option<String>>(3)?}),
            )
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    // Wallets are searched by address fragment, which is what people paste
    // when they have part of one.
    let wallets = {
        let mut stmt = db.prepare(
            "SELECT DISTINCT wallet FROM candidate_wallets WHERE wallet LIKE ?1 ORDER BY sol_volume DESC LIMIT 8",
        )?;
        stmt.query_map([&like], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let handles = {
        let mut stmt =
            db.prepare("SELECT DISTINCT handle FROM token_socials WHERE handle LIKE ?1 LIMIT 8")?;
        stmt.query_map([&like], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(Json(json!({
        "exact": Value::Null, "tokens": tokens, "wallets": wallets, "handles": handles,
        "note": "Searches what this service has observed, not every token on Solana. Paste a mint address to open something we have not seen."
    })))
}

async fn request_guard(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let method = req.method();
    if !matches!(
        *method,
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        let h = req.headers();
        if h.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site") {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error":"Cross-site writes are not permitted"})),
            )
                .into_response();
        }
        if let Some(origin) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            let allowed = std::env::var("APP_ORIGIN").ok();
            let host = h
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let same = origin
                .strip_prefix("http://")
                .or_else(|| origin.strip_prefix("https://"))
                .is_some_and(|v| v == host);
            if !same && allowed.as_deref() != Some(origin) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error":"Origin is not permitted"})),
                )
                    .into_response();
            }
        }
    }
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        "x-content-type-options",
        header::HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("x-frame-options", header::HeaderValue::from_static("DENY"));
    response.headers_mut().insert(
        "referrer-policy",
        header::HeaderValue::from_static("same-origin"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}
fn router(app: App) -> Router {
    Router::new()
        .route(
            "/api/health",
            get(|| async {
                Json(json!({"status":"ok","execution":"paper","live_available":false}))
            }),
        )
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/wallet/challenge", post(wallet_challenge))
        .route("/api/auth/wallet/verify", post(wallet_verify))
        .route("/api/funds", get(funds))
        .route("/api/fees", get(fee_history))
        .route("/api/account/close", post(close_account))
        .route("/api/funds/export-key", post(export_key))
        .route("/api/admin/fees/verify", get(verify_fee_account))
        .route("/api/funds/withdraw", post(withdraw))
        .route("/api/funds/withdrawals", get(withdrawal_history))
        .route("/api/me", get(me))
        .route("/api/overview", get(overview))
        .route("/api/wallets/{address}", get(wallet))
        .route(
            "/api/wallets/{address}/copyability",
            get(wallet_copyability),
        )
        .route(
            "/api/wallets/{address}/authenticity",
            get(wallet_authenticity),
        )
        .route("/api/wallets/{address}/capacity", get(wallet_capacity))
        .route("/api/tokens/{mint}/velocity", get(token_velocity))
        .route("/api/watches", post(watch))
        .route("/api/watches/{wallet}", delete(unwatch))
        .route("/api/settings", post(settings))
        .route("/api/connectors", get(connectors))
        .route("/api/plans", get(plans))
        .route("/api/keys", post(api_key).delete(revoke_key))
        .route("/api/paper/replay", post(replay))
        .route("/api/positions/{id}/close", post(close_position))
        .route("/api/admin", get(admin_overview))
        .route("/api/admin/users/{id}/ban", post(ban))
        .route("/api/admin/plans", post(save_plan))
        .route("/api/admin/subscriptions", post(grant))
        .route("/api/admin/import", post(import))
        .route("/api/admin/events", post(ingest_event))
        .route("/api/admin/ticks", post(ingest_ticks))
        .route("/api/admin/rpc/sync", post(sync_rpc))
        .route("/api/admin/rpc/backfill", post(backfill_rpc))
        .route("/api/admin/rpc/redecode", post(redecode))
        .route("/api/admin/discovery", get(discovery_overview))
        .route("/api/admin/discovery/promote", post(promote))
        .route("/api/admin/cluster/index", post(index_funding))
        .route("/api/admin/fees/settle", post(settle_fees))
        .route("/api/admin/platform", post(platform_settings))
        .route("/api/admin/risk/token", post(collect_token_risk))
        .route("/api/tokens/{mint}/risk", get(token_risk))
        .route("/api/tokens", get(token_list))
        .route("/api/market/{mint}", get(market_page))
        .route("/api/market/{mint}/candles", get(token_candles))
        .route("/api/discover", get(discover))
        .route("/api/search", get(search))
        .route("/api/portfolio", get(portfolio))
        .route("/api/tokens/{mint}/holders", get(token_holders))
        .route("/api/feed", get(follow_feed))
        .route("/api/swap/quote", post(swap_quote))
        .route("/api/swap/build", post(swap_build))
        .route("/api/swap/submitted", post(swap_submitted))
        .route("/api/swaps", get(swap_history))
        .route("/api/tokens/{mint}", get(token_page))
        .route("/api/admin/social/resolve", post(resolve_social))
        .route(
            "/api/{*path}",
            get(|| async {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error":"API endpoint not found"})),
                )
            }),
        )
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .layer(axum::middleware::from_fn(request_guard))
        .with_state(app)
}
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    tracing_subscriber::fmt::init();
    let email = std::env::var("ADMIN_EMAIL")
        .expect("Set ADMIN_EMAIL in .env")
        .trim()
        .to_lowercase();
    let password = std::env::var("ADMIN_PASSWORD").expect("Set ADMIN_PASSWORD in .env");
    assert!(
        password.len() >= 12 && !password.starts_with("replace-"),
        "Set a unique ADMIN_PASSWORD of at least 12 characters"
    );
    let db = setup(
        &std::env::var("DATABASE_PATH").unwrap_or("unskilled.db".into()),
        &email,
        &password,
    );
    let dist = std::env::var("FRONTEND_DIST").unwrap_or("../frontend/dist".into());
    let state = Arc::new(Mutex::new(db));
    if let Some(cfg) = discovery::Config::from_env() {
        println!("Discovery collector: {} (>= {} SOL)", cfg.url, cfg.min_sol);
        tokio::spawn(discovery::run(state.clone(), cfg));
    }
    if let Some(cfg) = watch::Config::from_env() {
        println!(
            "Live copy feed: {} (up to {} followed wallets)",
            cfg.ws_url, cfg.max_wallets
        );
        tokio::spawn(watch::run(state.clone(), cfg));
    }
    let app = router(state).fallback_service(
        ServeDir::new(&dist).not_found_service(ServeFile::new(format!("{dist}/index.html"))),
    );
    let bind = std::env::var("BIND_ADDRESS").unwrap_or("127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .expect("bind server");
    println!("Unskilled Trade: http://{bind} (paper execution)");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests;
