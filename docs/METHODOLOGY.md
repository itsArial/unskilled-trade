# Accounting, scoring and paper execution

## Inputs

`Trade` is a normalized event, not a raw transaction. Amounts use token units and SOL. IDs must uniquely identify a trade instruction/event, not just a transaction containing multiple swaps. The caller supplies liquidity; zero means unknown. Import is restricted to administrators. See `API.md` for the schema.

The optional RPC converter is intentionally narrow: one direct outer Pump or PumpSwap instruction; one changed non-WSOL asset owned by the signer; successful transaction; native SOL cash-flow direction opposite the token change. It sums native lamports over the wallet and its observed token accounts to offset ATA rent movements, separates the network fee if the watched wallet paid it, and rejects routed/multi-asset cases. Protocol fees remain embedded in the effective cash-flow price. It does not recognize every Pump version or all economic side effects. Results are estimated observations with explicit coverage flags, not tax-grade accounting. Liquidity is unknown and therefore blocks new paper positions.

Raw finalized RPC results are persisted in `raw_transactions`. The converter does not prove complete history, decode transfer provenance, resolve token symbols, support every quote asset or infer private platform account ownership. RPC transaction version support is currently capped at v0; an unsupported response is a visible error. Extend the decoder before increasing the cap.

## FIFO accounting

For each wallet and token, keep acquisition lots `(remaining_quantity, unit_cost_including_buy_fee, buy_timestamp)` in chronological order. Consume oldest lots on sells. Subtract sell fees proportionately to matched quantities. A partially unmatched sell records unmatched quantity and excludes the unmatched portion from realized P&L. It is never assigned zero cost.

Sort by `(timestamp, event ID)` for deterministic replay. Same-second ordering is therefore a limitation: production ingestion must add slot, transaction index and instruction index. Timestamp-only imports cannot establish true ordering within a block.

Metrics:

- **Realized P&L:** cumulative result of matched quantities after supplied fees; open holdings are not marked to market.
- **Win rate:** profitable matched sell events / all matched sell events. Partial sells are separate observations, not independent completed round trips. **This figure is mechanically inflated** by scaling out: a wallet that sells a winner in five tranches and dumps a loser in one reports 83% from a 50% hit rate. Prefer the round-trip rate below.
- **Round trips / round-trip win rate:** results grouped by *acquisition lot*. A lot accumulates the profit realized on each tranche and resolves to one observation when it is fully consumed, so scaling out no longer multiplies winners. An open lot is not yet a round trip. Where the two rates differ by more than ten points a flag says so.
- **Profit factor:** gross positive results / absolute gross negative results. Returns `null` when there are no losses, rather than inventing a finite value.
- **Realized drawdown:** maximum peak-to-trough decline of cumulative realized P&L in SOL; not equity drawdown or a percentage.
- **Median holding time:** median of FIFO lot matches; not quantity weighted, and split lots may contribute multiple observations.
- **Expectancy:** mean matched-sell result in SOL; descriptive only.
- **Largest win share:** largest profitable sell divided by gross profitable sells.
- **Longest losing streak:** maximum run of nonpositive matched sell events.
- **95% win-rate interval:** Wilson score interval with z=1.96. Independence is an approximation, not a proven property of these observations.
- **Unmatched quantity:** diagnostic sum across assets; token quantities are not economically comparable. A nonzero value is a missing-cost-basis flag, not a portfolio balance.

`f64` is used for research metrics and synthetic balances. Production accounting and live order quantities need bounded fixed-point/integer arithmetic by mint decimals. Input magnitudes are bounded and finite; this does not make floats appropriate for signed money movements.

## Score

The score's win-rate input is now the **round-trip** rate, not the sell-event rate. The weights are
unchanged and remain uncalibrated; `docs/EVIDENCE.md` C4 requires that any new formula be fitted and
held out rather than hand-tuned, so substituting a less biased input is the only correction made
here. The other defects EVIDENCE names in D2 — sample size counted twice, realized P&L entering as a
flat indicator, and an effective win-rate threshold that tightens as the sample *falls* — are
unaddressed and remain listed.

```text
base = 0.45 × round_trip_win_percentage
     + 30 × min(matched_sell_events, 40) / 40
     + (25 if realized_P&L > 0 else 0)
score = floor(base × (0.6 if unmatched_quantity > 0 else 1))
```

This is an uncalibrated heuristic. It does not estimate the probability of profit or fraud. Diagnostics additionally flag small samples, unknown/thin liquidity, rapid holds, concentrated gains, losing streaks, synthetic history and partial RPC coverage.

The public preview uses fixed example scores to demonstrate the layout. Authenticated analysis is calculated by Rust. Both preview and imported demo history are labeled synthetic.

## Decoding real swaps

Most Pump volume does not call the program directly. It arrives through Jupiter or a bot router with
the Pump instruction nested inside, so a decoder that required a direct top-level call rejected the
majority of live trades. Both shapes are decoded now.

The measurement is the wallet's **net position change**, not the route: an address that spent X SOL
and ended holding Y of one token paid an effective price of X/Y, however many pools were crossed.
Aggregator and platform fees sit inside that number, which makes the recorded price slightly worse
than the pool price — the conservative direction.

Each observation records whether it was `routed`. `Some(false)` is a direct invocation, `Some(true)`
went through an aggregator, and `None` means the source does not report it — absent is *not
observed*, never "was direct". Published work treats direct invocation versus routing as a proxy for
bot-attributed flow, so `routed_share_pct` is reported per wallet and a share at or above 90% raises
a flag for likely automated flow.

Known limits, all observed against live mainnet data:

- **Pool liquidity often stays unknown on routed swaps.** The counterparty is identified by lamport
  movement within a 10% band and must be unique; a multi-hop route moves SOL through several pools,
  so the match is frequently ambiguous. Ambiguous reports `0` = unknown, which blocks paper entry.
  In a live sample only one swap in three resolved a pool balance.
- **A route that merely passes through Pump is still recorded.** A Jupiter route that touches Pump
  but settles in another asset decodes as a trade in that asset. The price is the wallet's real
  effective price, but the venue attribution is loose.
- **A transaction that both swaps and transfers the same token nets to one delta**, so the implied
  price would be wrong. These are not currently detected.
- **Arbitrage is correctly refused.** A wallet that ends a transaction holding more tokens *and*
  more SOL, or with no net token change at all, fails the opposite-sign test. In a live sample of 35
  Pump transactions from one address, 34 were refused this way — the address was an arbitrage bot,
  not a directional trader. Refusing them is the intended behaviour.

## Copyability: follower-adjusted results

Leader P&L answers "did this wallet make money". It does not answer "would copying it have made
*me* money", which is the only question that matters to a follower. `copyability()` answers the
second one directly.

For each delay `D` on the ladder (0, 2, 5, 15 and 60 seconds by default):

1. Build a price path per token from **every** wallet's observed trades, not just the leader's.
2. On each leader buy with no open simulated position, the follower places an order at `D` seconds
   later. The fill price is the first observed trade on that token at or after `entry_time + D`,
   within `COPY_MAX_WAIT_SECONDS`.
3. Order size is `min(order_sol, supplied_liquidity × 0.1%)`. Unknown liquidity is never copied.
4. On the leader's sell, the follower exits at the first observed price at or after `exit_time + D`.
5. `COPY_COST_BPS` is charged on entry and exit.

Every case that is not a completed round trip is counted in its own field rather than dropped:

- `unknown_liquidity`: the leader's buy carried no liquidity observation.
- `unfilled_entries`: no price was observed in the wait window. **Not** a fill at the leader's price.
- `unresolved_exits`: the leader exited but no follower exit price was ever observed. The position
  is excluded from realized P&L; it is not marked at the last price and not counted as a win.

Reported per delay: `attempts`, `entered`, `resolved`, `realized_pnl_sol`, `win_rate`,
`expectancy_sol` and `median_entry_slippage_pct` (the follower's fill against the leader's price).

**Worst-case bound.** Each row also reports `realized_pnl_sol_worst_case`, marking every unresolved
exit at a total loss of the entered notional, and `resolution_rate`. In this dataset "no exit price
observed" and "the token stopped trading" are the same event, and tokens stop trading because they
died — so the excluded tail is not missing at random and the headline figure is an upper bound. When
the two disagree in sign, a flag says the row carries no information.

**Cost shape.** Costs are `fixed_lamports × (1 + failure_rate) + bps × notional` on each side, not
proportional alone. Network cost is mostly fixed per transaction and is charged on failures too, so
a purely proportional model understates small orders badly — and the liquidity cap pushes orders
small. At 0.1 SOL against a 100 SOL pool floor, the fixed term alone is already a material fraction
of a percent per side. `fixed_dominates_below_sol()` reports the crossover and a flag fires when the
configured order size is under it. The same function prices paper entries, paper exits, tick exits
and manual closes, so all four remain comparable.

**What this does not model.** Pool price impact, queue competition between followers, failed
transactions, partial fills, priority-fee escalation and reorgs are all absent. The simulation is
therefore *optimistic*: a wallet that already fails here would fail harder live. That asymmetry is
the reason the measure is worth reporting — it is useful for rejection, not for promise.

The price oracle is the observed trade set. A token that trades rarely has a sparse path, so its
fills are coarse and its results are weak evidence. Sample-size caveats apply exactly as they do to
the descriptive statistics above.

### Alpha decay

The shape of the ladder carries more information than any single number. A wallet profitable at
delay 0 and unprofitable at delay 60 has an edge that belongs to whoever is fastest, not to anyone
copying it. That case raises `Edge disappears across the delay ladder`. A wallet whose leader P&L is
positive while the longest-delay follower P&L is not raises a separate flag, because that gap is
precisely the difference a leaderboard hides.

## Authenticity: structural reasons a wallet is not copyable

A wallet can be reliably profitable for reasons that have nothing to do with selection skill, and
those reasons do not transfer to a follower. Two are measurable from data we already hold.

**Early-entry rate.** The share of a wallet's buys placed within `EARLY_ENTRY_WINDOW_SECONDS` of the
first trade *observed on that token in our dataset*. A wallet that is consistently present in the
opening seconds is either the deployer's, a sniper, or informed in advance. A follower acting on a
signal cannot arrive that early, so the wallet's results are unreachable regardless of how good they
look. Above `MAX_EARLY_ENTRY_PCT` this refuses entry, unless `ENFORCE_AUTHENTICITY=false`.

The reference point is our own first observation, not the token's true creation. Incomplete coverage
makes this measure *conservative in the wrong direction*: a wallet can look late simply because we
did not see the earlier trades. Read a low rate as weak evidence and a high rate as strong evidence.

**Funding graph.** Native SOL transfers extracted from archived raw transactions give, per wallet:
its observed `funders`, the `co_funded_wallets` fed by at least one of the same sources, and
`funded_by_counterparty` — funders that traded a token *before* this wallet bought it, which is the
shape shared information leaves.

Only the System Program's parsed `transfer` form is recognised. Token transfers, `transferWithSeed`
and unparsed CPI shapes are skipped. A missing edge therefore means "not observed", never "did not
happen", and `funding_graph_observed: false` marks a wallet whose graph is **unknown, not clean**.

Funding is reported, never enforced. Exchanges, bridges and shared custody produce exactly the same
edge shape as coordination, so a shared funder is routinely innocent. Only the timing measure gates,
because it is computed from our own trade record rather than inferred intent. Nothing in this
module asserts wrongdoing by any address.

## Paper policy

Per-user state: followed wallets, subscription expiry, ban status, enabled flag and order budget. Recent events from the trusted admin ingestion endpoint automatically evaluate eligible followers. Historical imports and manual RPC collection never auto-run strategies. Historical replay is a separate explicit user action.

Entry requirements:

- Active subscription, enabled execution, no ban and a followed source wallet.
- Source event is a buy.
- At least 20 prior matched sells, score at least 65, and no unmatched cost basis in prior history.
- Supplied liquidity at least 100 SOL; order size no more than 0.1% of that supplied liquidity.
- Aggregate open cost plus proposed order no more than five times the current order budget.
- Current UTC-day realized result has not fallen below minus twice the current order budget.

Entries additionally pass the token safety gate described in `API.md`: a mint reported as rugged, or
retaining mint or freeze authority, or whose worst observed market locks less pooled liquidity than
`TOKEN_RISK_MIN_LP_LOCKED`, or whose upstream risk score exceeds `TOKEN_RISK_MAX_SCORE`, is refused.
A token with no collected assessment is *unknown*; under `TOKEN_RISK_REQUIRED` unknown is refused
too. Reported holder concentration includes pool accounts and is context only, never a block.

Every refusal is recorded by reason and returned to the caller, so a user who sees no orders is told
which gate rejected the trade rather than being left to guess.

A threshold is a guard evaluated at observations, not a guaranteed maximum loss. Daily losses can exceed the threshold after an exit; the guard then blocks entries. Changing the order budget changes the derived limits. Production should persist independently configured risk limits and ledger reservations.

Entry quantity = `(budget − cost(budget)) / source_price`; the fixed component is consumed whether
or not it buys anything, so it does not acquire tokens. An order smaller than the cost of placing it
is refused with that reason.
Exit proceeds = `gross − cost(gross)` where `gross = quantity × observed_price`.

These are **assumptions**, not observed network fees or quotes. There is no real execution-latency model, pool price-impact model, queue competition, failed transaction model or follower-capacity model. The liquidity cutoff is a filter, not a simulated fill.

Exit conditions on a new event for the same leader/token:

- Any leader sell: close the follower’s complete simulated position, including on partial source exits.
- Observed price below 80% of the entry reference.
- Supplied liquidity below 100 SOL.

No independent price tick feed is connected. A price drop between source-wallet events is not detected. Unknown liquidity (`0`) can close an existing simulated position conservatively; it is not proof of a real liquidity drain. Manual close uses the latest supplied token reference price and can be stale.

## Ordering and idempotency

Paper positions and event markers commit in the same SQLite transaction. Repeated event IDs do not duplicate orders. Replay uses only the history preceding each event, never final-period wallet results. A per-user maximum processed timestamp prevents newly backfilled older observations from executing after later paper positions. For a clean replay of a revised history, use a fresh research database; there is no destructive reset endpoint.

The current replay is intended for small datasets. It recomputes prior wallet history and is not an optimized global engine. Before scaling, replace it with incremental per-wallet state, precomputed immutable analysis snapshots and a replayable event log.
