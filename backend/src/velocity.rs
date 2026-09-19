//! Liquidity velocity and carrying capacity.
//!
//! # Velocity (EVIDENCE C5)
//!
//! The published finding this implements: for any fixed level of SOL in a
//! curve, tokens that reach it in *fewer trades* graduate substantially more
//! often, and that dominated every other variable in the study. The operational
//! form is the cumulative number of observed swaps at the moment a liquidity
//! threshold is first crossed — fewer swaps to the same depth means larger,
//! more decisive buying rather than a crowd of dust.
//!
//! This is computed from our own observations, so it is a **lower bound on
//! trade count**: swaps we never saw make a token look faster than it was. A
//! token whose history we joined late is therefore reported with its coverage,
//! not silently flattered.
//!
//! # Capacity (EVIDENCE C9/D5)
//!
//! Every other gate asks whether an edge exists. None asked how much money it
//! can carry. A 5 SOL position needs a 5,000 SOL pool under the 0.1% cap, and
//! pools that deep are past the phase where these returns were made. Capacity
//! reports the order sizes the policy would actually have allowed, what that
//! costs as a share, and how it divides across a follower base.
use crate::analytics::Trade;
use serde::Serialize;
use std::collections::BTreeMap;

/// Liquidity levels, in SOL, at which velocity is measured. The last is the
/// Pump.fun graduation threshold.
pub const THRESHOLDS: [f64; 4] = [10.0, 25.0, 50.0, 85.0];

#[derive(Debug, Serialize, PartialEq)]
pub struct Crossing {
    pub threshold_sol: f64,
    /// Observed swaps up to and including the one that first crossed it.
    pub trades_to_cross: usize,
    pub seconds_to_cross: i64,
}

#[derive(Debug, Serialize)]
pub struct Velocity {
    pub token: String,
    pub observed_trades: usize,
    pub first_seen: Option<i64>,
    pub peak_liquidity_sol: f64,
    pub crossings: Vec<Crossing>,
    pub flags: Vec<String>,
}

/// Velocity for one token, from the trades we hold.
///
/// Trades carrying unknown liquidity (`0`) still count toward the trade total —
/// they happened — but cannot themselves cross a threshold, because an unknown
/// depth is not evidence of reaching one.
pub fn velocity(token: &str, all: &[Trade]) -> Velocity {
    let mut history: Vec<&Trade> = all.iter().filter(|t| t.token == token).collect();
    history.sort_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)));
    let first_seen = history.first().map(|t| t.timestamp);
    let peak = history.iter().fold(0.0_f64, |m, t| m.max(t.liquidity_sol));
    let mut crossings = vec![];
    for threshold in THRESHOLDS {
        if let Some((index, trade)) = history
            .iter()
            .enumerate()
            .find(|(_, t)| t.liquidity_sol >= threshold)
        {
            crossings.push(Crossing {
                threshold_sol: threshold,
                trades_to_cross: index + 1,
                seconds_to_cross: trade.timestamp - first_seen.unwrap_or(trade.timestamp),
            });
        }
    }
    let mut flags = vec![
        "Trade counts are from observations this service holds, so they are a lower bound: swaps we never saw make a token look faster than it was.".to_string(),
    ];
    if history.iter().any(|t| t.liquidity_sol <= 0.0) {
        flags.push("Some observations carry unknown liquidity and cannot cross a threshold".into());
    }
    if crossings.is_empty() && peak > 0.0 {
        flags.push(format!(
            "Never reached the lowest measured level; peak observed depth was {peak:.1} SOL"
        ));
    }
    Velocity {
        token: token.into(),
        observed_trades: history.len(),
        first_seen,
        peak_liquidity_sol: peak,
        crossings,
        flags,
    }
}

/// Median trades-to-cross across the tokens a wallet bought, per threshold.
///
/// Answers "what velocity regime does this wallet buy into" — a wallet that
/// only ever enters tokens which took hundreds of swaps to fill is playing a
/// different game from one that enters the fast ones.
pub fn wallet_regime(wallet: &str, all: &[Trade]) -> BTreeMap<String, usize> {
    let tokens: std::collections::BTreeSet<&str> = all
        .iter()
        .filter(|t| t.wallet == wallet && t.side == "buy")
        .map(|t| t.token.as_str())
        .collect();
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for token in tokens {
        for crossing in velocity(token, all).crossings {
            buckets
                .entry(format!("{:.0}", crossing.threshold_sol))
                .or_default()
                .push(crossing.trades_to_cross);
        }
    }
    buckets
        .into_iter()
        .map(|(k, mut v)| {
            v.sort_unstable();
            (k, v[v.len() / 2])
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct Capacity {
    pub order_sol: f64,
    /// Entries the policy would have allowed at this size.
    pub eligible_entries: usize,
    /// Of those, how many were capped below the requested size by liquidity.
    pub capped_by_liquidity: usize,
    pub median_allowed_sol: Option<f64>,
    pub smallest_allowed_sol: Option<f64>,
    /// Entries whose allowed size fell under the point where fixed costs
    /// dominate, i.e. where the trade cannot pay for itself.
    pub below_cost_floor: usize,
    /// Largest simultaneous exposure one follower would have carried.
    pub peak_concurrent_sol: f64,
    /// That exposure divided across a follower base, at the same per-order size.
    pub per_follower_sol: BTreeMap<String, f64>,
    pub flags: Vec<String>,
}

/// What the strategy can actually carry at a given order size.
///
/// `min_liquidity` and the 0.1% cap mirror the paper policy, so this reports
/// the sizes that policy would really have used rather than an idealised one.
pub fn capacity(
    wallet: &str,
    all: &[Trade],
    order_sol: f64,
    min_liquidity: f64,
    cost: &crate::fees::CostModel,
    followers: &[usize],
) -> Capacity {
    let mut history: Vec<&Trade> = all.iter().filter(|t| t.wallet == wallet).collect();
    history.sort_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)));
    let floor = cost.fixed_dominates_below_sol();
    let (mut allowed, mut capped, mut below) = (vec![], 0usize, 0usize);
    let (mut open, mut peak) = (BTreeMap::<&str, f64>::new(), 0.0_f64);
    for t in &history {
        if t.side == "sell" {
            open.remove(t.token.as_str());
            continue;
        }
        if t.liquidity_sol < min_liquidity {
            continue;
        }
        let cap = t.liquidity_sol * 0.001;
        let size = order_sol.min(cap);
        if cap < order_sol {
            capped += 1;
        }
        if floor.is_some_and(|f| size < f) {
            below += 1;
        }
        allowed.push(size);
        open.insert(t.token.as_str(), size);
        peak = peak.max(open.values().sum::<f64>());
    }
    allowed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut flags = vec![];
    if !allowed.is_empty() && capped * 2 > allowed.len() {
        flags.push(format!(
            "Liquidity capped the order on {capped} of {} entries: this wallet's edge does not scale to {order_sol} SOL",
            allowed.len()
        ));
    }
    if below > 0 {
        flags.push(format!(
            "{below} entries fall below {:.4} SOL, where the fixed per-transaction cost outweighs the proportional one and the trade cannot pay for itself",
            floor.unwrap_or(0.0)
        ));
    }
    if allowed.is_empty() {
        flags.push("No entry in this history met the liquidity minimum at any size".into());
    }
    Capacity {
        order_sol,
        eligible_entries: allowed.len(),
        capped_by_liquidity: capped,
        median_allowed_sol: allowed.get(allowed.len() / 2).copied(),
        smallest_allowed_sol: allowed.first().copied(),
        below_cost_floor: below,
        peak_concurrent_sol: peak,
        per_follower_sol: followers
            .iter()
            .map(|n| (n.to_string(), peak * *n as f64))
            .collect(),
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(id: &str, wallet: &str, token: &str, side: &str, time: i64, liq: f64) -> Trade {
        Trade {
            id: id.into(),
            wallet: wallet.into(),
            token: token.into(),
            symbol: "T".into(),
            side: side.into(),
            quantity: 100.0,
            price_sol: 0.01,
            fee_sol: 0.0,
            timestamp: time,
            liquidity_sol: liq,
            routed: None,
        }
    }
    fn free() -> crate::fees::CostModel {
        crate::fees::CostModel {
            fixed_lamports: 0,
            bps: 0.0,
            failure_rate: 0.0,
        }
    }

    #[test]
    fn velocity_counts_swaps_to_each_depth() {
        // Three trades to reach 25 SOL, five to reach 50.
        let all = vec![
            t("1", "a", "x", "buy", 100, 5.0),
            t("2", "b", "x", "buy", 110, 12.0),
            t("3", "c", "x", "buy", 120, 30.0),
            t("4", "d", "x", "buy", 130, 40.0),
            t("5", "e", "x", "buy", 140, 60.0),
        ];
        let v = velocity("x", &all);
        assert_eq!(v.observed_trades, 5);
        assert_eq!(v.peak_liquidity_sol, 60.0);
        let by = |th: f64| v.crossings.iter().find(|c| c.threshold_sol == th).unwrap();
        assert_eq!(by(10.0).trades_to_cross, 2);
        assert_eq!(by(25.0).trades_to_cross, 3);
        assert_eq!(by(25.0).seconds_to_cross, 20);
        assert_eq!(by(50.0).trades_to_cross, 5);
        // 85 was never reached, so it is absent rather than reported as slow.
        assert!(v.crossings.iter().all(|c| c.threshold_sol != 85.0));
    }
    #[test]
    fn a_faster_token_shows_fewer_trades_to_the_same_depth() {
        let slow: Vec<Trade> = (0..20)
            .map(|i| {
                t(
                    &format!("s{i}"),
                    "w",
                    "slow",
                    "buy",
                    100 + i,
                    1.0 + i as f64 * 1.6,
                )
            })
            .collect();
        let fast = vec![
            t("f1", "w", "fast", "buy", 100, 14.0),
            t("f2", "w", "fast", "buy", 101, 30.0),
        ];
        let at = |v: &Velocity, th: f64| {
            v.crossings
                .iter()
                .find(|c| c.threshold_sol == th)
                .map(|c| c.trades_to_cross)
        };
        assert_eq!(at(&velocity("fast", &fast), 25.0), Some(2));
        assert!(at(&velocity("slow", &slow), 25.0).unwrap() > 2);
    }
    #[test]
    fn unknown_liquidity_cannot_cross_a_threshold() {
        let all = vec![
            t("1", "a", "x", "buy", 100, 0.0),
            t("2", "b", "x", "buy", 110, 0.0),
        ];
        let v = velocity("x", &all);
        assert!(v.crossings.is_empty());
        assert_eq!(v.observed_trades, 2, "they still happened");
        assert!(v.flags.iter().any(|f| f.contains("unknown liquidity")));
    }
    #[test]
    fn velocity_always_states_that_it_is_a_lower_bound() {
        let v = velocity("x", &[t("1", "a", "x", "buy", 100, 90.0)]);
        assert!(v.flags.iter().any(|f| f.contains("lower bound")));
        assert_eq!(v.crossings.len(), 4, "one trade crossed every level");
    }
    #[test]
    fn a_wallets_regime_is_the_median_of_what_it_buys() {
        let all = vec![
            // fast token: 1 trade to 25
            t("1", "w", "fast", "buy", 100, 30.0),
            // slow token: 3 trades to 25
            t("2", "o", "slow", "buy", 100, 5.0),
            t("3", "o", "slow", "buy", 101, 15.0),
            t("4", "w", "slow", "buy", 102, 30.0),
        ];
        let r = wallet_regime("w", &all);
        assert_eq!(r.get("25"), Some(&3), "median of [1,3] takes the upper");
        assert!(r.contains_key("10"));
    }
    #[test]
    fn capacity_reports_where_liquidity_caps_the_order() {
        // Thin pools: 0.1% of 200 SOL is 0.2, well under a 1 SOL request.
        let all: Vec<Trade> = (0..5)
            .map(|i| {
                t(
                    &format!("b{i}"),
                    "w",
                    &format!("t{i}"),
                    "buy",
                    100 + i,
                    200.0,
                )
            })
            .collect();
        let c = capacity("w", &all, 1.0, 100.0, &free(), &[1, 100]);
        assert_eq!(c.eligible_entries, 5);
        assert_eq!(c.capped_by_liquidity, 5);
        assert_eq!(c.median_allowed_sol, Some(0.2));
        assert!(c.flags.iter().any(|f| f.contains("does not scale")));
        // Five concurrent 0.2 SOL positions, and what that means for a crowd.
        assert!((c.peak_concurrent_sol - 1.0).abs() < 1e-9);
        assert_eq!(c.per_follower_sol.get("100"), Some(&100.0));
    }
    #[test]
    fn exposure_falls_when_positions_close() {
        let all = vec![
            t("1", "w", "a", "buy", 100, 100000.0),
            t("2", "w", "b", "buy", 101, 100000.0),
            t("3", "w", "a", "sell", 102, 100000.0),
            t("4", "w", "c", "buy", 103, 100000.0),
        ];
        let c = capacity("w", &all, 1.0, 100.0, &free(), &[1]);
        // Peak is two concurrent, not three cumulative.
        assert!((c.peak_concurrent_sol - 2.0).abs() < 1e-9);
        assert_eq!(c.capped_by_liquidity, 0);
    }
    #[test]
    fn orders_under_the_cost_floor_are_counted() {
        let cost = crate::fees::CostModel {
            fixed_lamports: 10_000_000, // crossover at 10 SOL against 10 bps
            bps: 10.0,
            failure_rate: 0.0,
        };
        let all = vec![t("1", "w", "x", "buy", 100, 200.0)];
        let c = capacity("w", &all, 1.0, 100.0, &cost, &[1]);
        assert_eq!(c.below_cost_floor, 1);
        assert!(c.flags.iter().any(|f| f.contains("cannot pay for itself")));
    }
    #[test]
    fn a_history_that_never_qualifies_says_so() {
        let all = vec![t("1", "w", "x", "buy", 100, 5.0)];
        let c = capacity("w", &all, 1.0, 100.0, &free(), &[1]);
        assert_eq!(c.eligible_entries, 0);
        assert_eq!(c.median_allowed_sol, None);
        assert!(
            c.flags
                .iter()
                .any(|f| f.contains("met the liquidity minimum"))
        );
    }
}
