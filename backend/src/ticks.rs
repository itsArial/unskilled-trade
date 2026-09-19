//! Independent price ticks and tick-evaluated exits.
//!
//! Without this the only way a simulated position could close was a *leader*
//! event on the same token. A leader who stops trading, or who exits somewhere we
//! never observe, left the follower holding indefinitely. That is not an
//! auto-stop, so exits are evaluated here against an independent price path.
//!
//! A tick is an observation, not a quote. It records that someone traded at this
//! price, which is the best evidence available without a pool-state feed; it is
//! not a guarantee that this position could have been sold at that price, and no
//! price impact for our own size is modeled.
use rusqlite::{Connection, params};
use serde::Serialize;

pub struct Policy {
    pub trailing_pct: f64,
    pub hard_stop_pct: f64,
    pub time_stop_seconds: i64,
    /// Fraction of the way to migration at which a curve position is closed.
    pub graduation_exit_progress: f64,
    /// Network cost, fixed plus proportional. Exits priced here use the same
    /// model as leader-driven exits so the two remain comparable.
    pub cost: crate::fees::CostModel,
    pub schedule: crate::fees::Schedule,
}
impl Policy {
    pub fn from_env() -> Self {
        let number = |key: &str, fallback: f64| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| v.is_finite() && *v > 0.0)
                .unwrap_or(fallback)
        };
        Self {
            trailing_pct: number("EXIT_TRAILING_PCT", 25.0),
            hard_stop_pct: number("EXIT_HARD_STOP_PCT", 20.0),
            time_stop_seconds: number("EXIT_TIME_STOP_SECONDS", 86400.0) as i64,
            graduation_exit_progress: number("EXIT_GRADUATION_PROGRESS", 0.9).clamp(0.01, 1.0),
            cost: crate::fees::CostModel::from_env(),
            schedule: crate::fees::Schedule::from_env(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Closed {
    pub id: String,
    pub user_id: String,
    pub reason: String,
    pub exit: f64,
    pub pnl: f64,
}

/// Exit decision for one position at one observed price.
///
/// The hard stop is measured from the entry reference and the trailing stop from
/// the best price actually observed while holding, so a position that ran up and
/// gave it back exits on the give-back rather than waiting for the entry stop.
/// `progress` is how far the token's bonding curve is toward migration, where
/// that is observable, and `None` for a venue that does not report it.
pub fn decide(
    entry: f64,
    peak: f64,
    price: f64,
    age: i64,
    progress: Option<f64>,
    p: &Policy,
) -> Option<String> {
    if !(entry.is_finite() && peak.is_finite() && price.is_finite()) || entry <= 0.0 {
        return None;
    }
    if price <= entry * (1.0 - p.hard_stop_pct / 100.0) {
        return Some(format!(
            "Hard stop: {:.1}% below entry reference",
            p.hard_stop_pct
        ));
    }
    // Marginal price is continuous across migration but *depth* is not: virtual
    // reserves vanish and the pool is backed only by real ones. Holding
    // inventory constant, selling before graduation always yields more SOL than
    // selling after. That is an algebraic consequence of the constant-product
    // invariant, not a fitted signal, so it is acted on rather than weighed.
    if let Some(g) = progress.filter(|g| g.is_finite())
        && g >= p.graduation_exit_progress
    {
        return Some(format!(
            "Migration imminent: curve is {:.0}% to graduation, where pool depth drops discontinuously",
            g * 100.0
        ));
    }
    if peak > entry && price <= peak * (1.0 - p.trailing_pct / 100.0) {
        return Some(format!(
            "Trailing stop: {:.1}% below the best observed price while held",
            p.trailing_pct
        ));
    }
    if age >= p.time_stop_seconds {
        return Some(format!(
            "Time stop: held for {} seconds without an exit signal",
            p.time_stop_seconds
        ));
    }
    None
}

pub fn record(
    db: &Connection,
    token: &str,
    price: f64,
    at: i64,
    source: &str,
) -> Result<(), rusqlite::Error> {
    db.execute(
        "INSERT OR IGNORE INTO price_ticks VALUES(?1,?2,?3,?4)",
        params![token, at, price, source],
    )?;
    Ok(())
}

/// Applies one observed price to every open position in that token, across all
/// users. Runs inside the caller's transaction so a tick either updates every
/// affected position or none of them.
pub fn apply(
    db: &Connection,
    token: &str,
    price: f64,
    at: i64,
    progress: Option<f64>,
    p: &Policy,
) -> Result<Vec<Closed>, rusqlite::Error> {
    if !price.is_finite() || price <= 0.0 {
        return Ok(vec![]);
    }
    // A tick never rewrites history: positions opened after it are not affected.
    db.execute(
        "UPDATE positions SET peak=MAX(COALESCE(peak,entry),?1) WHERE token=?2 AND status='open' AND created<=?3",
        params![price, token, at],
    )?;
    let open = {
        let mut stmt = db.prepare("SELECT id,user_id,quantity,cost,entry,COALESCE(peak,entry),created FROM positions WHERE token=?1 AND status='open' AND created<=?2")?;
        stmt.query_map(params![token, at], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut closed = vec![];
    for (id, user_id, quantity, cost, entry, peak, created) in open {
        let Some(reason) = decide(entry, peak, price, at - created, progress, p) else {
            continue;
        };
        let gross = quantity * price;
        let pnl = gross - p.cost.charge(gross) - cost;
        // A per-trade commission, where the operator has chosen one, is billed
        // on the exit as well as the entry.
        crate::fees::charge_trade(db, &user_id, &format!("{id}:exit"), gross, at, &p.schedule)?;
        db.execute(
            "UPDATE positions SET status='closed',exit=?1,pnl=?2,reason=?3,closed=?4 WHERE id=?5 AND status='open'",
            params![price, pnl, reason, at, id],
        )?;
        closed.push(Closed {
            id,
            user_id,
            reason,
            exit: price,
            pnl,
        });
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    fn policy() -> Policy {
        Policy {
            trailing_pct: 25.0,
            hard_stop_pct: 20.0,
            time_stop_seconds: 86400,
            graduation_exit_progress: 0.9,
            cost: crate::fees::CostModel {
                fixed_lamports: 10_000,
                bps: 200.0,
                failure_rate: 0.0,
            },
            schedule: crate::fees::Schedule::from_env(),
        }
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE positions(id TEXT PRIMARY KEY,user_id TEXT,wallet TEXT,token TEXT,symbol TEXT,quantity REAL,cost REAL,entry REAL,exit REAL,pnl REAL,status TEXT,reason TEXT,created INTEGER,closed INTEGER,peak REAL);",
        )
        .unwrap();
        db.execute_batch(crate::TICK_SCHEMA).unwrap();
        db.execute_batch(crate::FEE_SCHEMA).unwrap();
        db
    }
    fn position(db: &Connection, id: &str, entry: f64, created: i64) {
        db.execute("INSERT INTO positions(id,user_id,wallet,token,symbol,quantity,cost,entry,status,reason,created) VALUES(?1,'u','w','tok','TOK',10.0,1.0,?2,'open','',?3)",
            params![id, entry, created]).unwrap();
    }
    #[test]
    fn hard_stop_measures_from_entry() {
        let p = policy();
        assert!(decide(1.0, 1.0, 0.81, 0, None, &p).is_none());
        assert!(
            decide(1.0, 1.0, 0.80, 0, None, &p)
                .unwrap()
                .starts_with("Hard stop")
        );
    }
    #[test]
    fn trailing_stop_measures_from_the_best_observed_price() {
        let p = policy();
        // Ran to 2.0 then fell to 1.5: still above entry, but 25% off the peak.
        assert!(
            decide(1.0, 2.0, 1.50, 0, None, &p)
                .unwrap()
                .starts_with("Trailing stop")
        );
        assert!(decide(1.0, 2.0, 1.51, 0, None, &p).is_none());
        // Never ran up: the trailing stop must not fire off the entry price.
        assert!(decide(1.0, 1.0, 0.90, 0, None, &p).is_none());
    }
    #[test]
    fn time_stop_fires_without_any_price_move() {
        let p = policy();
        assert!(decide(1.0, 1.0, 1.0, 86399, None, &p).is_none());
        assert!(
            decide(1.0, 1.0, 1.0, 86400, None, &p)
                .unwrap()
                .starts_with("Time stop")
        );
    }
    #[test]
    fn a_curve_approaching_migration_is_exited_regardless_of_price() {
        let p = policy();
        // Depth collapses at migration, so proximity alone is a reason to leave
        // even while the position is comfortably in profit.
        assert!(
            decide(1.0, 1.0, 1.5, 0, Some(0.95), &p)
                .unwrap()
                .starts_with("Migration imminent")
        );
        assert_eq!(decide(1.0, 1.0, 1.5, 0, Some(0.5), &p), None);
        // Exactly at the threshold counts.
        assert!(decide(1.0, 1.0, 1.5, 0, Some(0.9), &p).is_some());
        // A venue that does not report curve state must not be assumed safe or
        // assumed imminent: unknown simply does not trigger this rule.
        assert_eq!(decide(1.0, 1.0, 1.5, 0, None, &p), None);
        assert_eq!(decide(1.0, 1.0, 1.5, 0, Some(f64::NAN), &p), None);
    }
    #[test]
    fn the_hard_stop_still_wins_over_migration_proximity() {
        let p = policy();
        // Both apply; the loss-limiting reason is the one reported.
        assert!(
            decide(1.0, 1.0, 0.5, 0, Some(0.99), &p)
                .unwrap()
                .starts_with("Hard stop")
        );
    }
    #[test]
    fn migration_proximity_closes_open_positions_through_apply() {
        let p = policy();
        let db = db();
        position(&db, "a", 1.0, 100);
        assert!(
            apply(&db, "tok", 1.2, 200, Some(0.4), &p)
                .unwrap()
                .is_empty()
        );
        let closed = apply(&db, "tok", 1.2, 300, Some(0.92), &p).unwrap();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].reason.contains("graduation"));
        assert!(
            closed[0].pnl > 0.0,
            "exited in profit, before the depth drop"
        );
    }
    #[test]
    fn nonsense_prices_never_close_a_position() {
        let p = policy();
        assert!(decide(1.0, 1.0, f64::NAN, 0, None, &p).is_none());
        assert!(decide(0.0, 0.0, 0.0, 0, None, &p).is_none());
        let db = db();
        position(&db, "a", 1.0, 100);
        assert_eq!(apply(&db, "tok", f64::NAN, 200, None, &p).unwrap(), vec![]);
        assert_eq!(apply(&db, "tok", -1.0, 200, None, &p).unwrap(), vec![]);
    }
    #[test]
    fn a_tick_closes_every_affected_position_and_only_once() {
        let p = policy();
        let db = db();
        position(&db, "a", 1.0, 100);
        position(&db, "b", 1.0, 100);
        // Opened after the tick: untouched, because a tick never rewrites history.
        position(&db, "later", 1.0, 900);
        let closed = apply(&db, "tok", 0.5, 200, None, &p).unwrap();
        assert_eq!(closed.len(), 2);
        assert!(closed[0].reason.starts_with("Hard stop"));
        // 10 tokens at 0.5, 2% cost, minus the 1.0 SOL cost basis.
        assert!((closed[0].pnl - 3.89999).abs() < 1e-4);
        assert!(apply(&db, "tok", 0.5, 300, None, &p).unwrap().is_empty());
        let still_open: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM positions WHERE status='open'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_open, 1);
    }
    #[test]
    fn peak_accumulates_across_ticks_so_a_later_fall_trails_out() {
        let p = policy();
        let db = db();
        position(&db, "a", 1.0, 100);
        assert!(apply(&db, "tok", 2.0, 200, None, &p).unwrap().is_empty());
        let peak: Option<f64> = db
            .query_row("SELECT peak FROM positions WHERE id='a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(peak, Some(2.0));
        // 1.6 is above entry but 20% under the peak: not yet a trailing exit.
        assert!(apply(&db, "tok", 1.6, 300, None, &p).unwrap().is_empty());
        let closed = apply(&db, "tok", 1.4, 400, None, &p).unwrap();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].reason.starts_with("Trailing stop"));
    }
    #[test]
    fn ticks_are_deduplicated_per_token_and_time() {
        let db = db();
        record(&db, "tok", 1.0, 100, "test").unwrap();
        record(&db, "tok", 9.0, 100, "test").unwrap();
        let (count, price): (i64, f64) = db
            .query_row(
                "SELECT COUNT(*),MIN(price_sol) FROM price_ticks WHERE token='tok'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((count, price), (1, 1.0));
        assert!(
            db.query_row("SELECT source FROM price_ticks", [], |r| r
                .get::<_, String>(0))
                .optional()
                .unwrap()
                .is_some()
        );
    }
}
