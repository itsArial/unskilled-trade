use super::*;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;
// Custody and a fast-failing RPC are configured once for the whole test binary:
// handlers read them from the environment, and tests run in parallel.
fn test_environment() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        std::env::set_var("CUSTODY_MASTER_KEY", bs58::encode([5u8; 32]).into_string());
        std::env::set_var("SOLANA_RPC_URL", "http://127.0.0.1:1");
        // A Pump.fun curve migrates near 85 SOL, so the 100 SOL production
        // floor is unreachable on the curve. Lowered here to exercise it.
        std::env::set_var("MIN_LIQUIDITY_SOL", "10");
    });
}
fn app() -> App {
    test_environment();
    Arc::new(Mutex::new(setup(
        ":memory:",
        "admin@test.local",
        "test-admin-password-123",
    )))
}
async fn request(
    app: &App,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, HeaderMap) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let res = router(app.clone())
        .oneshot(
            builder
                .body(
                    body.map(|v| Body::from(v.to_string()))
                        .unwrap_or(Body::empty()),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let data = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, data, headers)
}
async fn login_as(app: &App, email: &str, password: &str) -> String {
    let (status, _, h) = request(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({"email":email,"password":password})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = h.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    cookie
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("session=")
        .unwrap()
        .into()
}
const WHALE: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const QUIET: &str = "So11111111111111111111111111111111111111112";
const MINT: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
async fn admin(app: &App) -> String {
    login_as(app, "admin@test.local", "test-admin-password-123").await
}
async fn member(app: &App) -> String {
    let (status, _, _) = request(
        app,
        "POST",
        "/api/auth/register",
        None,
        Some(json!({"email":"member@test.local","password":"member-password-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    login_as(app, "member@test.local", "member-password-123").await
}
#[tokio::test]
async fn authentication_and_admin_boundaries() {
    let a = app();
    assert_eq!(
        request(&a, "GET", "/api/admin", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let member = member(&a).await;
    assert_eq!(
        request(&a, "GET", "/api/admin", Some(&member), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/import",
            Some(&member),
            Some(json!([]))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let admin = admin(&a).await;
    assert_eq!(
        request(&a, "GET", "/api/admin", Some(&admin), None).await.0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn subscription_required_and_live_execution_rejected() {
    let a = app();
    let m = member(&a).await;
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/settings",
            Some(&m),
            Some(json!({"enabled":true,"budget":0.1,"mode":"paper"}))
        )
        .await
        .0,
        StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/settings",
            Some(&m),
            Some(json!({"enabled":true,"budget":0.1,"mode":"live"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&a, "POST", "/api/keys", Some(&m), None).await.0,
        StatusCode::PAYMENT_REQUIRED
    );
}
#[tokio::test]
async fn bans_revoke_existing_sessions() {
    let a = app();
    let m = member(&a).await;
    let admin = admin(&a).await;
    let (_, u, _) = request(&a, "GET", "/api/me", Some(&m), None).await;
    let path = format!("/api/admin/users/{}/ban", u["id"].as_str().unwrap());
    assert_eq!(
        request(
            &a,
            "POST",
            &path,
            Some(&admin),
            Some(json!({"banned":true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&a, "GET", "/api/me", Some(&m), None).await.0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn grants_are_atomic_and_complimentary_is_not_revenue() {
    let a = app();
    let m = member(&a).await;
    let ad = admin(&a).await;
    let (_, u, _) = request(&a, "GET", "/api/me", Some(&m), None).await;
    let grant = json!({"user_id":u["id"],"plan_id":"operator","reference":"grant-1","paid":false});
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/subscriptions",
            Some(&ad),
            Some(grant.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    let expires =
        request(&a, "GET", "/api/me", Some(&m), None).await.1["subscription_expires"].clone();
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/subscriptions",
            Some(&ad),
            Some(grant)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&a, "GET", "/api/me", Some(&m), None).await.1["subscription_expires"],
        expires
    );
    assert_eq!(
        request(&a, "GET", "/api/admin", Some(&ad), None).await.1["revenue_cents"],
        0
    );
}
#[tokio::test]
async fn api_keys_are_hashed_rotated_and_cannot_admin() {
    let a = app();
    let ad = admin(&a).await;
    let (_, k, _) = request(&a, "POST", "/api/keys", Some(&ad), None).await;
    let k = k["key"].as_str().unwrap();
    assert_eq!(
        request(&a, "GET", "/api/me", Some(k), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(&a, "GET", "/api/admin", Some(k), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    {
        let db = a.lock().unwrap();
        let stored: String = db
            .query_row("SELECT token FROM sessions WHERE kind='api'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_ne!(stored, k);
        assert_eq!(stored, hash(k));
    }
    request(&a, "POST", "/api/keys", Some(&ad), None).await;
    assert_eq!(
        request(&a, "GET", "/api/me", Some(k), None).await.0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn history_replay_has_no_lookahead_and_is_idempotent() {
    let a = app();
    let ad = admin(&a).await;
    let events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    let wallet = events[0]["wallet"].as_str().unwrap();
    let first = request(
        &a,
        "POST",
        "/api/admin/import",
        Some(&ad),
        Some(events.clone()),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1["inserted"], 360);
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/import",
            Some(&ad),
            Some(events.clone())
        )
        .await
        .1["inserted"],
        0
    );
    request(
        &a,
        "POST",
        "/api/watches",
        Some(&ad),
        Some(json!({"wallet":wallet})),
    )
    .await;
    request(
        &a,
        "POST",
        "/api/settings",
        Some(&ad),
        Some(json!({"enabled":true,"budget":0.1,"mode":"paper"})),
    )
    .await;
    let (status, result, _) = request(&a, "POST", "/api/paper/replay", Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(result["opened"].as_u64().unwrap() > 0);
    assert_eq!(
        request(&a, "POST", "/api/paper/replay", Some(&ad), None)
            .await
            .1["opened"],
        0
    );
    let (_, overview, _) = request(&a, "GET", "/api/overview", Some(&ad), None).await;
    let first_entry = overview["positions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["created"].as_i64().unwrap())
        .min()
        .unwrap();
    let preceding = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| {
            t["wallet"] == wallet
                && t["side"] == "sell"
                && t["timestamp"].as_i64().unwrap() < first_entry
        })
        .count();
    assert!(preceding >= 20);
}
#[tokio::test]
async fn malformed_import_does_not_partially_commit() {
    let a = app();
    let ad = admin(&a).await;
    let mut events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    events[1]["quantity"] = json!(-10);
    assert_eq!(
        request(&a, "POST", "/api/admin/import", Some(&ad), Some(events))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&a, "GET", "/api/overview", Some(&ad), None).await.1["wallets"],
        json!([])
    );
}
#[tokio::test]
async fn failed_logins_are_throttled() {
    let a = app();
    for _ in 0..8 {
        assert_eq!(
            request(
                &a,
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"email":"missing@test.local","password":"wrong-password"}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"email":"missing@test.local","password":"wrong-password"}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}
#[tokio::test]
async fn unpaid_overview_omits_detailed_history() {
    let a = app();
    let ad = admin(&a).await;
    let m = member(&a).await;
    let events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    request(&a, "POST", "/api/admin/import", Some(&ad), Some(events)).await;
    let (_, overview, _) = request(&a, "GET", "/api/overview", Some(&m), None).await;
    assert_eq!(overview["wallets"][0]["history"], json!([]));
    assert_eq!(overview["wallets"][0]["curve"], json!([]));
    let wallet = overview["wallets"][0]["wallet"].as_str().unwrap();
    assert_eq!(
        request(&a, "GET", &format!("/api/wallets/{wallet}"), Some(&m), None)
            .await
            .0,
        StatusCode::PAYMENT_REQUIRED
    );
}

#[tokio::test]
async fn realtime_paper_fanout_checks_subscription_and_deduplicates() {
    let a = app();
    let ad = admin(&a).await;
    let m = member(&a).await;
    let events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    request(
        &a,
        "POST",
        "/api/admin/import",
        Some(&ad),
        Some(events.clone()),
    )
    .await;
    let wallet = events[0]["wallet"].as_str().unwrap();
    for token in [&ad, &m] {
        request(
            &a,
            "POST",
            "/api/watches",
            Some(token),
            Some(json!({"wallet":wallet})),
        )
        .await;
    }
    request(
        &a,
        "POST",
        "/api/settings",
        Some(&ad),
        Some(json!({"enabled":true,"budget":0.1,"mode":"paper"})),
    )
    .await;
    // Even a stale enabled flag cannot bypass an expired subscription.
    a.lock()
        .unwrap()
        .execute("UPDATE users SET enabled=1 WHERE admin=0", [])
        .unwrap();
    let mut event = events[0].clone();
    event["id"] = json!("new-observed-event");
    event["timestamp"] = json!(now());
    let (status, r, _) = request(
        &a,
        "POST",
        "/api/admin/events",
        Some(&ad),
        Some(event.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(r["users_evaluated"], 1);
    assert_eq!(r["opened"], 1);
    let (_, r, _) = request(&a, "POST", "/api/admin/events", Some(&ad), Some(event)).await;
    assert_eq!(r["duplicate"], true);
    assert_eq!(r["opened"], 0);
    assert_eq!(
        request(&a, "GET", "/api/overview", Some(&m), None).await.1["positions"],
        json!([])
    );
}
#[tokio::test]
async fn cross_site_writes_are_rejected() {
    let a = app();
    let response = router(a)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/register")
                .header("sec-fetch-site", "cross-site")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"email":"evil@test.local","password":"long-enough-password"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn discovery_is_admin_only_and_ranks_without_claiming_skill() {
    let a = app();
    let member = member(&a).await;
    assert_eq!(
        request(&a, "GET", "/api/admin/discovery", Some(&member), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let admin = admin(&a).await;
    {
        let db = a.lock().unwrap();
        for (i, (side, sol)) in [("buy", 9.0), ("buy", 2.0), ("sell", 4.0)]
            .into_iter()
            .enumerate()
        {
            let o = discovery::Observation {
                wallet: if i == 1 { QUIET } else { WHALE }.into(),
                token: MINT.into(),
                side: side.into(),
                sol_amount: sol,
                token_amount: sol * 1000.0,
                virtual_reserve_sol: 31.5,
                pool: "pump".into(),
                signature: format!("sig{i}"),
            };
            discovery::record(&db, &o, 1000 + i as i64).unwrap();
        }
    }
    let (status, body, _) = request(&a, "GET", "/api/admin/discovery", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK);
    let candidates = body["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["wallet"], WHALE);
    assert_eq!(candidates[0]["sol_volume"], 13.0);
    assert_eq!(candidates[0]["distinct_tokens"], 1);
    assert_eq!(candidates[0]["promoted"], 0);
    assert_eq!(body["observations"], 3);

    // Promotion only marks a wallet for history collection; it grants no eligibility.
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/discovery/promote",
            Some(&member),
            Some(json!({ "wallet": WHALE })),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/discovery/promote",
            Some(&admin),
            Some(json!({"wallet":"not-a-real-address"})),
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, _, _) = request(
        &a,
        "POST",
        "/api/admin/discovery/promote",
        Some(&admin),
        Some(json!({ "wallet": WHALE })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body, _) = request(&a, "GET", "/api/admin/discovery", Some(&admin), None).await;
    assert_eq!(body["candidates"][0]["promoted"], 1);
    // A promoted candidate still has no analyzable history.
    assert_eq!(
        request(
            &a,
            "GET",
            &format!("/api/wallets/{WHALE}"),
            Some(&admin),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_blocked_token_stops_entry_and_the_refusal_is_named() {
    let a = app();
    let ad = admin(&a).await;
    let events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    let wallet = events[0]["wallet"].as_str().unwrap().to_string();
    request(
        &a,
        "POST",
        "/api/admin/import",
        Some(&ad),
        Some(events.clone()),
    )
    .await;
    request(
        &a,
        "POST",
        "/api/watches",
        Some(&ad),
        Some(json!({ "wallet": wallet })),
    )
    .await;
    request(
        &a,
        "POST",
        "/api/settings",
        Some(&ad),
        Some(json!({"enabled":true,"budget":0.1,"mode":"paper"})),
    )
    .await;
    // Block every mint this wallet buys, as a retained mint authority would.
    let blocked = "Mint authority is still active: supply can be inflated".to_string();
    {
        let db = a.lock().unwrap();
        let tokens: std::collections::BTreeSet<String> = events
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["token"].as_str().unwrap().to_string())
            .collect();
        for token in tokens {
            db.execute(
                "INSERT INTO token_risk VALUES(?1,90.0,?2,'{}',?3)",
                params![token, blocked, i64::MAX / 2],
            )
            .unwrap();
        }
    }
    let (status, result, _) = request(&a, "POST", "/api/paper/replay", Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["opened"], 0);
    // The user is told why, not silently given nothing.
    assert!(result["refusals"][&blocked].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn token_risk_detail_requires_a_subscription_and_a_collected_report() {
    let a = app();
    let ad = admin(&a).await;
    let path = format!("/api/tokens/{MINT}/risk");
    assert_eq!(
        request(&a, "GET", &path, Some(&ad), None).await.0,
        StatusCode::NOT_FOUND
    );
    {
        let db = a.lock().unwrap();
        let raw = json!({"mint":MINT,"score_normalised":12,"rugged":false,
            "token":{"mintAuthority":null,"freezeAuthority":null},
            "markets":[{"lp":{"lpLockedPct":100.0}}]});
        db.execute(
            "INSERT INTO token_risk VALUES(?1,12.0,NULL,?2,?3)",
            params![MINT, raw.to_string(), 1000],
        )
        .unwrap();
    }
    let (status, body, _) = request(&a, "GET", &path, Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["assessment"]["mint"], MINT);
    assert_eq!(body["assessment"]["mint_authority"], false);
    assert_eq!(body["blocked"], Value::Null);
    assert_eq!(body["stale"], true); // collected at epoch 1000
    let member = member(&a).await;
    assert_eq!(
        request(&a, "GET", &path, Some(&member), None).await.0,
        StatusCode::PAYMENT_REQUIRED
    );
}

#[tokio::test]
async fn copyability_is_reported_separately_from_leader_profit() {
    let a = app();
    let ad = admin(&a).await;
    let events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    let wallet = events[0]["wallet"].as_str().unwrap().to_string();
    request(
        &a,
        "POST",
        "/api/admin/import",
        Some(&ad),
        Some(events.clone()),
    )
    .await;
    let path = format!("/api/wallets/{wallet}/copyability");
    let (status, body, _) = request(&a, "GET", &path, Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    let decay = body["decay"].as_array().unwrap();
    assert_eq!(decay.len(), 5);
    assert_eq!(decay[0]["delay_seconds"], 0);
    assert_eq!(decay[4]["delay_seconds"], 60);
    assert!(decay[0]["attempts"].as_u64().unwrap() > 0);
    assert!(body["leader_realized_pnl_sol"].is_number());
    assert_eq!(body["order_sol"], 0.1);
    assert!(
        body["flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f.as_str().unwrap().contains("optimistic"))
    );
    // Unknown wallets and unsubscribed members get nothing.
    assert_eq!(
        request(
            &a,
            "GET",
            &format!("/api/wallets/{WHALE}/copyability"),
            Some(&ad),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let member = member(&a).await;
    assert_eq!(
        request(&a, "GET", &path, Some(&member), None).await.0,
        StatusCode::PAYMENT_REQUIRED
    );
}

#[tokio::test]
async fn a_price_tick_closes_a_position_without_any_leader_activity() {
    let a = app();
    let ad = admin(&a).await;
    let user: String = {
        let db = a.lock().unwrap();
        let id: String = db
            .query_row("SELECT id FROM users WHERE admin=1", [], |r| r.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO positions(id,user_id,wallet,token,symbol,quantity,cost,entry,status,reason,created) VALUES('p',?1,'w','tok','TOK',10.0,1.0,1.0,'open','test',?2)",
            params![id, now() - 10],
        )
        .unwrap();
        id
    };
    // Ticks are admin-only and must reject malformed observations outright.
    let member = member(&a).await;
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/ticks",
            Some(&member),
            Some(json!([{"token":"tok","price_sol":0.5,"timestamp":now()}])),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/ticks",
            Some(&ad),
            Some(json!([{"token":"tok","price_sol":0.0,"timestamp":now()}])),
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    // A price above the stop leaves the position open but raises its peak.
    let (status, body, _) = request(
        &a,
        "POST",
        "/api/admin/ticks",
        Some(&ad),
        Some(json!([{"token":"tok","price_sol":2.0,"timestamp":now()}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["recorded"], 1);
    assert!(body["closed"].as_array().unwrap().is_empty());

    // Giving back 25% from the peak trails out, with no leader event at all.
    let (_, body, _) = request(
        &a,
        "POST",
        "/api/admin/ticks",
        Some(&ad),
        Some(json!([{"token":"tok","price_sol":1.4,"timestamp":now()}])),
    )
    .await;
    let closed = body["closed"].as_array().unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["user_id"], user);
    assert!(
        closed[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Trailing stop")
    );
    let (state, reason): (String, String) = {
        let db = a.lock().unwrap();
        db.query_row(
            "SELECT status,reason FROM positions WHERE id='p'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(state, "closed");
    assert!(reason.starts_with("Trailing stop"));
    // The exit is recorded once; a repeat tick cannot close it again.
    let (_, body, _) = request(
        &a,
        "POST",
        "/api/admin/ticks",
        Some(&ad),
        Some(json!([{"token":"tok","price_sol":0.1,"timestamp":now()}])),
    )
    .await;
    assert!(body["closed"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn authenticity_reports_funding_and_refuses_uncopyable_entry_timing() {
    let a = app();
    let ad = admin(&a).await;
    // WHALE is always first into every token; QUIET follows much later.
    let mut events = vec![];
    for i in 0..4 {
        let base = 1_700_000_000 + i * 10_000;
        let token = format!("token{i}");
        events.push(json!({"id":format!("e{i}a"),"wallet":WHALE,"token":token,"symbol":"T",
            "side":"buy","quantity":100.0,"price_sol":1.0,"fee_sol":0.0,"timestamp":base,"liquidity_sol":1000.0}));
        events.push(json!({"id":format!("e{i}b"),"wallet":QUIET,"token":token,"symbol":"T",
            "side":"buy","quantity":100.0,"price_sol":1.0,"fee_sol":0.0,"timestamp":base+900,"liquidity_sol":1000.0}));
    }
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/import",
            Some(&ad),
            Some(Value::Array(events))
        )
        .await
        .1["inserted"],
        8
    );
    let (status, body, _) = request(
        &a,
        "GET",
        &format!("/api/wallets/{WHALE}/authenticity"),
        Some(&ad),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["buys"], 4);
    assert_eq!(body["early_entries"], 4);
    assert_eq!(body["early_entry_pct"], 100.0);
    assert!(body["blocked"].as_str().unwrap().contains("100%"));
    // No archived transactions: the funding graph is unknown, and says so.
    assert_eq!(body["funding_graph_observed"], false);
    assert!(
        body["flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f.as_str().unwrap().contains("unknown, not clean"))
    );

    // The later entrant is copyable on timing grounds.
    let (_, body, _) = request(
        &a,
        "GET",
        &format!("/api/wallets/{QUIET}/authenticity"),
        Some(&ad),
        None,
    )
    .await;
    assert_eq!(body["early_entry_pct"], 0.0);
    assert_eq!(body["blocked"], Value::Null);

    // Indexing is admin-only and reads only what RPC collection already archived.
    let member = member(&a).await;
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/cluster/index",
            Some(&member),
            Some(json!({ "wallet": WHALE })),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let raw = json!({"blockTime":1000,"meta":{"err":null,"innerInstructions":[]},
        "transaction":{"message":{"instructions":[{"programId":"11111111111111111111111111111111",
        "parsed":{"type":"transfer","info":{"source":QUIET,"destination":WHALE,"lamports":500000}}}]}}});
    {
        let db = a.lock().unwrap();
        db.execute(
            "INSERT INTO raw_transactions VALUES('sig',?1,?2,0)",
            params![WHALE, raw.to_string()],
        )
        .unwrap();
    }
    let (status, body, _) = request(
        &a,
        "POST",
        "/api/admin/cluster/index",
        Some(&ad),
        Some(json!({ "wallet": WHALE })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["edges"], 1);
    let (_, body, _) = request(
        &a,
        "GET",
        &format!("/api/wallets/{WHALE}/authenticity"),
        Some(&ad),
        None,
    )
    .await;
    assert_eq!(body["funders"][0], QUIET);
    assert_eq!(body["funding_graph_observed"], true);
    // QUIET funded WHALE and traded the same tokens, though only afterwards.
    assert!(
        body["funded_by_counterparty"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

fn solana_keypair() -> (String, ed25519_dalek::SigningKey) {
    let key = ed25519_dalek::SigningKey::from_bytes(&[77u8; 32]);
    (
        bs58::encode(key.verifying_key().as_bytes()).into_string(),
        key,
    )
}

#[tokio::test]
async fn wallet_sign_in_creates_an_account_with_a_managed_wallet() {
    use ed25519_dalek::Signer;
    let a = app();
    let (address, key) = solana_keypair();
    let (status, challenge, _) = request(
        &a,
        "POST",
        "/api/auth/wallet/challenge",
        None,
        Some(json!({"address":address,"chain":"solana"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let message = challenge["message"].as_str().unwrap().to_string();
    let nonce = challenge["nonce"].as_str().unwrap().to_string();
    // The prompt the user sees must say it authorizes nothing.
    assert!(message.contains("does not approve a transaction"));
    assert!(message.contains(&address));

    // A signature over a different message is rejected, and burns the nonce.
    let wrong = bs58::encode(key.sign(b"something else").to_bytes()).into_string();
    let body = json!({"address":address,"chain":"solana","nonce":nonce,"signature":wrong});
    assert_eq!(
        request(&a, "POST", "/api/auth/wallet/verify", None, Some(body))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let good = bs58::encode(key.sign(message.as_bytes()).to_bytes()).into_string();
    let replay = json!({"address":address,"chain":"solana","nonce":nonce,"signature":good});
    assert_eq!(
        request(&a, "POST", "/api/auth/wallet/verify", None, Some(replay))
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "a burned nonce must not be reusable even with a valid signature"
    );

    // A fresh challenge signed correctly signs the user in.
    let (_, challenge, _) = request(
        &a,
        "POST",
        "/api/auth/wallet/challenge",
        None,
        Some(json!({"address":address,"chain":"solana"})),
    )
    .await;
    let message = challenge["message"].as_str().unwrap().to_string();
    let signature = bs58::encode(key.sign(message.as_bytes()).to_bytes()).into_string();
    let (status, _, headers) = request(
        &a,
        "POST",
        "/api/auth/wallet/verify",
        None,
        Some(json!({"address":address,"chain":"solana","nonce":challenge["nonce"],"signature":signature})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token: String = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("session=")
        .unwrap()
        .into();

    // Signing in with a wallet still provisions a managed wallet we custody.
    let (status, funds, _) = request(&a, "GET", "/api/funds", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let managed = funds["address"].as_str().unwrap().to_string();
    assert!(rpc::valid_address(&managed));
    assert_ne!(
        managed, address,
        "the managed wallet is not the login wallet"
    );
    assert_eq!(funds["custody_enabled"], true);
    assert_eq!(funds["deposit_address"], managed);
    // The balance provider is unreachable here: report that, never a zero balance.
    assert!(funds["balance_lamports"].is_null());
    assert!(funds["balance_error"].is_string());

    // Unsupported chains and malformed addresses are refused.
    for bad in [
        json!({"address":address,"chain":"bitcoin"}),
        json!({"address":"nope","chain":"solana"}),
        json!({"address":"0xshort","chain":"ethereum"}),
    ] {
        assert_eq!(
            request(&a, "POST", "/api/auth/wallet/challenge", None, Some(bad))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn registration_provisions_a_wallet_and_commission_is_disclosed() {
    let a = app();
    let (_, created, _) = request(
        &a,
        "POST",
        "/api/auth/register",
        None,
        Some(json!({"email":"funded@test.local","password":"member-password-123"})),
    )
    .await;
    assert!(rpc::valid_address(
        created["managed_wallet"].as_str().unwrap()
    ));
    let token = login_as(&a, "funded@test.local", "member-password-123").await;
    let (status, fees, _) = request(&a, "GET", "/api/fees", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    // Default model is the performance fee, and it states its own terms.
    assert_eq!(fees["schedule"]["model"], "Performance");
    assert!(
        fees["conflict_note"]
            .as_str()
            .unwrap()
            .contains("losing period costs nothing")
    );
    assert_eq!(fees["total_charged_lamports"], 0);
    assert_eq!(fees["high_water_lamports"], 0);
    // The cost model must carry a fixed per-transaction term, not bps alone.
    assert!(fees["cost_model"]["fixed_lamports"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn a_losing_account_is_never_settled_and_a_winning_one_is_charged_once() {
    let a = app();
    let ad = admin(&a).await;
    let id: String = {
        let db = a.lock().unwrap();
        let id: String = db
            .query_row("SELECT id FROM users WHERE admin=1", [], |r| r.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO positions(id,user_id,wallet,token,symbol,quantity,cost,entry,pnl,status,reason,created,closed) VALUES('loss',?1,'w','t','T',1.0,1.0,1.0,-4.0,'closed','',1,2)",
            params![id],
        )
        .unwrap();
        id
    };
    let (status, result, _) = request(&a, "POST", "/api/admin/fees/settle", Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["accounts_charged"], 0);
    assert_eq!(result["charged_lamports"], 0);

    // Now net positive by 6 SOL: 15% of 6 SOL = 0.9 SOL.
    // The empty run above consumed this epoch, which is the intended weekly
    // cadence, so advance the clock to the next one.
    {
        let db = a.lock().unwrap();
        db.execute(
            "INSERT INTO positions(id,user_id,wallet,token,symbol,quantity,cost,entry,pnl,status,reason,created,closed) VALUES('win',?1,'w','t','T',1.0,1.0,1.0,10.0,'closed','',1,2)",
            params![id],
        )
        .unwrap();
        db.execute("UPDATE fee_state SET last_settled=0", [])
            .unwrap();
    }
    let (_, result, _) = request(&a, "POST", "/api/admin/fees/settle", Some(&ad), None).await;
    assert_eq!(result["accounts_charged"], 1);
    assert_eq!(result["charged_lamports"], 900_000_000u64);
    // Re-running must not bill the same profit twice, even once the epoch rolls.
    {
        let db = a.lock().unwrap();
        db.execute("UPDATE fee_state SET last_settled=0", [])
            .unwrap();
    }
    let (_, again, _) = request(&a, "POST", "/api/admin/fees/settle", Some(&ad), None).await;
    assert_eq!(again["charged_lamports"], 0);
    let (_, funds, _) = request(&a, "GET", "/api/funds", Some(&ad), None).await;
    assert_eq!(funds["commission_charged_lamports"], 900_000_000u64);
    assert_eq!(funds["commission_owed_lamports"], 900_000_000u64);
}

#[tokio::test]
async fn closing_an_account_releases_the_key_exactly_once() {
    let a = app();
    let member = member(&a).await;
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/account/close",
            Some(&member),
            Some(json!({"confirm":"no"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, released, _) = request(
        &a,
        "POST",
        "/api/account/close",
        Some(&member),
        Some(json!({"confirm":"CLOSE"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let secret = released["secret_key"].as_str().unwrap();
    assert_eq!(bs58::decode(secret).into_vec().unwrap().len(), 64);
    assert!(released["warning"].as_str().unwrap().contains("shown once"));
    // Sessions are revoked, so the same token cannot ask again.
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/account/close",
            Some(&member),
            Some(json!({"confirm":"CLOSE"}))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // The stored ciphertext is destroyed: custody has genuinely ended.
    let stored: String = {
        let db = a.lock().unwrap();
        db.query_row(
            "SELECT secret FROM managed_wallets WHERE released>0",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(stored.is_empty());
}

#[tokio::test]
async fn withdrawals_are_off_by_default_and_validate_before_signing() {
    let a = app();
    let member = member(&a).await;
    let (status, body, _) = request(
        &a,
        "POST",
        "/api/funds/withdraw",
        Some(&member),
        Some(json!({"destination":WHALE,"lamports":1000})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("not enabled"));
    let (status, history, _) =
        request(&a, "GET", "/api/funds/withdrawals", Some(&member), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["enabled"], false);
    assert!(history["withdrawals"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn live_ingestion_builds_real_history_and_drives_paper_positions() {
    let a = app();
    let ad = admin(&a).await;
    let cfg = discovery::Config {
        url: String::new(),
        min_sol: 0.0,
        tracked_tokens: 10,
        ingest_trades: true,
        api_key: None,
    };
    let policy = ticks::Policy::from_env();
    let observe =
        |sig: &str, side: &str, sol: f64, tokens: f64, reserve: f64| discovery::Observation {
            wallet: WHALE.into(),
            token: MINT.into(),
            side: side.into(),
            sol_amount: sol,
            token_amount: tokens,
            virtual_reserve_sol: reserve,
            signature: sig.into(),
            pool: "pump".into(),
        };
    // Build a real-shaped history: 21 profitable round trips on the curve.
    {
        let mut db = a.lock().unwrap();
        for i in 0..21 {
            let at = 1_700_000_000 + i * 100;
            discovery::ingest(
                &mut db,
                &observe(&format!("b{i}"), "buy", 1.0, 1000.0, 72.5),
                at,
                &cfg,
                &policy,
            )
            .unwrap();
            discovery::ingest(
                &mut db,
                &observe(&format!("s{i}"), "sell", 2.0, 1000.0, 72.5),
                at + 50,
                &cfg,
                &policy,
            )
            .unwrap();
        }
    }
    let (_, analysis, _) =
        request(&a, "GET", &format!("/api/wallets/{WHALE}"), Some(&ad), None).await;
    assert_eq!(analysis["matched_sells"], 21);
    assert!(analysis["score"].as_u64().unwrap() >= 65);
    // Liquidity is the real curve reserve, not the virtual one.
    // 72.5 virtual less the 30 SOL seed: real, sellable curve reserve.
    assert_eq!(analysis["history"][0]["liquidity_sol"], 42.5);
    assert!(!analysis["synthetic"].as_bool().unwrap());

    // Follow the wallet, enable paper execution, then feed one more live buy.
    request(
        &a,
        "POST",
        "/api/watches",
        Some(&ad),
        Some(json!({"wallet":WHALE})),
    )
    .await;
    request(
        &a,
        "POST",
        "/api/settings",
        Some(&ad),
        Some(json!({"enabled":true,"budget":0.01,"mode":"paper"})),
    )
    .await;
    let opened = {
        let mut db = a.lock().unwrap();
        discovery::ingest(
            &mut db,
            &observe("entry", "buy", 1.0, 1000.0, 72.5),
            1_700_010_000,
            &cfg,
            &policy,
        )
        .unwrap()
    };
    assert!(opened.stored);
    assert_eq!(opened.opened, 1, "a live event must open a paper position");

    // A live sell by the leader closes it, on real observed prices.
    let closed = {
        let mut db = a.lock().unwrap();
        discovery::ingest(
            &mut db,
            &observe("exit", "sell", 3.0, 1000.0, 72.5),
            1_700_010_100,
            &cfg,
            &policy,
        )
        .unwrap()
    };
    assert_eq!(closed.closed, 1);
    let (_, overview, _) = request(&a, "GET", "/api/overview", Some(&ad), None).await;
    let position = &overview["positions"][0];
    assert_eq!(position["status"], "closed");
    assert!(position["pnl"].as_f64().unwrap() > 0.0);
    // Real funds were never touched.
    assert_eq!(overview["mode"], "paper");
}

#[tokio::test]
async fn live_ingestion_stays_off_unless_configured() {
    let a = app();
    let cfg = discovery::Config {
        url: String::new(),
        min_sol: 0.0,
        tracked_tokens: 10,
        ingest_trades: false,
        api_key: None,
    };
    let policy = ticks::Policy::from_env();
    let o = discovery::Observation {
        wallet: WHALE.into(),
        token: MINT.into(),
        side: "buy".into(),
        sol_amount: 1.0,
        token_amount: 1000.0,
        virtual_reserve_sol: 72.5,
        signature: "sig".into(),
        pool: "pump".into(),
    };
    let r = {
        let mut db = a.lock().unwrap();
        discovery::ingest(&mut db, &o, 1_700_000_000, &cfg, &policy).unwrap()
    };
    assert!(!r.stored);
    let count: i64 = {
        let db = a.lock().unwrap();
        db.query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count, 0, "a passive collector must not write history");
}

#[tokio::test]
async fn exporting_a_key_is_a_copy_and_closing_is_a_handover() {
    let a = app();
    let member = member(&a).await;
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/funds/export-key",
            Some(&member),
            Some(json!({"confirm":"yes"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, exported, _) = request(
        &a,
        "POST",
        "/api/funds/export-key",
        Some(&member),
        Some(json!({"confirm":"EXPORT"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        bs58::decode(exported["secret_key"].as_str().unwrap())
            .into_vec()
            .unwrap()
            .len(),
        64
    );
    // The critical disclosure: an export is not an exit.
    assert_eq!(exported["custody_ended"], false);
    assert!(
        exported["warning"]
            .as_str()
            .unwrap()
            .contains("still holds this key")
    );
    // Exporting does not revoke the session or the wallet, unlike closing.
    let (_, funds, _) = request(&a, "GET", "/api/funds", Some(&member), None).await;
    assert_eq!(funds["address"], exported["address"]);

    // The audit trail records the act, never the key.
    let ad = admin(&a).await;
    let (_, panel, _) = request(&a, "GET", "/api/admin", Some(&ad), None).await;
    let logs = panel["logs"].as_array().unwrap();
    let entry = logs
        .iter()
        .find(|l| l["action"] == "wallet.exported")
        .expect("export is audited");
    assert_eq!(entry["detail"], exported["address"]);
    let secret = exported["secret_key"].as_str().unwrap();
    assert!(
        !logs.iter().any(|l| l.to_string().contains(secret)),
        "a private key must never reach the audit log"
    );

    // Closing still ends custody, which export deliberately does not.
    let (_, closed, _) = request(
        &a,
        "POST",
        "/api/account/close",
        Some(&member),
        Some(json!({"confirm":"CLOSE"})),
    )
    .await;
    assert_eq!(closed["secret_key"].as_str().unwrap(), secret);
    let stored: String = {
        let db = a.lock().unwrap();
        db.query_row("SELECT secret FROM managed_wallets", [], |r| r.get(0))
            .unwrap()
    };
    assert!(stored.is_empty(), "closing destroys our copy");
}

#[tokio::test]
async fn an_unconfigured_fee_account_is_reported_rather_than_silently_collecting_nothing() {
    let a = app();
    let member = member(&a).await;
    assert_eq!(
        request(&a, "GET", "/api/admin/fees/verify", Some(&member), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let ad = admin(&a).await;
    let (status, body, _) = request(&a, "GET", "/api/admin/fees/verify", Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["configured"], false);
    assert_eq!(body["collecting"], false);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("every quote reports 0 bps")
    );
    assert!(body["how_to_fix"].as_str().unwrap().contains("wrapped-SOL"));
    // And the quote path agrees rather than claiming a fee it cannot receive.
    let (_, fees, _) = request(&a, "GET", "/api/fees", Some(&ad), None).await;
    assert_eq!(fees["schedule"]["model"], "Performance");
}

#[tokio::test]
async fn switching_copy_trading_off_closes_every_path_not_just_the_interface() {
    let a = app();
    let ad = admin(&a).await;
    let events: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    let wallet = events[0]["wallet"].as_str().unwrap().to_string();
    request(&a, "POST", "/api/admin/import", Some(&ad), Some(events)).await;
    request(
        &a,
        "POST",
        "/api/watches",
        Some(&ad),
        Some(json!({ "wallet": wallet })),
    )
    .await;
    request(
        &a,
        "POST",
        "/api/settings",
        Some(&ad),
        Some(json!({"enabled":true,"budget":0.1,"mode":"paper"})),
    )
    .await;
    assert!(
        request(&a, "POST", "/api/paper/replay", Some(&ad), None)
            .await
            .1["opened"]
            .as_u64()
            .unwrap()
            > 0
    );

    // Only an administrator may flip it.
    let member = member(&a).await;
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/admin/platform",
            Some(&member),
            Some(json!({"copy_trading":false}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, body, _) = request(
        &a,
        "POST",
        "/api/admin/platform",
        Some(&ad),
        Some(json!({"copy_trading":false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["note"]
            .as_str()
            .unwrap()
            .contains("Swapping is unaffected")
    );

    // Every entry point refuses, and the flag is visible to the interface.
    assert_eq!(
        request(&a, "POST", "/api/paper/replay", Some(&ad), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &a,
            "POST",
            "/api/settings",
            Some(&ad),
            Some(json!({"enabled":true,"budget":0.1,"mode":"paper"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&a, "GET", "/api/me", Some(&ad), None).await.1["copy_trading"],
        false
    );
    // Accounts are actually stood down, not left armed for a silent restart.
    let still_enabled: i64 = {
        let db = a.lock().unwrap();
        db.query_row("SELECT COUNT(*) FROM users WHERE enabled=1", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    assert_eq!(still_enabled, 0);

    // The live collector reaches fanout without a route, so it must be closed too.
    let fresh: Value =
        serde_json::from_str(include_str!("../../fixtures/demo-trades.json")).unwrap();
    let mut event = fresh[0].clone();
    event["id"] = json!("switch-test-event");
    event["timestamp"] = json!(now());
    let (_, result, _) = request(&a, "POST", "/api/admin/events", Some(&ad), Some(event)).await;
    assert_eq!(result["opened"], 0);
    assert_eq!(result["users_evaluated"], 0);

    // Swapping is genuinely unaffected by the switch.
    assert_ne!(
        request(
            &a,
            "POST",
            "/api/swap/quote",
            Some(&ad),
            Some(json!({"input_mint":"So11111111111111111111111111111111111111112","output_mint":"So11111111111111111111111111111111111111112","amount":1,"slippage_bps":50}))
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "the swap path must not be gated by the copy-trading switch"
    );

    // And it can be turned back on.
    let (_, back, _) = request(
        &a,
        "POST",
        "/api/admin/platform",
        Some(&ad),
        Some(json!({"copy_trading":true})),
    )
    .await;
    assert!(
        back["note"]
            .as_str()
            .unwrap()
            .contains("opt in individually")
    );
    assert_eq!(
        request(&a, "GET", "/api/me", Some(&ad), None).await.1["copy_trading"],
        true
    );
}

#[tokio::test]
async fn a_deployer_spamming_one_name_collapses_to_one_row_with_the_count() {
    // Observed live: twelve identical "YO / LAST COIN 3.3 MILL" mints from a
    // single address inside forty seconds. They are distinct mints, so nothing
    // was duplicated — but listing all twelve buries every other launch.
    let a = app();
    let ad = admin(&a).await;
    {
        let db = a.lock().unwrap();
        let add = |mint: &str, symbol: &str, name: &str, creator: Option<&str>, seen: i64| {
            db.execute(
                "INSERT INTO tokens(mint,name,symbol,creator,first_seen) VALUES(?1,?2,?3,?4,?5)",
                params![mint, name, symbol, creator, seen],
            )
            .unwrap();
        };
        for i in 0..12 {
            add(
                &format!("spam{i}"),
                "YO",
                "LAST COIN",
                Some("deployer-a"),
                100 + i,
            );
        }
        // Same name, a different deployer: a separate coin, not the same batch.
        add("other", "YO", "LAST COIN", Some("deployer-b"), 150);
        // No creator recorded, so it groups only with itself.
        add("lone1", "HELLO", "HI", None, 160);
        add("lone2", "HELLO", "HI", None, 161);
    }
    let (status, body, _) = request(&a, "GET", "/api/tokens", Some(&ad), None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["tokens"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "twelve spam mints collapse to one");

    let batch: Vec<&Value> = rows.iter().filter(|r| r["copies"] == 12).collect();
    assert_eq!(batch.len(), 1);
    // The newest of the batch represents it, so the row is still current.
    assert_eq!(batch[0]["mint"], "spam11");

    // A shared name alone never merges two deployers.
    assert_eq!(
        rows.iter().filter(|r| r["mint"] == "other").count(),
        1,
        "a different deployer stays its own row"
    );
    assert!(
        rows.iter()
            .filter(|r| r["symbol"] == "HELLO")
            .all(|r| r["copies"] == 1),
        "an unknown creator groups only with itself"
    );
}

#[tokio::test]
async fn every_search_branch_returns_every_key() {
    // A pasted address once came back without `handles`, and the interface
    // called `.map` on nothing and blanked the whole page. The browser test
    // did not catch it because its mock supplied the key the server omitted.
    let a = app();
    let ad = admin(&a).await;
    let keys = ["exact", "tokens", "wallets", "handles", "note"];
    for q in [
        "So11111111111111111111111111111111111111112", // an address
        WHALE,                                         // an address that has traded
        "abc",                                         // free text
    ] {
        let (status, body, _) =
            request(&a, "GET", &format!("/api/search?q={q}"), Some(&ad), None).await;
        assert_eq!(status, StatusCode::OK, "{q}");
        for key in keys {
            assert!(body.get(key).is_some(), "{q} is missing {key}");
        }
        for key in ["tokens", "wallets", "handles"] {
            assert!(body[key].is_array(), "{q}: {key} must be an array");
        }
    }
}
