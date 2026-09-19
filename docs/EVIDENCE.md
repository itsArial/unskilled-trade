# Published evidence and resulting change list

Intended location: `docs/EVIDENCE.md`. Reviewed 2026-09-18.

This document exists because `RESEARCH.md` §5 lists hypotheses to test and `AGENTS.md` rule 2 forbids
claiming predictive power. Neither says what the outside literature has already established. This
file closes that gap: it records what has been published, with sources, and converts it into a
prioritised change list.

**Nothing here establishes that copy trading is profitable.** The one published out-of-sample test of
this repository's core thesis returned *above baseline, below breakeven* — see §2. Every change below
is a measurement or a rejection rule, not a promise. Do not move any item into a claim of edge. If an
implementation lands, move it out of the "missing" list in `RESEARCH.md` with the gap named, per
`AGENTS.md` rule 7 and the definition of done.

---

## Part I — What the literature establishes

### 1. Trader skill persists, but is rare (strongest supporting result)

Barber, Lee, Liu and Odean tracked over 450,000 Taiwanese day traders, 1992–2006. Sorting by returns
in year *y* and measuring year *y+1*, the top 500 went on to earn +37.9 bps per day net of fees while
bottom-ranked traders earned −28.9 bps net. Past performance — returns or dollar profits — is by a
large margin the best predictor of future performance; concentration of trading in a few instruments
is second. Fewer than 1% of the population was predictably and reliably profitable net of fees.

**Implication.** Ranking wallets by realized P&L is the correct primary variable, not a naive one.
But the prior for any individual wallet is roughly 1-in-100, which is the number the score's 0–100
range currently obscures.

Sources: Barber, Lee, Liu, Odean, *The Cross-Section of Speculator Skill*, Journal of Financial
Markets 18 (2014); same authors, *Do Day Traders Rationally Learn About Their Ability?*

### 2. The direct on-chain test of copying top wallets — read this one first

Marino, Naviglio, Tarantelli and Lillo, *Predicting the success of new crypto-tokens: the Pump.fun
case*, arXiv:2602.14860 (Feb 2026). One month of fully on-chain Pump.fun data, September 2025:
655,770 tokens from 243,123 creators, 4,338 graduations (0.63%), 2.6M distinct trader addresses.

Their design is the one this repo should copy. They split the month into two halves, identified
top-performing wallets by realized SOL profit **in the first half only**, fixed that set, and then
measured whether those wallets' participation predicted graduation in the second half. All
conditioning variables are computed using only information available up to the current point on the
bonding curve.

**Result.** The conditional graduation probability given early participation by an ex-ante identified
top trader lies *above* the baseline but *below* the economic breakeven curve. A naive buy-and-hold
on that signal loses money. The authors attribute this to a double-edged mechanism: experienced
traders accelerate early liquidity and also exit rapidly, often around graduation.

Two further details matter for wallet selection:

- Several of their top-PnL wallets executed **only sell transactions**, which they read as liquidity
  exit points aggregating funds from multiple addresses rather than traders making decisions. A
  leaderboard's best wallet may not be a copyable agent at all.
- Conditioning on prolific token *creators* did not improve prediction over baseline, except for a
  top-10 creator set whose statistical support they explicitly call limited.

### 3. What outperformed the top-trader signal

From the same paper, in order of strength:

1. **Liquidity velocity.** For any fixed level of SOL in the bonding curve, tokens reaching that
   level in fewer trades graduated substantially more often. This dominated every other variable
   across the entire range, and survived excluding near-instant graduations (≥10 trade floor). The
   authors call it the single most informative predictor in the study.
2. **Non-bot share.** Tokens with a higher share of non-bot-attributed early trades graduated more
   often; the signal saturates above roughly 30% non-bot. Their bot flag is derived from transaction
   logs — direct program invocation versus routing through the official frontend.
3. **Trade-count-to-liquidity** is the operational form of (1): cumulative swaps observed when a
   given liquidity threshold is first crossed.

**This repo measures none of the three.**

### 4. A mechanical result, not a statistical one

At graduation, virtual reserves disappear and the pool is backed only by real reserves. Marginal
price is continuous by construction; **depth is not**. Holding inventory constant, selling strictly
before graduation always yields more SOL than selling immediately after. This is an algebraic
consequence of the constant-product invariant across the migration boundary, not a fitted finding.

It gives insiders a provable incentive to dump pre-graduation, and the data follows: of 184,282
tokens with at least 30 swaps, 169,938 (92.22%) showed at least one dump event under a robust
median/MAD control-chart rule; only 2.55% of that set graduated. Dumps cluster at higher liquidity
levels, 56.4% single-wallet and 43.6% multi-wallet.

**Implication.** A leader's sell near a migration boundary is not evidence of a view. It is the
expected action. Exit logic should treat proximity to graduation as its own condition.

### 5. Rug detection has a measured ceiling

Li, Kuznetsov, Yanovich et al., *Catching the Rug*, arXiv:2608.20271 (Aug 2026). 6.4M Solana tokens
over 7 months (Nov 2024 – Jun 2025), Raydium and PumpFun, predicting a 1-hour outcome from the first
5 minutes of trading. Labels: TVL drawdown ≥99% from peak, OR idle ≥80% of lifetime. Rolling
forward-window time-series cross-validation, explicitly to avoid look-ahead bias.

| Metric | Best result | Notes |
| --- | --- | --- |
| MCC | ~0.39 (XGBoost, fused training → PumpFun) | In-domain PumpFun: 0.356 |
| AUCPRC | ~0.80 | In-domain: 0.756 |
| Cross-DEX MCC | ~0 or negative | Raydium→PumpFun: −0.0026 to −0.24 |

The authors state plainly that these numbers are **not yet sufficient for real-world deployment**,
especially where false negatives carry investor losses.

Two things to carry forward:

- **Their PumpFun test set was 43,835 rug / 9,711 non-rug — an 82% base rate.** A classifier that
  labels everything a rug scores F1 ≈ 0.90, higher than the 0.78 they report. Accuracy and F1 are
  meaningless here. Report MCC and AUCPRC only.
- **Tree models beat every transformer** (FT-Transformer, TabTransformer, AutoInt) on cross-domain
  robustness. Architecture is not the bottleneck; distribution shift is.

Base rate corroboration: SolRugDetector (arXiv:2603.24625) identified 76,469 rug pulls among 100,063
tokens issued in H1 2025.

### 6. Structural detection is where the proven signal is

This validates `cluster.rs` over any chart-based approach.

**Deployer-funded snipers.** Pine Analytics isolated a high-confidence subset by requiring a
*provable direct pre-snipe transfer between deployer and sniper*. Over one month: 15,000+ SOL
extracted, 15,000+ launches, 4,600+ sniper wallets, 10,400+ deployers, and **87% of snipes
profitable**, with clean exits and structured operational patterns. An 87% hit rate is position, not
skill, and it is identifiable from exactly the transfer edges `cluster.rs` already extracts.

**Bundled coordination.** MELT / MemeTrans (arXiv:2602.13480), 41,470 launches and 200M+
transactions Dec 2024 – Mar 2025, with bundle traces linking accounts controlled by one entity. On
average **36.5% of token supply is held by coordinated accounts** — a concealment strategy that hides
true ownership concentration from buyers. This is the empirical answer to why
`METHODOLOGY.md` is right to say holder concentration is context, not a gate: the visible
concentration is not the real one. Models trained on their 122 features reduced simulated investment
loss by up to 56.1%.

Note what that 56.1% is: a **loss reduction**, not a return. It is the strongest "this works" number
in the literature and it is a rejection result.

### 7. Costs are larger than the simulation assumes

- Sandwich bots on Solana extracted **$370M–$500M over 16 months** to May 2025, paying only 5–15%
  back out in tips (Ghostlogs, Accelerate 2025, from 8.5B trades / $1T DEX volume).
- A peer-reviewed measurement of Jito over four months in early 2025 found **500K+ sandwiching
  instances causing $7.7M+ in victim losses**, plus 2.4M defensive user actions (ACM IMC 2025).
- Per-attack damage is typically a fraction of a percent of a swap, but it lands on top of priority
  fees, protocol fees and entry/exit slippage.

### 8. Copy-trading literature is consistently unkind to followers

- Copied trades have a **higher probability** of positive returns than ordinary trades, but the ROI
  of successful copy trades is **lower**, and losses are typically **larger** for copied trades
  (Liu et al. 2014; Pelster & Hofmann 2018, eToro data).
- Signal providers increase exposure to lottery-like assets when at an extreme of the relative
  performance spectrum, passing a lottery-like return structure to followers.
- Traders who acquire followers become **more** susceptible to the disposition effect than traders
  nobody follows (Pelster & Hofmann, JBF 2018).
- Providing information on others' success raises risk-taking; the option to copy directly raises it
  further (Apesteguia, Oechssler, Weidenholzer, *Management Science* 66(12), 2020).
- IOSCO FR/06/2025 (*Online Imitative Trading Practices*, May 2025) flags conflicts where the
  intermediary's remuneration structure for lead traders creates conflicts against copiers, and
  recommends regular review of lead-trader conduct and copier outcomes.

**Implication for this product.** These are structural, not incidental. A platform that grows a
follower base changes the leader's behaviour. Nothing in the repo models that feedback.

### 9. Base rates and regime

| Quantity | Value | Source |
| --- | --- | --- |
| Pump.fun graduation rate | 0.63% (4,338 / 655,770, Sep 2025) | arXiv:2602.14860 |
| Median time to graduation | 4.4 minutes; median 457 bonding-curve steps | arXiv:2602.14860 |
| Solana DEX wallets profitable over 90 days | 6.25% (19,003 / 304,161); median trader −$120; 88% of winners under $100 | FOMO dashboard via press, Aug–Sep 2026 |
| Pump.fun monthly profitable wallets | <50% most months Apr 2024 – Dec 2025; low 30.1% (Jun 2025); 73.3% (Apr 2026) | CoinGecko Research / Dune |

**The regime shifted and this matters more than the level.** CoinGecko attributes the 2026 rise to a
compositional change — monthly active wallets fell from 5.2M (May 2025) to 1.8M (Dec 2025) — i.e. the
losing cohort left. Any model fitted on 2024–25 data is fitted to a different population than the one
trading now. This is the same concept drift the rug-detection paper measured across venues, appearing
across time instead.

---

## Part II — Defects in the current methodology

These are properties of the code as documented, found before the literature review. They are
independent of anything above.

### D1. Win rate is mechanically inflated by exit style — and the score weights it 45%

`METHODOLOGY.md` counts profitable **matched sell events**, with partial sells as separate
observations. A wallet that scales out of winners in five tranches and dumps losers in one produces
five winning observations and one losing one from a 50% hit rate — reporting 83%. The bias is
systematic and favours precisely the scaling-out behaviour common in wallets that look good.

**Fix.** Compute round-trip-level results by grouping FIFO matches by acquisition lot, not by sell
event. Report both; gate on the round-trip figure.

### D2. The entry gate is backwards

`score = floor((0.45·win% + 30·min(sells,40)/40 + 25·[pnl>0]) · [0.6 if unmatched])`, gated at ≥65
with ≥20 matched sells.

- With ≥40 sells and any positive P&L: 55 points already, so it passes at **23% win rate**.
- At the 20-sell minimum: requires **56% win rate**.
- The threshold therefore tightens as sample size *falls*.
- Realized P&L contributes a flat 25 whether the wallet netted 0.002 SOL or 900 SOL.
- Sample size is counted twice (hard gate *and* 30 points) and saturates at 40.

### D3. Excluding unresolved exits biases realized P&L upward

The price oracle is the observed trade set, so "no exit price observed" and "the token stopped
trading" are the same event — and tokens stop trading because they died. Dropping those from
`realized_pnl_sol` removes a left tail that is **not missing at random**. §5 above puts a number on
how often that happens: the idle-based rug label exists precisely because trading cessation is the
dominant failure mode.

**Fix.** Compute a worst-case variant marking each unresolved exit at total loss. If the sign flips
between reported and worst-case, the delay row carries no information. Report `resolved / entered`
beside every figure on the row.

### D4. The cost model has the wrong shape

`COPY_COST_BPS` and the ±2% simulation constants are proportional to order size. The dominant real
costs are not: base fee plus the priority fee needed to land during memecoin activity is
approximately fixed per transaction, and is charged on failures too. At the order sizes this policy
produces — 0.1% of a ≥100 SOL pool, so 0.1 SOL at the floor — a few thousandths of a SOL is a full
percentage point per side. The error grows as orders shrink, which is the direction the liquidity cap
pushes.

**Fix.** `cost = fixed_lamports + bps × notional`, with a configurable failure rate charging only the
fixed term.

### D5. Capacity is never measured

Every gate asks whether an edge exists; none asks how much capital it can carry. A 5 SOL position
needs a 5,000 SOL pool, and pools that deep are past the phase where the observed returns were made.
The size ladder runs against the edge ladder. This is not fixed by raising the 0.1% cap — raising it
makes the missing price-impact model load-bearing.

---

## Part III — Prioritised change list

Each item names the module, the acceptance criterion, and which section justifies it. Follow the
existing `AGENTS.md` definition of done: inline unit tests for pure logic, route test if a route is
added, passing `make check`, `API.md` entry if the contract changes, `METHODOLOGY.md` entry if a
user-visible number changes.

### P0 — Correctness of numbers already shown to users

**C1. Round-trip win rate.** `analytics.rs`. Group FIFO matches by acquisition lot; emit
`round_trip_win_rate` and `round_trip_count` alongside the existing sell-event figures. Deprecate the
sell-event win rate in the score. *Justified by D1.*

**C2. Worst-case copyability bound.** `analytics.rs::copyability()`. Add
`realized_pnl_sol_worst_case` per delay row, marking every unresolved exit at −100% of the entered
notional, and `resolution_rate = resolved / entered`. Add a diagnostic flag `Unresolved exits flip
the sign of this row`. Do not remove the existing field. *Justified by D3, §5.*

**C3. Fixed-plus-proportional cost model.** New env `COPY_COST_FIXED_LAMPORTS` and
`COPY_FAILURE_RATE`; apply in both `copyability()` and the paper-policy entry/exit arithmetic.
Default the failure rate to 0 so existing databases are unchanged until configured. *Justified by
D4, §7.*

**C4. Score rebuild.** Replace the current formula. Requirements, not a proposed formula — the
formula must be fitted and held out, never hand-tuned to look right:
- expectancy and profit factor must carry non-zero weight;
- realized P&L must enter by magnitude, not as a flat indicator;
- sample size must not be both a hard gate and a large score term;
- the effective win-rate threshold must not fall as sample size rises.
Keep the name `score`, keep it documented as uncalibrated, do not rename (`AGENTS.md` rule 2).
*Justified by D2.*

### P1 — Features with published out-of-sample support

**C5. Liquidity velocity.** `analytics.rs` or a new module. Per token, the cumulative number of
observed trades at the moment each liquidity threshold is first crossed. Expose as a wallet-level
distribution (what velocity regime does this wallet buy into?) and as a token-level pre-entry signal.
This is the strongest published predictor and the repo does not compute it. *Justified by §3.*

**C6. Bot-share of early flow.** `rpc.rs` / `discovery.rs`. The reference implementation flags
transactions that invoke the program directly rather than routing through the platform frontend. Our
decoder already distinguishes direct Pump/PumpSwap invocation (`direct_pf_invocation`-equivalent), so
the signal is partly reachable. Where it is not observable, record it as unknown — never default.
*Justified by §3, `AGENTS.md` rule 1.*

**C7. Pre-graduation proximity as an exit condition.** `ticks.rs`. Selling before migration is
mechanically more profitable than after, so leader sells near the boundary are expected behaviour
rather than information. Add bonding-curve progress as a tick-evaluated exit input. *Justified by
§4.*

**C8. Deployer→sniper funding gate (narrow form only).** `cluster.rs`. Promote from "reported for
review" to a gate **only** for the provable case: a direct SOL transfer from the token's deployer to
the wallet, preceding that wallet's buy of that deployer's token within a configured block window.
Everything else — shared funders, exchange edges, co-funding sets — stays report-only, exactly as
`METHODOLOGY.md` argues. New env `ENFORCE_DEPLOYER_FUNDING`, default false. *Justified by §6.*

### P2 — Evaluation infrastructure

**C9. Capacity report.** New route and UI panel. Three figures, all computable today:
(a) histogram of `min(order_sol, 0.1% × liquidity)` across the eligible trade set, with the share
falling below the C3 break-even; (b) peak concurrent SOL at risk for one follower; (c) that figure
divided across N followers. *Justified by D5.*

**C10. Split-half wallet evaluation harness.** The Marino et al. design, run against our own data:
fix a top-wallet set on period 1, measure period-2 outcomes with all conditioning variables computed
only from information available at that point on the curve. Report against the three baselines
`RESEARCH.md` §5 already names (random eligible wallet, liquidity filter, realized-profit ranking) and
a no-trade baseline. *Justified by §2.*

**C11. Metric discipline.** Anywhere a classifier-shaped number is reported, report MCC and AUCPRC
and state the base rate. Never report accuracy or F1 alone. Add a test asserting the base rate is
emitted alongside. *Justified by §5.*

**C12. Temporal-split enforcement.** Any fitted component must use rolling forward-window validation
with the final window held for one-time evaluation, and must report performance in at least two
distinct market regimes. §9 shows the population changed composition within twelve months; a model
that cannot show cross-regime survival has not been validated. *Justified by §5, §9.*

### P3 — Not now, but record the gap

- **Follower-feedback modelling.** §8 establishes that acquiring followers changes leader behaviour.
  Platform-wide per-mint exposure caps are already listed as missing in `ARCHITECTURE.md`; add the
  behavioural channel to that entry, not just the price-impact one.
- **Leave-one-out fragility.** `RESEARCH.md` §5 lists this as scaffolding-only. Best-token and
  best-day removal remain unimplemented; §1's finding that profitable traders concentrate in few
  instruments makes this more important, not less, because concentration is both a skill marker and
  a fragility marker.

---

## Part IV — Claims that must not be made

Derived from the sources above, for `README.md` and any user-facing copy:

1. Do not describe any implemented signal as predicting profit. The only clean out-of-sample test of
   top-wallet copying put it **below** the economic breakeven (§2).
2. Do not present the 56.1% figure, or anything like it, as a return. It is a simulated **loss
   reduction** (§6).
3. Do not present rug detection as solved. The largest published study reports MCC ≈ 0.39 and its
   own authors call it insufficient for deployment (§5).
4. Do not cite platform-wide profitability percentages as evidence the strategy works. The 2026 rise
   is attributed to the losing cohort leaving (§9).
5. Do not treat a wallet's leaderboard position as evidence it is an agent making decisions. Some
   top-PnL wallets are sell-only consolidation addresses (§2).

---

## Source list

- Barber, Lee, Liu, Odean (2014), *The Cross-Section of Speculator Skill*, Journal of Financial
  Markets 18, 1–24.
- Barber, Lee, Liu, Odean, *Do Day Traders Rationally Learn About Their Ability?*
- Marino, Naviglio, Tarantelli, Lillo (2026), *Predicting the success of new crypto-tokens: the
  Pump.fun case*, arXiv:2602.14860.
- Li, Kuznetsov, Yanovich, Nott-Whaley, Vodolazov (2026), *Catching the Rug: Early Prediction of
  Fraudulent Memecoins on Solana via Machine Learning*, arXiv:2608.20271.
- Hu, Tekin, Xu, Liu (2026), *MemeTrans / MELT*, arXiv:2602.13480.
- *SolRugDetector*, arXiv:2603.24625.
- Yaremus, Li, Kalacheva, Vodolazov, Yanovich (2025), *Detecting Rug Pulls in Decentralized
  Exchanges: ML Evidence from the TON Blockchain*, arXiv:2509.01168.
- Pine Analytics (2025), deployer-funded sniper report.
- *Quantifying the Threat of Sandwiching MEV on Jito*, ACM Internet Measurement Conference 2025.
- Chang, Datskos / Ghostlogs (2025), *The State of Solana MEV*, Accelerate 2025.
- Apesteguia, Oechssler, Weidenholzer (2020), *Copy Trading*, Management Science 66(12), 5608–5622.
- Pelster, Hofmann (2018), *About the fear of reputational loss: Social trading and the disposition
  effect*, Journal of Banking & Finance.
- IOSCO (2025), FR/06/2025 *Online Imitative Trading Practices: Copy Trading, Mirror Trading, Social
  Trading*.
- CoinGecko Research / Dune Analytics, Pump.fun trader profitability series, 2024–2026.
