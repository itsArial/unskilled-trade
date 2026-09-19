use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub wallet: String,
    pub token: String,
    pub symbol: String,
    pub side: String,
    pub quantity: f64,
    pub price_sol: f64,
    pub fee_sol: f64,
    pub timestamp: i64,
    pub liquidity_sol: f64,
    /// `Some(true)` when the venue was reached through an aggregator,
    /// `Some(false)` when the signer called it directly, `None` when the source
    /// does not report it. Absent is *not observed*, never "was direct".
    #[serde(default)]
    pub routed: Option<bool>,
}
impl Trade {
    pub fn validate(&self) -> bool {
        // The id is a composite key, not an address. A real Solana signature is
        // 88 characters and an address 44, so a natural
        // `source:signature:wallet` id runs past 128 — a limit that silently
        // rejected every genuine mainnet observation until live data exposed it.
        !self.id.is_empty()
            && self.id.len() <= 256
            && [&self.wallet, &self.token, &self.symbol]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 128)
            && [
                self.quantity,
                self.price_sol,
                self.fee_sol,
                self.liquidity_sol,
            ]
            .iter()
            .all(|n| n.is_finite())
            && self.quantity * self.price_sol <= 1e12
            && self.fee_sol <= 1e9
            && self.quantity <= 1e24
            && self.quantity >= 1e-9
            && self.quantity > 0.0
            && self.price_sol >= 1e-18
            && self.fee_sol >= 0.0
            && self.liquidity_sol >= 0.0
            && self.timestamp > 0
            && matches!(self.side.as_str(), "buy" | "sell")
    }
}
#[derive(Debug, Serialize)]
pub struct Analysis {
    pub wallet: String,
    pub trades: usize,
    pub matched_sells: usize,
    pub win_rate: Option<f64>,
    /// Completed acquisition lots. One buy scaled out over several sells is one
    /// round trip, not several observations.
    /// Trades whose route was observed, and the share of those that went
    /// through an aggregator. Published work treats direct program invocation
    /// versus routing as a proxy for bot-attributed flow; a wallet that is
    /// almost entirely routed is more likely automated than discretionary.
    /// Both are `None` when no trade in the history recorded a route.
    pub routed_observed: usize,
    pub routed_share_pct: Option<f64>,
    pub round_trips: usize,
    /// Profitable round trips as a percentage. Prefer this to `win_rate`:
    /// counting sell events rewards scaling out of winners and dumping losers
    /// in one piece, which inflates the sell-event figure systematically.
    pub round_trip_win_rate: Option<f64>,
    pub win_rate_interval_95: Option<(f64, f64)>,
    pub expectancy_sol: Option<f64>,
    pub largest_win_share: Option<f64>,
    pub max_loss_streak: usize,
    pub synthetic: bool,
    pub realized_pnl_sol: f64,
    pub profit_factor: Option<f64>,
    pub max_drawdown_sol: f64,
    pub median_hold_seconds: Option<i64>,
    pub unmatched_quantity: f64,
    pub score: u32,
    pub flags: Vec<String>,
    pub curve: Vec<(i64, f64)>,
    pub history: Vec<Trade>,
}
// FIFO cost basis includes buy fees and proportional sell fees. Unknown inventory
// is excluded from profit, never treated as free acquisition.
pub fn analyze(wallet: &str, input: &[Trade]) -> Analysis {
    let mut history: Vec<Trade> = input
        .iter()
        .filter(|t| t.wallet == wallet)
        .cloned()
        .collect();
    history.sort_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)));
    // Lots carry the profit already realized on their sold portion, so a lot
    // scaled out over several sells still resolves to one round trip.
    let mut lots: BTreeMap<String, VecDeque<(f64, f64, i64, f64)>> = BTreeMap::new();
    let (mut round_trips, mut round_trip_wins) = (0usize, 0usize);
    let (mut pnl, mut gains, mut losses, mut peak, mut dd, mut unmatched) =
        (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    let (mut sells, mut wins) = (0, 0);
    let mut largest_win = 0.0_f64;
    let mut loss_streak = 0;
    let mut max_loss_streak = 0;
    let mut holds = vec![];
    let mut curve = vec![];
    for t in &history {
        let queue = lots.entry(t.token.clone()).or_default();
        if t.side == "buy" {
            queue.push_back((
                t.quantity,
                t.price_sol + t.fee_sol / t.quantity,
                t.timestamp,
                0.0,
            ));
            continue;
        }
        let mut remaining = t.quantity;
        let mut result = 0.0;
        let mut matched = 0.0;
        while remaining > 1e-9 {
            let Some((qty, cost, time, accrued)) = queue.pop_front() else {
                break;
            };
            let take = remaining.min(qty);
            let part = take * (t.price_sol - cost) - t.fee_sol * take / t.quantity;
            result += part;
            matched += take;
            remaining -= take;
            holds.push(t.timestamp - time);
            if qty - take > 1e-9 {
                queue.push_front((qty - take, cost, time, accrued + part));
            } else {
                // The lot is exhausted: one acquisition has completed its round
                // trip, whether it was sold in one piece or five.
                round_trips += 1;
                if accrued + part > 0.0 {
                    round_trip_wins += 1;
                }
            }
        }
        unmatched += remaining;
        if matched > 0.0 {
            sells += 1;
            if result > 0.0 {
                wins += 1;
                gains += result;
                largest_win = largest_win.max(result);
                loss_streak = 0;
            } else {
                losses -= result;
                loss_streak += 1;
                max_loss_streak = max_loss_streak.max(loss_streak);
            }
            pnl += result;
            peak = peak.max(pnl);
            dd = dd.max(peak - pnl);
            curve.push((t.timestamp, pnl));
        }
    }
    holds.sort();
    let wr = if sells > 0 {
        Some(wins as f64 / sells as f64 * 100.0)
    } else {
        None
    };
    let interval = if sells > 0 {
        let n = sells as f64;
        let p = wins as f64 / n;
        let z = 1.96_f64;
        let den = 1.0 + z * z / n;
        let center = (p + z * z / (2.0 * n)) / den;
        let margin = z * ((p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt()) / den;
        Some(((center - margin) * 100.0, (center + margin) * 100.0))
    } else {
        None
    };
    let route_observed = history.iter().filter(|t| t.routed.is_some()).count();
    let route_routed = history.iter().filter(|t| t.routed == Some(true)).count();
    let synthetic = history.iter().any(|t| t.id.starts_with("demo-"));
    let mut flags = vec!["Historical observations; score is not a probability of profit".into()];
    if synthetic {
        flags.push("Synthetic demonstration history; not actual wallet performance".into());
    }
    if history.iter().any(|t| t.id.starts_with("rpc:")) {
        flags.push("Partial RPC coverage: estimated native SOL cash flow, unknown liquidity; routed trades excluded".into());
    }
    if gains > 0.0 && largest_win / gains > 0.5 {
        flags.push("More than half of gross gains came from one sell event".into());
    }
    if max_loss_streak >= 4 {
        flags.push("At least four consecutive losing matched sells".into());
    }
    if let Some(rt) = (round_trips > 0).then(|| round_trip_wins as f64 / round_trips as f64 * 100.0)
        && wr.is_some_and(|w| w - rt > 10.0)
    {
        flags.push(format!(
            "Sell-event win rate {:.0}% overstates the round-trip rate of {rt:.0}%: this wallet scales out of winners",
            wr.unwrap_or(0.0)
        ));
    }
    if route_observed > 0 {
        let share = route_routed as f64 / route_observed as f64 * 100.0;
        if share >= 90.0 {
            flags.push(format!(
                "{share:.0}% of observed trades were routed through an aggregator, which is consistent with automated flow"
            ));
        }
    }
    if sells < 20 {
        flags.push("Small sample: fewer than 20 matched sells".into());
    }
    if unmatched > 1e-9 {
        flags.push("Incomplete cost basis: unmatched sells excluded from P&L".into());
    }
    if history.iter().any(|t| t.liquidity_sol < 100.0) {
        flags.push("Thin or unknown liquidity in supplied history".into());
    }
    if holds.first().is_some_and(|x| *x < 10) {
        flags.push("Very short holds may be impossible to copy after latency".into());
    }
    // The score's win-rate input is the round-trip rate, not the sell-event
    // rate: the latter is mechanically inflated by scaling out of winners, and
    // it carried 45% of the weight. The weights themselves are unchanged and
    // remain uncalibrated — they must be fitted and held out before they move,
    // so substituting a less biased input is the only correction made here.
    let scoring_win_rate = (round_trips > 0)
        .then(|| round_trip_wins as f64 / round_trips as f64 * 100.0)
        .or(wr);
    let score = (((scoring_win_rate.unwrap_or(0.0) * 0.45)
        + (sells.min(40) as f64 / 40.0 * 30.0)
        + if pnl > 0.0 { 25.0 } else { 0.0 })
        * if unmatched > 1e-9 { 0.6 } else { 1.0 }) as u32;
    Analysis {
        wallet: wallet.into(),
        trades: history.len(),
        matched_sells: sells,
        routed_observed: route_observed,
        routed_share_pct: (route_observed > 0)
            .then(|| route_routed as f64 / route_observed as f64 * 100.0),
        win_rate: wr,
        round_trips,
        round_trip_win_rate: (round_trips > 0)
            .then(|| round_trip_wins as f64 / round_trips as f64 * 100.0),
        win_rate_interval_95: interval,
        expectancy_sol: if sells > 0 {
            Some(pnl / sells as f64)
        } else {
            None
        },
        largest_win_share: if gains > 0.0 {
            Some(largest_win / gains * 100.0)
        } else {
            None
        },
        max_loss_streak,
        synthetic,
        realized_pnl_sol: pnl,
        profit_factor: if losses > 0.0 {
            Some(gains / losses)
        } else {
            None
        },
        max_drawdown_sol: dd,
        median_hold_seconds: holds.get(holds.len() / 2).copied(),
        unmatched_quantity: unmatched,
        score,
        flags,
        curve,
        history,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(id: &str, side: &str, q: f64, p: f64, fee: f64, time: i64) -> Trade {
        Trade {
            id: id.into(),
            wallet: "w".into(),
            token: "t".into(),
            symbol: "T".into(),
            side: side.into(),
            quantity: q,
            price_sol: p,
            fee_sol: fee,
            timestamp: time,
            liquidity_sol: 1000.0,
            routed: None,
        }
    }
    #[test]
    fn fifo_partial_sell_accounts_for_fees() {
        let a = analyze(
            "w",
            &[
                t("1", "buy", 10.0, 2.0, 1.0, 1),
                t("2", "buy", 10.0, 4.0, 0.0, 2),
                t("3", "sell", 15.0, 5.0, 1.5, 3),
            ],
        );
        assert!((a.realized_pnl_sol - 32.5).abs() < 1e-8);
        assert_eq!(a.matched_sells, 1);
    }
    #[test]
    fn unknown_cost_basis_is_not_profit() {
        let a = analyze("w", &[t("1", "sell", 5.0, 100.0, 0.1, 1)]);
        assert_eq!(a.realized_pnl_sol, 0.0);
        assert_eq!(a.win_rate, None);
        assert_eq!(a.unmatched_quantity, 5.0);
    }
    #[test]
    fn chronology_and_drawdown() {
        let a = analyze(
            "w",
            &[
                t("4", "sell", 1.0, 1.0, 0.0, 4),
                t("2", "sell", 1.0, 4.0, 0.0, 2),
                t("1", "buy", 1.0, 2.0, 0.0, 1),
                t("3", "buy", 1.0, 3.0, 0.0, 3),
            ],
        );
        assert_eq!(a.realized_pnl_sol, 0.0);
        assert_eq!(a.max_drawdown_sol, 2.0);
        assert_eq!(a.win_rate, Some(50.0));
    }

    /// Costless by construction, so a test that is not about costs measures
    /// only what it claims to.
    fn free() -> crate::fees::CostModel {
        crate::fees::CostModel {
            fixed_lamports: 0,
            bps: 0.0,
            failure_rate: 0.0,
        }
    }
    #[test]
    fn route_share_is_reported_only_when_routes_were_observed() {
        // Nothing in the history records a route: absent, not "all direct".
        let a = analyze("w", &[t("1", "buy", 1.0, 1.0, 0.0, 1)]);
        assert_eq!(a.routed_observed, 0);
        assert_eq!(a.routed_share_pct, None);

        let mut direct = t("2", "buy", 1.0, 1.0, 0.0, 2);
        direct.routed = Some(false);
        let mut routed = t("3", "buy", 1.0, 1.0, 0.0, 3);
        routed.routed = Some(true);
        let a = analyze("w", &[direct, routed.clone()]);
        assert_eq!(a.routed_observed, 2);
        assert_eq!(a.routed_share_pct, Some(50.0));
        assert!(!a.flags.iter().any(|f| f.contains("automated flow")));
    }
    #[test]
    fn an_entirely_routed_wallet_is_flagged_as_likely_automated() {
        let history: Vec<Trade> = (0..10)
            .map(|i| {
                let mut x = t(&format!("{i}"), "buy", 1.0, 1.0, 0.0, i + 1);
                x.routed = Some(true);
                x
            })
            .collect();
        let a = analyze("w", &history);
        assert_eq!(a.routed_share_pct, Some(100.0));
        assert!(a.flags.iter().any(|f| f.contains("automated flow")));
    }
    #[test]
    fn scaling_out_of_winners_inflates_the_sell_event_rate_but_not_round_trips() {
        // One winning lot sold in four tranches, one losing lot dumped at once.
        // Sell events: 4 wins / 1 loss = 80%. Round trips: 1 win / 1 loss = 50%.
        let a = analyze(
            "w",
            &[
                t("b1", "buy", 4.0, 1.0, 0.0, 1),
                t("s1", "sell", 1.0, 2.0, 0.0, 2),
                t("s2", "sell", 1.0, 2.0, 0.0, 3),
                t("s3", "sell", 1.0, 2.0, 0.0, 4),
                t("s4", "sell", 1.0, 2.0, 0.0, 5),
                t("b2", "buy", 4.0, 1.0, 0.0, 6),
                t("s5", "sell", 4.0, 0.5, 0.0, 7),
            ],
        );
        assert_eq!(a.win_rate, Some(80.0));
        assert_eq!(a.round_trips, 2);
        assert_eq!(a.round_trip_win_rate, Some(50.0));
        assert!(a.flags.iter().any(|f| f.contains("scales out of winners")));
    }
    #[test]
    fn a_lot_that_only_breaks_even_across_tranches_is_not_a_winning_round_trip() {
        // Sold half at a profit and half at an equal loss: two sell events, one
        // of them "winning", but the acquisition itself made nothing.
        let a = analyze(
            "w",
            &[
                t("b", "buy", 2.0, 1.0, 0.0, 1),
                t("s1", "sell", 1.0, 1.5, 0.0, 2),
                t("s2", "sell", 1.0, 0.5, 0.0, 3),
            ],
        );
        assert_eq!(a.win_rate, Some(50.0));
        assert_eq!(a.round_trips, 1);
        assert_eq!(a.round_trip_win_rate, Some(0.0));
        assert!(a.realized_pnl_sol.abs() < 1e-12);
    }
    #[test]
    fn an_open_lot_is_not_yet_a_round_trip() {
        let a = analyze("w", &[t("b", "buy", 5.0, 1.0, 0.0, 1)]);
        assert_eq!(a.round_trips, 0);
        assert_eq!(a.round_trip_win_rate, None);
    }

    fn tr(id: &str, w: &str, side: &str, q: f64, p: f64, time: i64, liq: f64) -> Trade {
        Trade {
            id: id.into(),
            wallet: w.into(),
            token: "t".into(),
            symbol: "T".into(),
            side: side.into(),
            quantity: q,
            price_sol: p,
            fee_sol: 0.0,
            timestamp: time,
            liquidity_sol: liq,
            routed: None,
        }
    }
    // Leader buys at 1.0 and sells at 2.0. The market prints 1.1 two seconds after
    // the entry and 1.9 two seconds after the exit, so a slower follower pays more
    // and receives less.
    fn market() -> Vec<Trade> {
        vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "other", "buy", 10.0, 1.1, 102, 10000.0),
            tr("3", "w", "sell", 100.0, 2.0, 200, 10000.0),
            tr("4", "other", "sell", 10.0, 1.9, 202, 10000.0),
        ]
    }
    #[test]
    fn delay_costs_the_follower_money() {
        let all = market();
        let immediate = follower_result("w", &all, 0, 1.0, &free(), 3600);
        let slow = follower_result("w", &all, 2, 1.0, &free(), 3600);
        assert_eq!(
            (immediate.attempts, immediate.entered, immediate.resolved),
            (1, 1, 1)
        );
        assert!((immediate.realized_pnl_sol - 1.0).abs() < 1e-9);
        assert_eq!(immediate.median_entry_slippage_pct, Some(0.0));
        // 1/1.1 tokens sold at 1.9 leaves about 0.727 SOL.
        assert!((slow.realized_pnl_sol - 0.727272727).abs() < 1e-6);
        assert!(slow.realized_pnl_sol < immediate.realized_pnl_sol);
        assert!((slow.median_entry_slippage_pct.unwrap() - 10.0).abs() < 1e-9);
    }
    #[test]
    fn costs_are_charged_on_both_sides() {
        let free = follower_result("w", &market(), 0, 1.0, &free(), 3600);
        let charged = follower_result(
            "w",
            &market(),
            0,
            1.0,
            &crate::fees::CostModel {
                fixed_lamports: 0,
                bps: 200.0,
                failure_rate: 0.0,
            },
            3600,
        );
        assert!(charged.realized_pnl_sol < free.realized_pnl_sol);
    }
    #[test]
    fn an_unavailable_price_is_an_unfilled_order_not_a_free_fill() {
        // Nothing prints within the wait window after the delayed entry time.
        let r = follower_result("w", &market(), 50, 1.0, &free(), 10);
        assert_eq!((r.attempts, r.entered, r.resolved), (1, 0, 0));
        assert_eq!(r.unfilled_entries, 1);
        assert_eq!(r.realized_pnl_sol, 0.0);
        assert_eq!(r.win_rate, None);
    }
    #[test]
    fn unknown_liquidity_is_never_copied() {
        let mut all = market();
        all[0].liquidity_sol = 0.0;
        let r = follower_result("w", &all, 0, 1.0, &free(), 3600);
        assert_eq!((r.attempts, r.entered, r.unknown_liquidity), (1, 0, 1));
        assert_eq!(r.realized_pnl_sol, 0.0);
    }
    #[test]
    fn order_size_is_capped_by_supplied_liquidity() {
        let mut all = market();
        for t in &mut all {
            t.liquidity_sol = 200.0; // cap is 0.2 SOL
        }
        let r = follower_result("w", &all, 0, 5.0, &free(), 3600);
        assert!((r.realized_pnl_sol - 0.2).abs() < 1e-9);
    }
    #[test]
    fn a_position_the_leader_never_exits_is_unresolved_not_profitable() {
        let all = vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "other", "buy", 10.0, 9.9, 150, 10000.0),
        ];
        let r = follower_result("w", &all, 0, 1.0, &free(), 3600);
        assert_eq!((r.entered, r.resolved, r.unresolved_exits), (1, 0, 1));
        assert_eq!(r.realized_pnl_sol, 0.0);
    }
    #[test]
    fn unresolved_exits_are_marked_at_total_loss_in_the_worst_case_column() {
        let all = vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "other", "buy", 10.0, 9.9, 150, 10000.0),
        ];
        let r = follower_result("w", &all, 0, 1.0, &free(), 3600);
        assert_eq!((r.entered, r.resolved, r.unresolved_exits), (1, 0, 1));
        assert_eq!(r.realized_pnl_sol, 0.0);
        // The whole 1 SOL entered and never came back out.
        assert!((r.realized_pnl_sol_worst_case + 1.0).abs() < 1e-9);
        assert_eq!(r.resolution_rate, Some(0.0));
    }
    #[test]
    fn a_row_whose_sign_flips_under_the_worst_case_is_flagged_as_uninformative() {
        // One small winner that resolves, one entry that never exits.
        let all = vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "w", "sell", 100.0, 1.2, 200, 10000.0),
            tr("3", "w", "buy", 100.0, 1.0, 300, 10000.0),
        ];
        let c = copyability("w", &all, &[0], 1.0, &free(), 3600);
        assert!(c.decay[0].realized_pnl_sol > 0.0);
        assert!(c.decay[0].realized_pnl_sol_worst_case < 0.0);
        assert!(c.flags.iter().any(|f| f.contains("flip the sign")));
    }
    #[test]
    fn the_fixed_cost_term_is_charged_on_both_sides() {
        let cost = crate::fees::CostModel {
            fixed_lamports: 10_000_000, // 0.01 SOL per transaction
            bps: 0.0,
            failure_rate: 0.0,
        };
        // Entry 1.0 SOL: 0.01 goes to cost, 0.99 buys tokens at 1.0, exit at 1.0
        // returns 0.99 gross less 0.01 = 0.98. Net −0.02, the two fixed charges.
        let all = vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "w", "sell", 100.0, 1.0, 200, 10000.0),
        ];
        let r = follower_result("w", &all, 0, 1.0, &cost, 3600);
        assert!((r.realized_pnl_sol + 0.02).abs() < 1e-9);
    }
    #[test]
    fn an_order_below_the_cost_crossover_is_flagged() {
        let all = vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "w", "sell", 100.0, 1.0, 200, 10000.0),
        ];
        // 0.01 SOL fixed against 10 bps crosses over at 10 SOL, so a 0.001 SOL
        // order is overwhelmingly fixed cost.
        let cost = crate::fees::CostModel {
            fixed_lamports: 10_000_000,
            bps: 10.0,
            failure_rate: 0.0,
        };
        assert!((cost.fixed_dominates_below_sol().unwrap() - 10.0).abs() < 1e-9);
        let small = copyability("w", &all, &[0], 0.001, &cost, 3600);
        assert!(
            small
                .flags
                .iter()
                .any(|f| f.contains("fixed per-transaction cost"))
        );
        // A large order is past the crossover and is not flagged for this.
        let large = copyability("w", &all, &[0], 50.0, &cost, 3600);
        assert!(
            !large
                .flags
                .iter()
                .any(|f| f.contains("fixed per-transaction cost"))
        );
    }
    #[test]
    fn decay_ladder_flags_an_edge_that_only_speed_captures() {
        // Profitable immediately; the price has already collapsed 60 seconds later.
        let all = vec![
            tr("1", "w", "buy", 100.0, 1.0, 100, 10000.0),
            tr("2", "other", "buy", 10.0, 3.0, 160, 10000.0),
            tr("3", "w", "sell", 100.0, 3.0, 200, 10000.0),
            tr("4", "other", "sell", 10.0, 0.2, 260, 10000.0),
        ];
        let c = copyability("w", &all, &DEFAULT_DELAYS, 1.0, &free(), 3600);
        assert_eq!(c.decay.len(), 5);
        assert!(c.decay[0].realized_pnl_sol > 0.0);
        assert!(c.decay[4].realized_pnl_sol < 0.0);
        assert!(c.flags.iter().any(|f| f.contains("depends on speed")));
        assert!(c.leader_realized_pnl_sol > 0.0);
        assert!(
            c.flags
                .iter()
                .any(|f| f.contains("simulated follower is not"))
        );
    }
}

/// Outcome of copying one wallet at one fixed follower delay.
///
/// `attempts` counts leader buys considered. Everything that did not become a
/// completed round trip is reported in its own counter rather than quietly
/// dropped, because the excluded cases are where copying actually fails.
#[derive(Debug, Serialize, PartialEq)]
pub struct FollowerResult {
    pub delay_seconds: i64,
    pub attempts: usize,
    pub entered: usize,
    pub resolved: usize,
    pub unknown_liquidity: usize,
    pub unfilled_entries: usize,
    pub unresolved_exits: usize,
    pub realized_pnl_sol: f64,
    /// Every unresolved exit marked at a total loss of the entered notional.
    ///
    /// "No exit price observed" and "the token stopped trading" are the same
    /// event in this dataset, and tokens stop trading because they died. The
    /// excluded tail is not missing at random, so the headline figure is an
    /// upper bound and this is the lower one.
    pub realized_pnl_sol_worst_case: f64,
    pub resolution_rate: Option<f64>,
    pub win_rate: Option<f64>,
    pub expectancy_sol: Option<f64>,
    pub median_entry_slippage_pct: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct Copyability {
    pub wallet: String,
    pub order_sol: f64,
    pub cost_model: crate::fees::CostModel,
    pub max_wait_seconds: i64,
    pub leader_realized_pnl_sol: f64,
    pub decay: Vec<FollowerResult>,
    pub flags: Vec<String>,
}

/// Observed price path for one token, from every wallet's trades in the dataset.
fn price_series(all: &[Trade]) -> BTreeMap<String, Vec<(i64, f64)>> {
    let mut series: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
    for t in all {
        series
            .entry(t.token.clone())
            .or_default()
            .push((t.timestamp, t.price_sol));
    }
    for points in series.values_mut() {
        points.sort_by_key(|a| a.0);
    }
    series
}

/// First observed price at or after `at`, within `max_wait`. Returns `None` when
/// nothing was observed in the window: an unfilled order, never an assumed fill at
/// the leader's price.
fn fill_at(points: &[(i64, f64)], at: i64, max_wait: i64) -> Option<f64> {
    let index = points.partition_point(|(time, _)| *time < at);
    points
        .get(index)
        .filter(|(time, _)| *time - at <= max_wait)
        .map(|(_, price)| *price)
}

/// What a follower would have realized copying this wallet, entering and exiting
/// `delay` seconds after each of its trades, at observed prices.
///
/// This is a *price-observation* backtest. It applies a flat cost to each side and
/// caps the order at 0.1% of supplied liquidity. It does not model pool price
/// impact, queue competition against other followers, failed transactions, or
/// partial fills, so it remains optimistic — a wallet that fails here would fail
/// live by more, which is the direction that makes the measure useful.
pub fn follower_result(
    wallet: &str,
    all: &[Trade],
    delay: i64,
    order_sol: f64,
    cost: &crate::fees::CostModel,
    max_wait: i64,
) -> FollowerResult {
    let series = price_series(all);
    let mut leader: Vec<&Trade> = all.iter().filter(|t| t.wallet == wallet).collect();
    leader.sort_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)));
    let mut open: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let (mut attempts, mut entered, mut resolved) = (0, 0, 0);
    let (mut unknown, mut unfilled, mut slippages) = (0, 0, vec![]);
    let (mut pnl, mut wins) = (0.0_f64, 0);
    for t in &leader {
        let Some(points) = series.get(&t.token) else {
            continue;
        };
        if t.side == "buy" {
            if open.contains_key(&t.token) {
                continue;
            }
            attempts += 1;
            if t.liquidity_sol <= 0.0 {
                unknown += 1;
                continue;
            }
            let Some(price) = fill_at(points, t.timestamp + delay, max_wait) else {
                unfilled += 1;
                continue;
            };
            let size = order_sol.min(t.liquidity_sol * 0.001);
            // Entry cost is fixed-plus-proportional, so the whole order does not
            // buy tokens: the fixed part is consumed regardless of size.
            let spendable = size - cost.charge(size);
            if !price.is_finite() || price <= 0.0 || spendable <= 0.0 {
                unfilled += 1;
                continue;
            }
            slippages.push((price - t.price_sol) / t.price_sol * 100.0);
            open.insert(t.token.clone(), (spendable / price, size));
            entered += 1;
        } else if let Some((quantity, spent)) = open.remove(&t.token) {
            let Some(price) = fill_at(points, t.timestamp + delay, max_wait) else {
                // Leader is out; the follower is still holding with no observed
                // exit price. Unresolved, and excluded from realized P&L.
                open.insert(t.token.clone(), (quantity, spent));
                continue;
            };
            let gross = quantity * price;
            let result = gross - cost.charge(gross) - spent;
            pnl += result;
            resolved += 1;
            if result > 0.0 {
                wins += 1;
            }
        }
    }
    slippages.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let stranded: f64 = open.values().map(|(_, spent)| *spent).sum();
    FollowerResult {
        delay_seconds: delay,
        attempts,
        entered,
        resolved,
        unknown_liquidity: unknown,
        unfilled_entries: unfilled,
        unresolved_exits: open.len(),
        realized_pnl_sol: pnl,
        realized_pnl_sol_worst_case: pnl - stranded,
        resolution_rate: (entered > 0).then(|| resolved as f64 / entered as f64 * 100.0),
        win_rate: (resolved > 0).then(|| wins as f64 / resolved as f64 * 100.0),
        expectancy_sol: (resolved > 0).then(|| pnl / resolved as f64),
        median_entry_slippage_pct: slippages.get(slippages.len() / 2).copied(),
    }
}

pub const DEFAULT_DELAYS: [i64; 5] = [0, 2, 5, 15, 60];

/// Runs the follower simulation across a delay ladder. The shape of the resulting
/// curve is the point: an edge that disappears between 0 and 5 seconds belongs to
/// whoever is fastest, not to anyone copying the wallet.
pub fn copyability(
    wallet: &str,
    all: &[Trade],
    delays: &[i64],
    order_sol: f64,
    cost: &crate::fees::CostModel,
    max_wait: i64,
) -> Copyability {
    let decay: Vec<_> = delays
        .iter()
        .map(|d| follower_result(wallet, all, *d, order_sol, cost, max_wait))
        .collect();
    let leader_pnl = analyze(wallet, all).realized_pnl_sol;
    let mut flags = vec![
        "Follower outcomes at observed prices. No pool impact, queue competition, failed transactions or partial fills are modeled, so these results are optimistic.".to_string(),
    ];
    let immediate = decay.first().map(|r| r.realized_pnl_sol);
    let delayed = decay.last().map(|r| r.realized_pnl_sol);
    if let (Some(a), Some(b)) = (immediate, delayed)
        && a > 0.0
        && b <= 0.0
    {
        flags.push(
            "Edge disappears across the delay ladder: profit here depends on speed, not selection"
                .into(),
        );
    }
    if leader_pnl > 0.0 && delayed.is_some_and(|p| p <= 0.0) {
        flags.push(
            "Source wallet is profitable but the simulated follower is not at the longest delay"
                .into(),
        );
    }
    if decay.iter().any(|r| r.unresolved_exits > 0) {
        flags.push(
            "Some simulated positions never observed an exit price and are excluded from the headline P&L; the worst-case column marks them at a total loss"
                .into(),
        );
    }
    if decay
        .iter()
        .any(|r| r.realized_pnl_sol > 0.0 && r.realized_pnl_sol_worst_case <= 0.0)
    {
        flags.push(
            "Unresolved exits flip the sign of at least one row: that row carries no information"
                .into(),
        );
    }
    if let Some(floor) = cost.fixed_dominates_below_sol()
        && order_sol < floor
    {
        flags.push(format!(
            "At {order_sol} SOL the fixed per-transaction cost outweighs the proportional one; below {floor:.4} SOL order size dominates the result"
        ));
    }
    if decay
        .iter()
        .any(|r| r.attempts > 0 && r.resolved * 2 < r.attempts)
    {
        flags.push("Fewer than half of the leader's buys became completed copies".into());
    }
    Copyability {
        wallet: wallet.into(),
        order_sol,
        cost_model: cost.clone(),
        max_wait_seconds: max_wait,
        leader_realized_pnl_sol: leader_pnl,
        decay,
        flags,
    }
}

#[cfg(test)]
mod identifier_tests {
    use super::*;
    /// Regression: real signatures are 88 characters and real addresses 44, so a
    /// composite id is ~137. An id-length bound below that silently discards
    /// every genuine observation while every short-fixture test still passes.
    #[test]
    fn a_realistically_long_composite_id_is_accepted() {
        let signature = "a".repeat(88);
        let wallet = "b".repeat(44);
        let t = Trade {
            id: format!("rpc:{signature}:{wallet}"),
            wallet: wallet.clone(),
            token: "c".repeat(44),
            symbol: "TOK".into(),
            side: "buy".into(),
            quantity: 100.0,
            price_sol: 0.001,
            fee_sol: 0.000005,
            timestamp: 1_700_000_000,
            liquidity_sol: 50.0,
            routed: Some(true),
        };
        assert!(
            t.id.len() > 128,
            "the id really is longer than the old bound"
        );
        assert!(t.validate());
        // The bound still exists; it is just placed where real data fits.
        let mut absurd = t.clone();
        absurd.id = "x".repeat(257);
        assert!(!absurd.validate());
        let mut empty = t.clone();
        empty.id = String::new();
        assert!(!empty.validate());
        // Addresses are still held to the tighter bound.
        let mut long_wallet = t.clone();
        long_wallet.wallet = "w".repeat(129);
        assert!(!long_wallet.validate());
    }
}
