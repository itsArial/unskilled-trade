//! Commission model and the trading-cost function both it and the simulator use.
//!
//! # Why the default is a performance fee, not a per-trade cut
//!
//! A per-trade commission makes platform revenue a function of *turnover*. The
//! evidence in `docs/EVIDENCE.md` says turnover is exactly where followers lose:
//! the only clean out-of-sample test of copying top wallets landed above baseline
//! but **below breakeven** (§2), and IOSCO FR/06/2025 specifically flags
//! remuneration structures that pay the intermediary regardless of copier outcome
//! (§8). Charging per trade would mean earning most from the users doing worst,
//! and would push an already sub-breakeven strategy further under.
//!
//! A performance fee inverts that. Revenue exists only when the user is ahead of
//! their own previous peak, which makes the platform's incentive to reduce churn
//! rather than manufacture it.
//!
//! `PerTrade` and `Hybrid` are implemented because the operator may choose them,
//! but they are not the default and the API labels their conflict explicitly.
//!
//! # Arithmetic
//!
//! Fees are integer lamports. `f64` is used for research metrics elsewhere in
//! this codebase and is documented as wrong for money; this module is where
//! money is actually owed, so it does not inherit that debt.
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

pub const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;

/// Expected network cost of one side of one trade.
///
/// The shape matters more than the level. Network cost is mostly **fixed** per
/// transaction — base fee plus the priority fee needed to land — and is charged
/// on failures too. A purely proportional model understates cost badly at the
/// small order sizes the liquidity cap produces, and the error grows as orders
/// shrink.
#[derive(Debug, Clone, Serialize)]
pub struct CostModel {
    pub fixed_lamports: u64,
    pub bps: f64,
    /// Expected failed attempts per successful trade. Failures pay the fixed
    /// term and nothing else.
    pub failure_rate: f64,
}
impl CostModel {
    pub fn from_env() -> Self {
        Self {
            fixed_lamports: env_number("COPY_COST_FIXED_LAMPORTS", 150_000.0) as u64,
            bps: env_number("COPY_COST_BPS", 200.0),
            failure_rate: env_number("COPY_FAILURE_RATE", 0.0).clamp(0.0, 10.0),
        }
    }
    /// Cost in SOL of transacting `notional_sol` on one side.
    pub fn charge(&self, notional_sol: f64) -> f64 {
        if !notional_sol.is_finite() || notional_sol < 0.0 {
            return 0.0;
        }
        let fixed = self.fixed_lamports as f64 / LAMPORTS_PER_SOL * (1.0 + self.failure_rate);
        fixed + notional_sol * self.bps / 10_000.0
    }
    /// Notional at which the fixed term alone equals `bps` of the order, i.e.
    /// below this size the per-transaction cost dominates everything else.
    pub fn fixed_dominates_below_sol(&self) -> Option<f64> {
        (self.bps > 0.0).then(|| {
            self.fixed_lamports as f64 / LAMPORTS_PER_SOL * (1.0 + self.failure_rate) * 10_000.0
                / self.bps
        })
    }
}

fn env_number(key: &str, fallback: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &f64| v.is_finite() && *v >= 0.0)
        .unwrap_or(fallback)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Model {
    /// Charged only on new profit above the account's own previous peak.
    Performance,
    /// Charged on every entry and exit regardless of outcome.
    PerTrade,
    /// Both.
    Hybrid,
    /// No commission is taken.
    None,
}
impl Model {
    fn parse(s: &str) -> Self {
        match s {
            "per_trade" => Self::PerTrade,
            "hybrid" => Self::Hybrid,
            "none" => Self::None,
            _ => Self::Performance,
        }
    }
    pub fn charges_per_trade(&self) -> bool {
        matches!(self, Self::PerTrade | Self::Hybrid)
    }
    pub fn charges_performance(&self) -> bool {
        matches!(self, Self::Performance | Self::Hybrid)
    }
    /// Plain statement of whose interest the model serves, shown to users.
    pub fn conflict_note(&self) -> &'static str {
        match self {
            Self::Performance => {
                "Commission is charged only on profit above your account's previous peak. A losing period costs nothing, and a recovery is not charged twice."
            }
            Self::PerTrade => {
                "Commission is charged on every trade whether it wins or loses. This pays the platform more when you trade more, which is a conflict with your interest; published evidence associates higher turnover with worse copier outcomes."
            }
            Self::Hybrid => {
                "A per-trade commission is charged on every trade regardless of outcome, in addition to a performance fee. The per-trade component pays the platform more when you trade more, which is a conflict with your interest."
            }
            Self::None => "No commission is charged.",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub model: Model,
    /// Share of new profit above the high-water mark, in basis points.
    pub performance_bps: u64,
    pub trade_bps: u64,
    pub trade_fixed_lamports: u64,
    pub epoch_seconds: i64,
    /// Accruals below this are carried forward rather than written, so dust does
    /// not cost more in fees than it collects.
    pub min_settlement_lamports: u64,
}
impl Schedule {
    pub fn from_env() -> Self {
        Self {
            model: Model::parse(&std::env::var("FEE_MODEL").unwrap_or_default()),
            performance_bps: env_number("FEE_PERFORMANCE_BPS", 1500.0).min(5000.0) as u64,
            trade_bps: env_number("FEE_TRADE_BPS", 0.0).min(500.0) as u64,
            trade_fixed_lamports: env_number("FEE_TRADE_FIXED_LAMPORTS", 0.0) as u64,
            epoch_seconds: env_number("FEE_EPOCH_SECONDS", 604800.0) as i64,
            min_settlement_lamports: env_number("FEE_MIN_SETTLEMENT_LAMPORTS", 1_000_000.0) as u64,
        }
    }
    /// Commission on one executed trade of `notional_sol`, in lamports.
    pub fn trade_fee_lamports(&self, notional_sol: f64) -> u64 {
        if !self.model.charges_per_trade() || !notional_sol.is_finite() || notional_sol <= 0.0 {
            return 0;
        }
        let proportional = notional_sol * self.trade_bps as f64 / 10_000.0 * LAMPORTS_PER_SOL;
        self.trade_fixed_lamports + proportional.max(0.0).round() as u64
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Settlement {
    pub charged_lamports: u64,
    /// Cumulative realized profit, in lamports, at settlement time.
    pub cumulative_lamports: i64,
    pub high_water_lamports: i64,
    pub new_profit_lamports: i64,
    pub reason: String,
}

fn to_lamports(sol: f64) -> i64 {
    if !sol.is_finite() {
        return 0;
    }
    (sol * LAMPORTS_PER_SOL).round() as i64
}

/// Realized profit is netted across **all** closed positions, not per trade.
/// Charging winners while ignoring losers would take a cut from an account that
/// is shrinking.
fn cumulative_realized(db: &Connection, user_id: &str) -> Result<i64, rusqlite::Error> {
    let sol: f64 = db.query_row(
        "SELECT COALESCE(SUM(pnl),0) FROM positions WHERE user_id=?1 AND status='closed'",
        [user_id],
        |r| r.get(0),
    )?;
    Ok(to_lamports(sol))
}

/// Settles the performance fee for one account.
///
/// Idempotent per epoch: the ledger reference is derived from the epoch index, so
/// repeating a settlement inside the same epoch writes nothing. The high-water
/// mark only ever rises, so a drawdown and recovery is charged once, not twice.
pub fn settle(
    db: &Connection,
    user_id: &str,
    at: i64,
    schedule: &Schedule,
) -> Result<Settlement, rusqlite::Error> {
    let cumulative = cumulative_realized(db, user_id)?;
    let (high_water, last_settled) = db
        .query_row(
            "SELECT high_water_lamports,last_settled FROM fee_state WHERE user_id=?1",
            [user_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
        .unwrap_or((0, 0));
    let new_profit = cumulative - high_water;
    let mut settlement = Settlement {
        charged_lamports: 0,
        cumulative_lamports: cumulative,
        high_water_lamports: high_water,
        new_profit_lamports: new_profit,
        reason: String::new(),
    };
    if !schedule.model.charges_performance() {
        settlement.reason = "Performance fees are not enabled".into();
        return Ok(settlement);
    }
    if at - last_settled < schedule.epoch_seconds {
        settlement.reason = "Settlement epoch has not elapsed".into();
        return Ok(settlement);
    }
    if new_profit <= 0 {
        // The account is at or below its own peak. Nothing is owed, and the mark
        // is not lowered, so the next recovery is not charged twice.
        db.execute("INSERT INTO fee_state VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET last_settled=excluded.last_settled", params![user_id, high_water, at])?;
        settlement.reason = "No new profit above the high-water mark".into();
        return Ok(settlement);
    }
    let charge = (new_profit as f64 * schedule.performance_bps as f64 / 10_000.0).round() as u64;
    if charge < schedule.min_settlement_lamports {
        settlement.reason =
            "Accrual is below the minimum settlement amount and is carried forward".into();
        return Ok(settlement);
    }
    let epoch = at / schedule.epoch_seconds.max(1);
    let reference = format!("performance:{user_id}:{epoch}");
    if db.execute(
        "INSERT OR IGNORE INTO fee_ledger(id,user_id,kind,lamports,basis_lamports,reference,created) VALUES(?1,?2,'performance',?3,?4,?5,?6)",
        params![uuid::Uuid::new_v4().to_string(), user_id, charge as i64, new_profit, reference, at],
    )? == 0
    {
        settlement.reason = "This epoch has already been settled".into();
        return Ok(settlement);
    }
    db.execute("INSERT INTO fee_state VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET high_water_lamports=excluded.high_water_lamports,last_settled=excluded.last_settled", params![user_id, cumulative, at])?;
    settlement.charged_lamports = charge;
    settlement.high_water_lamports = cumulative;
    settlement.reason = format!(
        "{}% of {} SOL in new profit above the previous peak",
        schedule.performance_bps as f64 / 100.0,
        new_profit as f64 / LAMPORTS_PER_SOL
    );
    Ok(settlement)
}

/// Records a per-trade commission. `reference` makes it idempotent per position
/// and side, so a replayed event cannot bill twice.
pub fn charge_trade(
    db: &Connection,
    user_id: &str,
    reference: &str,
    notional_sol: f64,
    at: i64,
    schedule: &Schedule,
) -> Result<u64, rusqlite::Error> {
    let lamports = schedule.trade_fee_lamports(notional_sol);
    if lamports == 0 {
        return Ok(0);
    }
    if db.execute(
        "INSERT OR IGNORE INTO fee_ledger(id,user_id,kind,lamports,basis_lamports,reference,created) VALUES(?1,?2,'trade',?3,?4,?5,?6)",
        params![uuid::Uuid::new_v4().to_string(), user_id, lamports as i64, to_lamports(notional_sol), reference, at],
    )? == 0
    {
        return Ok(0);
    }
    Ok(lamports)
}

pub fn total_charged(db: &Connection, user_id: &str) -> Result<i64, rusqlite::Error> {
    db.query_row(
        "SELECT COALESCE(SUM(lamports),0) FROM fee_ledger WHERE user_id=?1",
        [user_id],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn schedule(model: Model) -> Schedule {
        Schedule {
            model,
            performance_bps: 2000,
            trade_bps: 50,
            trade_fixed_lamports: 0,
            epoch_seconds: 100,
            min_settlement_lamports: 0,
        }
    }
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE positions(id TEXT PRIMARY KEY,user_id TEXT,pnl REAL,status TEXT);",
        )
        .unwrap();
        db.execute_batch(crate::FEE_SCHEMA).unwrap();
        db
    }
    fn close(db: &Connection, id: &str, pnl: f64) {
        db.execute(
            "INSERT INTO positions VALUES(?1,'u',?2,'closed')",
            params![id, pnl],
        )
        .unwrap();
    }

    #[test]
    fn fixed_cost_dominates_at_small_order_sizes() {
        let c = CostModel {
            fixed_lamports: 150_000,
            bps: 200.0,
            failure_rate: 0.0,
        };
        // 0.1 SOL order: 0.00015 fixed + 0.002 proportional.
        assert!((c.charge(0.1) - 0.00215).abs() < 1e-9);
        // The fixed term equals 200bps at 0.0075 SOL, so below that it dominates.
        assert!((c.fixed_dominates_below_sol().unwrap() - 0.0075).abs() < 1e-9);
        assert_eq!(c.charge(-1.0), 0.0);
        assert_eq!(c.charge(f64::NAN), 0.0);
    }
    #[test]
    fn failures_add_the_fixed_term_only() {
        let base = CostModel {
            fixed_lamports: 100_000,
            bps: 100.0,
            failure_rate: 0.0,
        };
        let flaky = CostModel {
            failure_rate: 1.0,
            ..base.clone()
        };
        // One expected retry doubles the fixed component and nothing else.
        assert!((flaky.charge(1.0) - (base.charge(1.0) + 0.0001)).abs() < 1e-12);
    }
    #[test]
    fn a_losing_account_is_never_charged() {
        let db = db();
        close(&db, "a", -5.0);
        let s = settle(&db, "u", 1000, &schedule(Model::Performance)).unwrap();
        assert_eq!(s.charged_lamports, 0);
        assert!(s.reason.contains("No new profit"));
        assert_eq!(total_charged(&db, "u").unwrap(), 0);
    }
    #[test]
    fn profit_is_netted_against_losses_before_any_cut() {
        let db = db();
        close(&db, "win", 10.0);
        close(&db, "loss", -9.0);
        let s = settle(&db, "u", 1000, &schedule(Model::Performance)).unwrap();
        // 20% of 1 SOL net, not 20% of the 10 SOL winner.
        assert_eq!(s.charged_lamports, 200_000_000);
    }
    #[test]
    fn a_drawdown_and_recovery_is_charged_once() {
        let db = db();
        let sch = schedule(Model::Performance);
        close(&db, "a", 10.0);
        assert_eq!(
            settle(&db, "u", 1000, &sch).unwrap().charged_lamports,
            2_000_000_000
        );
        // Give it all back.
        close(&db, "b", -10.0);
        let down = settle(&db, "u", 2000, &sch).unwrap();
        assert_eq!(down.charged_lamports, 0);
        assert_eq!(down.high_water_lamports, 10_000_000_000);
        // Earn it back: still under the previous peak, so nothing is owed.
        close(&db, "c", 10.0);
        assert_eq!(settle(&db, "u", 3000, &sch).unwrap().charged_lamports, 0);
        // Only genuinely new profit is charged.
        close(&db, "d", 4.0);
        assert_eq!(
            settle(&db, "u", 4000, &sch).unwrap().charged_lamports,
            800_000_000
        );
        assert_eq!(total_charged(&db, "u").unwrap(), 2_800_000_000);
    }
    #[test]
    fn settlement_is_idempotent_within_an_epoch() {
        let db = db();
        let sch = schedule(Model::Performance);
        close(&db, "a", 10.0);
        assert_eq!(
            settle(&db, "u", 1000, &sch).unwrap().charged_lamports,
            2_000_000_000
        );
        // Same epoch: refused on the elapsed check.
        let again = settle(&db, "u", 1050, &sch).unwrap();
        assert_eq!(again.charged_lamports, 0);
        assert!(again.reason.contains("epoch has not elapsed"));
        assert_eq!(total_charged(&db, "u").unwrap(), 2_000_000_000);
    }
    #[test]
    fn dust_accruals_are_carried_forward_not_written() {
        let db = db();
        let mut sch = schedule(Model::Performance);
        sch.min_settlement_lamports = 1_000_000;
        close(&db, "a", 0.001); // 20% of 0.001 SOL = 200,000 lamports
        let s = settle(&db, "u", 1000, &sch).unwrap();
        assert_eq!(s.charged_lamports, 0);
        assert!(s.reason.contains("below the minimum"));
        // The mark did not move, so the profit is still collectable later.
        close(&db, "b", 1.0);
        assert!(settle(&db, "u", 2000, &sch).unwrap().charged_lamports > 0);
    }
    #[test]
    fn per_trade_fees_apply_only_in_the_models_that_declare_them() {
        assert_eq!(schedule(Model::Performance).trade_fee_lamports(1.0), 0);
        assert_eq!(schedule(Model::None).trade_fee_lamports(1.0), 0);
        // 50 bps of 1 SOL.
        assert_eq!(schedule(Model::PerTrade).trade_fee_lamports(1.0), 5_000_000);
        assert_eq!(schedule(Model::Hybrid).trade_fee_lamports(1.0), 5_000_000);
        assert_eq!(schedule(Model::PerTrade).trade_fee_lamports(0.0), 0);
        assert_eq!(schedule(Model::PerTrade).trade_fee_lamports(f64::NAN), 0);
    }
    #[test]
    fn a_trade_is_billed_once_per_reference() {
        let db = db();
        let sch = schedule(Model::PerTrade);
        assert_eq!(
            charge_trade(&db, "u", "pos1:entry", 1.0, 10, &sch).unwrap(),
            5_000_000
        );
        assert_eq!(
            charge_trade(&db, "u", "pos1:entry", 1.0, 10, &sch).unwrap(),
            0
        );
        assert_eq!(
            charge_trade(&db, "u", "pos1:exit", 1.0, 20, &sch).unwrap(),
            5_000_000
        );
        assert_eq!(total_charged(&db, "u").unwrap(), 10_000_000);
    }
    #[test]
    fn every_model_states_its_own_conflict() {
        assert!(
            Model::PerTrade
                .conflict_note()
                .contains("conflict with your interest")
        );
        assert!(
            Model::Hybrid
                .conflict_note()
                .contains("conflict with your interest")
        );
        assert!(
            Model::Performance
                .conflict_note()
                .contains("losing period costs nothing")
        );
        assert_eq!(Model::parse("per_trade"), Model::PerTrade);
        assert_eq!(Model::parse("anything-else"), Model::Performance);
    }
}
