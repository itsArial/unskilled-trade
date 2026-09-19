//! X (Twitter) identity for a token, and the reuse signals that come with it.
//!
//! The feature traders actually use is not the timeline — it is knowing whether
//! the account promoting a token is *new*, or a recycled one wearing a new name.
//! A handle that has fronted five tokens, or an account that was called
//! something else last week, is the single cheapest scam tell available.
//!
//! # Cost
//!
//! X removed its free tier in February 2026; reads are metered. This module is
//! therefore split:
//!
//! - **Free, always on.** Handle reuse across mints, derived from our own token
//!   table. No third-party call, and the dataset improves the longer we run.
//! - **Metered, optional.** Resolving a handle to its immutable numeric user id,
//!   which is what makes *rename* detection possible, plus account age and
//!   follower count. Off unless `X_API_BEARER` is set.
//!
//! A handle is mutable and an id is not. Without the id we can prove that one
//! handle fronted many tokens; we cannot prove that one account wore many
//! handles. That distinction is kept honest everywhere below: an unresolved
//! account reports *unknown*, never "clean".
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

/// Extracts the account handle from whatever a token's metadata claims.
///
/// Creators put anything in this field: a bare handle, an @handle, a profile
/// URL, a link to one post, or a community. Only a real profile yields a
/// handle; a community or status-only link has no owning account we can judge,
/// and guessing one would attach a reputation to the wrong party.
pub fn handle(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() || text.len() > 300 {
        return None;
    }
    let lowered = text.to_lowercase();
    let path = ["https://", "http://"]
        .iter()
        .find_map(|p| lowered.strip_prefix(p))
        .map(|rest| rest.trim_start_matches("www."))
        .and_then(|rest| {
            [
                "x.com/",
                "twitter.com/",
                "mobile.twitter.com/",
                "vxtwitter.com/",
            ]
            .iter()
            .find_map(|host| rest.strip_prefix(*host))
        });
    let candidate = match path {
        Some(rest) => rest.split(['/', '?', '#']).next()?,
        None => lowered.trim_start_matches('@'),
    };
    // Reserved paths are not accounts.
    if matches!(
        candidate,
        "i" | "intent" | "share" | "home" | "search" | "hashtag" | "explore" | "messages" | ""
    ) {
        return None;
    }
    (candidate.len() <= 15
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then(|| candidate.to_string())
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Reputation {
    pub handle: String,
    /// Mints in our own record that have pointed at this handle.
    pub tokens_promoted: usize,
    /// Earlier mints, so a trader can see what this account fronted before.
    pub previous_mints: Vec<String>,
    /// Immutable account id, when it has been resolved.
    pub user_id: Option<String>,
    /// Handles this same account has previously worn. Requires `user_id`.
    pub previous_handles: Vec<String>,
    pub followers: Option<i64>,
    /// Unix seconds. A very young account behind a token is a warning.
    pub account_created: Option<i64>,
    /// False when the account has never been resolved to an id, which makes
    /// rename detection impossible for it. Unknown, not clean.
    pub identity_resolved: bool,
    pub flags: Vec<String>,
}

/// Records that a mint claims this handle. Returns how many distinct mints have.
pub fn link(db: &Connection, mint: &str, handle: &str, at: i64) -> Result<(), rusqlite::Error> {
    db.execute(
        "INSERT OR IGNORE INTO token_socials VALUES(?1,?2,?3)",
        params![mint, handle, at],
    )?;
    Ok(())
}

/// Records a resolved identity and notices a rename.
///
/// The handle history is keyed by id, so a second handle appearing under an id
/// we have seen before *is* the rename signal.
pub fn record_identity(
    db: &Connection,
    user_id: &str,
    handle: &str,
    followers: Option<i64>,
    created: Option<i64>,
    at: i64,
) -> Result<(), rusqlite::Error> {
    db.execute("INSERT INTO x_accounts(user_id,handle,followers,account_created,first_seen,last_seen) VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT(user_id) DO UPDATE SET handle=excluded.handle,followers=excluded.followers,last_seen=excluded.last_seen",
        params![user_id, handle, followers, created, at])?;
    db.execute(
        "INSERT OR IGNORE INTO x_handle_history VALUES(?1,?2,?3)",
        params![user_id, handle, at],
    )?;
    Ok(())
}

pub fn reputation(
    db: &Connection,
    handle: &str,
    mint: &str,
) -> Result<Reputation, rusqlite::Error> {
    let previous_mints: Vec<String> = {
        let mut stmt = db.prepare(
            "SELECT mint FROM token_socials WHERE handle=?1 AND mint<>?2 ORDER BY first_seen DESC LIMIT 20",
        )?;
        stmt.query_map(params![handle, mint], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    let identity = db
        .query_row(
            "SELECT user_id,followers,account_created FROM x_accounts WHERE handle=?1",
            [handle],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?;
    let (user_id, followers, account_created) = match identity {
        Some((id, f, c)) => (Some(id), f, c),
        None => (None, None, None),
    };
    let previous_handles: Vec<String> = match &user_id {
        Some(id) => {
            let mut stmt = db.prepare(
                "SELECT handle FROM x_handle_history WHERE user_id=?1 AND handle<>?2 ORDER BY first_seen DESC LIMIT 20",
            )?;
            stmt.query_map(params![id, handle], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        }
        None => vec![],
    };
    let tokens_promoted = previous_mints.len() + 1;
    let mut flags = vec![];
    if !previous_mints.is_empty() {
        flags.push(format!(
            "This account has fronted {} other token{} we have seen",
            previous_mints.len(),
            if previous_mints.len() == 1 { "" } else { "s" }
        ));
    }
    if !previous_handles.is_empty() {
        flags.push(format!(
            "Renamed: this account previously went by {}",
            previous_handles.join(", ")
        ));
    }
    if user_id.is_none() {
        flags.push(
            "Account identity has not been resolved, so a rename could not be checked. Unknown, not clean.".into(),
        );
    }
    if let Some(created) = account_created
        && at_age_days(created, crate::now()) < 30.0
    {
        flags.push(format!(
            "Account is {:.0} days old",
            at_age_days(created, crate::now())
        ));
    }
    if followers.is_some_and(|f| f < 100) {
        flags.push("Fewer than 100 followers".into());
    }
    Ok(Reputation {
        handle: handle.to_string(),
        tokens_promoted,
        identity_resolved: user_id.is_some(),
        previous_mints,
        user_id,
        previous_handles,
        followers,
        account_created,
        flags,
    })
}

fn at_age_days(created: i64, now: i64) -> f64 {
    (now - created).max(0) as f64 / 86400.0
}

/// Resolves a handle to its immutable id. Metered, so it is called once per
/// account and cached, never per page view.
pub async fn resolve(
    bearer: &str,
    handle: &str,
) -> Result<(String, Option<i64>, Option<i64>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|_| "Cannot create X client")?;
    let response = client
        .get(format!(
            "https://api.x.com/2/users/by/username/{handle}?user.fields=created_at,public_metrics"
        ))
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|_| "X request failed".to_string())?;
    if response.status() == 429 {
        return Err("X rate limit reached".into());
    }
    if !response.status().is_success() {
        return Err(format!("X returned HTTP {}", response.status().as_u16()));
    }
    let v: Value = response.json().await.map_err(|_| "Unreadable X response")?;
    let data = &v["data"];
    let id = data["id"]
        .as_str()
        .ok_or("X did not return an account id")?;
    let followers = data["public_metrics"]["followers_count"].as_i64();
    let created = data["created_at"].as_str().and_then(parse_iso8601);
    Ok((id.to_string(), followers, created))
}

/// Minimal RFC3339 parser for the one timestamp shape X returns.
fn parse_iso8601(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil epoch, Howard Hinnant's algorithm.
    let y_adj = if mo <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(crate::SOCIAL_SCHEMA).unwrap();
        db
    }

    #[test]
    fn handles_are_extracted_from_every_shape_creators_use() {
        for (input, expected) in [
            ("https://x.com/somecoin", Some("somecoin")),
            ("https://twitter.com/SomeCoin", Some("somecoin")),
            ("http://www.x.com/somecoin/", Some("somecoin")),
            ("https://x.com/somecoin/status/1234567890", Some("somecoin")),
            ("https://mobile.twitter.com/somecoin?s=21", Some("somecoin")),
            ("@somecoin", Some("somecoin")),
            ("somecoin", Some("somecoin")),
            ("  @Some_Coin  ", Some("some_coin")),
        ] {
            assert_eq!(handle(input).as_deref(), expected, "input: {input}");
        }
    }
    #[test]
    fn things_that_are_not_an_account_yield_nothing() {
        for input in [
            "",
            "   ",
            "https://x.com/i/communities/1234",
            "https://x.com/search?q=coin",
            "https://t.me/somegroup",
            "https://example.com/somecoin",
            "waaaaaaaaaaaaaaytoolongforahandle",
            "bad-handle-with-dashes",
        ] {
            assert_eq!(handle(input), None, "input: {input}");
        }
    }
    #[test]
    fn a_fresh_handle_is_reported_as_unresolved_not_clean() {
        let db = db();
        link(&db, "mint1", "somecoin", 100).unwrap();
        let r = reputation(&db, "somecoin", "mint1").unwrap();
        assert_eq!(r.tokens_promoted, 1);
        assert!(r.previous_mints.is_empty());
        assert_eq!(r.user_id, None);
        assert!(!r.identity_resolved, "an unresolved account must say so");
        assert!(r.flags.iter().any(|f| f.contains("Unknown, not clean")));
    }
    #[test]
    fn reuse_across_mints_is_detected_with_no_third_party_call() {
        let db = db();
        link(&db, "mint1", "somecoin", 100).unwrap();
        link(&db, "mint2", "somecoin", 200).unwrap();
        link(&db, "mint3", "somecoin", 300).unwrap();
        let r = reputation(&db, "somecoin", "mint3").unwrap();
        assert_eq!(r.tokens_promoted, 3);
        assert_eq!(r.previous_mints, vec!["mint2".to_string(), "mint1".into()]);
        assert!(r.flags.iter().any(|f| f.contains("fronted 2 other tokens")));
        // Linking the same pair twice must not inflate the count.
        link(&db, "mint3", "somecoin", 400).unwrap();
        assert_eq!(
            reputation(&db, "somecoin", "mint3")
                .unwrap()
                .tokens_promoted,
            3
        );
    }
    #[test]
    fn a_rename_is_detected_only_through_the_immutable_id() {
        let db = db();
        record_identity(&db, "99", "oldname", Some(5000), None, 100).unwrap();
        record_identity(&db, "99", "newname", Some(5200), None, 200).unwrap();
        link(&db, "mint1", "newname", 200).unwrap();
        let r = reputation(&db, "newname", "mint1").unwrap();
        assert_eq!(r.user_id.as_deref(), Some("99"));
        assert!(r.identity_resolved);
        assert_eq!(r.previous_handles, vec!["oldname".to_string()]);
        assert!(
            r.flags
                .iter()
                .any(|f| f.contains("previously went by oldname"))
        );
        assert_eq!(r.followers, Some(5200));
    }
    #[test]
    fn a_small_or_young_account_is_called_out() {
        let db = db();
        let recent = crate::now() - 3 * 86400;
        record_identity(&db, "42", "brandnew", Some(7), Some(recent), 100).unwrap();
        link(&db, "mint1", "brandnew", 100).unwrap();
        let r = reputation(&db, "brandnew", "mint1").unwrap();
        assert!(r.flags.iter().any(|f| f.contains("days old")));
        assert!(
            r.flags
                .iter()
                .any(|f| f.contains("Fewer than 100 followers"))
        );
    }
    #[test]
    fn x_timestamps_parse_to_unix_seconds() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso8601("2024-03-01T12:30:45.000Z"), Some(1709296245));
        assert_eq!(parse_iso8601("not a date"), None);
        assert_eq!(parse_iso8601("2024-13-01T00:00:00.000Z"), None);
    }
}
