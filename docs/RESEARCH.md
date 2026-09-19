# Integration and product research

Reviewed 2026-09-18. Implementation status updated 2026-09-18. Distinguish verified primary-source capabilities, this implementation, and proposed work. This is engineering/product research, not evidence of an investable strategy.

## 1. The feasible version of the idea

The clarified constraint—avoid paid data APIs, allow supported APIs, and let users pay transaction fees—is technically coherent. The original zero-API/zero-transaction-cost version is not. Account linking cannot substitute for a market feed, a transaction transport, or signing authority.

There are three separate systems:

1. **Observation:** indexed finalized/confirmed transactions, historical trades, token metadata, liquidity and ownership state.
2. **Decision:** wallet accounting, eligibility checks, anomaly detection and per-user risk limits.
3. **Execution:** authorization to sign, order construction, simulation, broadcasting, confirmation, expiry handling and reconciliation.

A supported platform can provide several of these. A linked public address alone provides none of the permission to spend from it. A normal browser wallet connection does not grant unattended server-side signing.

## 2. Platform findings

| Target | Verified primary-source material | What remains unknown or missing |
| --- | --- | --- |
| Axiom | Official docs include wallet tracking and Trader Scan. The published documentation index covers the consumer trading workflow. | A public, officially supported delegated account-trading/OAuth flow was not established in the docs reviewed. This does not prove private partner access does not exist. |
| Fomo | Official site advertises a social feed, leaderboards, trader alerts, and web/mobile trading. | The official public material reviewed did not establish third-party account-linking credentials or delegated trading scopes. “Gasless” UX is not evidence of a free execution API for this application. |
| Pump.fun | Official `pump-fun/pump-public-docs` contains program documentation, IDLs, and SDK references. | Onchain protocol instructions are not an OAuth integration with a Pump website account. Trading needs a signer, transaction transport, accurate current program accounts, and transaction fees. |
| Solana | RPC methods provide signature history and transaction details; fees are paid by a transaction fee payer. | A shared free RPC is not a reliable global, low-latency archival indexer. An authenticated website account does not make historical indexing free. |

Sources: [Axiom documentation index](https://docs.axiom.trade/llms.txt), [Axiom Trader Scan](https://docs.axiom.trade/trader-scan), [Axiom wallet monitoring](https://docs.axiom.trade/wallet-tracking/monitor-wallets), [Fomo official site](https://fomo.family/), [Pump official public docs](https://github.com/pump-fun/pump-public-docs).

Third-party sites and reverse-engineered libraries claiming Axiom/Fomo access were not treated as official integration contracts. No credential scraping, browser-cookie replay or private endpoint emulation was implemented.

## 3. Fees and data costs

Solana transactions have a mandatory base fee and may include priority fees. Someone pays these even when an application abstracts them away. Keeping those costs user-funded is possible; removing them through account linking is not. [Solana fees](https://solana.com/docs/core/fees)

Shared public endpoints are rate limited and officially described as unsuitable for production applications. A free endpoint is useful for a prototype and small historical samples. It cannot support a promise to observe every large meme-coin trade globally, reconstruct every wallet’s full history and execute for thousands of followers at low latency. [Solana RPC guidance](https://solana.com/docs/references/clusters)

The implemented manual importer uses `getSignaturesForAddress` followed by `getTransaction`, requests finalized transactions, and processes at most 20 signatures per page. It spaces requests, retries HTTP 429 responses, restricts concurrent collections, and archives raw responses. Missing provider history and unsupported transactions are not turned into fabricated observations. [Signature history](https://solana.com/docs/rpc/http/getsignaturesforaddress), [Transaction retrieval](https://solana.com/docs/rpc/http/gettransaction)

Finalized historical collection is deliberately unsuitable for earliest-possible copy entry. For production, choose an authorized platform stream or a dedicated node/indexer, measure the latency, and budget infrastructure. “No paid data API” does not mean “no infrastructure expense.”

## 4. What should be analyzed

A large buy is an observation, not evidence that the buyer is skilled. Recommended evidence layers:

| Layer | Questions | Status here |
| --- | --- | --- |
| Accounting | Were acquisition costs observed? Are fees included? Are partial exits matched correctly? | FIFO implemented for supplied observations; unmatched sells excluded. Round-trip results grouped by acquisition lot are now reported beside sell-event results, because the sell-event win rate is mechanically inflated by scaling out. |
| Repeatability | Is profit concentrated in one sell? How many independent tokens and market regimes were traded? | Largest-win share and sell counts implemented; independent-token/regime evaluation still needed. |
| Sample uncertainty | Is a high win rate based on a small sample? | Wilson interval implemented, with an independence caveat. |
| Copyability | Could a follower buy at a later slot, then exit at available liquidity after costs? | **Implemented.** Costs are fixed-plus-proportional, and each row carries a worst-case bound marking unresolved exits at total loss plus a resolution rate. Follower-adjusted P&L across a 0/2/5/15/60-second delay ladder, at the user's own order size, with costs charged both sides and sizing capped by supplied liquidity. Unfilled entries, unresolved exits and unknown liquidity are counted separately rather than assumed away. Still missing: pool price impact, queue competition, failed transactions and partial fills, so results stay optimistic. |
| Inventory | Is this a partial exit? Does the leader still hold meaningful exposure elsewhere? | Wallet events observed; current paper policy still closes fully on any leader sell. Proportional mirroring of partial exits and cross-wallet consolidated inventory remain unimplemented. |
| Funding and coordination | Was the wallet funded by a deployer? Did linked wallets accumulate before launch? | **Partially implemented.** Native SOL funding edges are extracted from archived transactions, giving funders, co-funded wallet sets, and funders that traded a token before this wallet bought it. Coverage is bounded by what RPC collection archived, and an unindexed wallet is reported as *unknown*, not clean. Still a lead, not proof of identity or misconduct: funding is surfaced for review and never gates entry. |
| Token state | Can authority permissions change? Is supply concentrated? Are executable reserves deteriorating? | **Partially implemented.** A cached pre-entry gate checks the rugged flag, mint and freeze authority, metadata mutability, worst-market LP lock and an upstream risk score; an unmentioned authority reads as present, and an uncollected token is unknown rather than safe. Holder concentration is reported but deliberately not gated, because pool accounts legitimately dominate a young supply. Executable-reserve deterioration under our own order size is still not measured. |
| Crowding | Do many followers enter after the leader? Does the leader repeatedly sell into their buying? | Not implemented; requires multi-wallet temporal and price-impact data. Platform-wide per-mint exposure caps are also still missing, so a large follower base would become its own price impact. |
| Entry timing | Does the wallet reach tokens before any follower could? | **Implemented.** Early-entry rate against each token's first observed trade, gating entry above a configured share. The reference point is our first observation rather than true creation, so the measure is conservative: a low rate is weak evidence, a high rate is strong evidence. |
| Exit control | Can a position be closed without the leader acting? | **Implemented.** Tick-evaluated hard, trailing and time stops run against an independent observed price path, so a leader who stops trading no longer leaves a follower holding. Ticks are observations, not executable quotes, and no price impact for our own size is modeled. |
| Discovery | Where do candidate wallets come from at all? | **Implemented.** A public firehose collector records wallets trading at or above a configured size, deduplicated by signature and pruned weekly. Size is not skill: candidates gain no eligibility until history is collected and the normal gates pass. |
| Automation | Is this flow a bot or a person? | **Partially implemented.** Every decoded observation records whether the venue was called directly or reached through an aggregator, and the per-wallet routed share is reported with a flag above 90%. This is the published proxy for bot-attributed flow, not a bot classifier, and the firehose source does not report it at all. |
| Reliability | Are events fresh, final, deduplicated and replayable? | Import IDs, event freshness checks, raw snapshots and transactionally deduplicated paper positions implemented. Global reorg handling not implemented. |

## 5. Early-warning research design

These are hypotheses to test, not capabilities established by the current score. Two of them now
have measurement scaffolding, which is not the same as validation:

- **Copy-farming pattern:** repeated lead buy → dense follower buy window → leader exit, with follower realized losses. Compare against matched control wallets and normal high-volume behavior. *Scaffolding:* the funding graph and follower-adjusted P&L exist; the dense-follower-window detector and the matched control comparison do not.
- **Abnormal sizing:** a buy far outside the wallet’s historical size distribution. Normalize by available wallet capital, executable liquidity and token age, rather than an arbitrary absolute SOL threshold.
- **Liquidity deterioration:** reserve decline and worse executable quotes under the user’s actual order size. Do not confuse virtual bonding-curve reserves with immediately sellable liquidity.
- **Behavior drift:** historically patient wallets switching to very short holds, correlated entries or increased drawdown. Measure change against a rolling baseline without future information.
- **Profit fragility:** edge disappears after removing the best token or best day, including failed transactions, or applying conservative follower delay. *Scaffolding:* the conservative-delay half is implemented as the alpha-decay ladder. Leave-one-out over best token and best day is not.

Validation procedure:

1. Retain immutable raw observations with slot, transaction index, instruction index, observation time, block time, source and commitment.
2. Reconstruct inventory and transfer provenance; label missing history explicitly.
3. Define outcomes at a predetermined follower delay and maximum order size. Distinguish source-wallet P&L from achievable follower P&L.
4. Split chronologically. Keep related tokens and wallet clusters from contaminating train/test boundaries. Include dead tokens and failed wallets to reduce survivorship bias.
5. Compare against simple baselines: random eligible wallet, liquidity filter, realized-profit ranking and a no-trade baseline.
6. Report out-of-sample net expectancy, drawdown, turnover, fill rate, time in market, uncertainty and capacity at different follower counts. Include fee/slippage/latency sensitivity.
7. Run a prospective paper/shadow period with frozen rules. A retrospective leaderboard is not validation.

The delay ladder is the closest thing here to a real finding: a wallet profitable at zero delay and
unprofitable at sixty seconds has an edge that belongs to whoever is fastest. That is a measurement,
not a validated selection rule, and it has not been run prospectively.

Do not label the current 0–100 score a prediction confidence. It is a transparent, uncalibrated ranking heuristic. Wilson intervals are descriptive under an independent-Bernoulli approximation; correlated trades and split exits violate that assumption. A production statistical evaluation should cluster by token/day/wallet.

## 6. Why execution is not just “copy when they sell”

The leader’s transaction may already have changed the pool before the service receives it. Followers compete for liquidity. Exits can fail or become uneconomic. Submitted transactions can expire or remain unconfirmed; a retry must not create a second economic order. An exit request must reconcile the actual position and actual chain result. [Solana transaction confirmation and expiration](https://solana.com/developers/cookbook/transactions/confirmation)

A live adapter needs narrowly scoped, revocable authority; per-order and daily spend limits; a user-funded fee budget; simulation; bounded slippage; idempotent intent storage; transaction-status reconciliation; a pause control; and a position-reconciliation job. These are concrete missing execution components, not something a login button can supply.

## 7. Commercial assessment

Basic whale alerts and wallet history face existing competition: Axiom already documents Trader Scan and wallet monitoring; Fomo advertises trader alerts. The plausible differentiator is **measurable copyability after costs**, transparent exclusion reasons, and a trustworthy execution record—not simply a larger collection of wallet statistics. This is an inference from the existing feature sets, not established customer demand.

The first two now exist in code: `/wallets/{address}/copyability` reports follower P&L beside leader P&L across a delay ladder, and every refused paper entry returns a named reason. Whether either changes a customer's behavior is still the open question, and building them does not answer it. The validation step below is unchanged.

Before scaling live execution, recruit a small paying cohort to an analysis/watchlist product and ask what they use today, what data they distrust, and whether a measured copyability report changes their behavior. Record retention and actual renewal; avoid treating trading volume or free signups as profit.

Illustrative economics, not forecasts: 100 subscribers at €49/month generate €4,900 gross monthly subscription revenue. Deduct infrastructure, payment fees, support, refunds, acquisition, taxes and engineering time. User-funded network fees do not cover the service’s data and support costs. Do not promise profitable trades to sell the subscription.

## 7b. How the platform is paid

Access can be sold by subscription or funded by commission (`ACCESS_MODEL`). The commission default
is a performance fee above a per-account high-water mark, not a per-trade cut.

That choice follows directly from `EVIDENCE.md`: §2 shows copying top wallets sits below economic
breakeven, so a fee proportional to turnover deepens the shortfall, and §8 records IOSCO
FR/06/2025 flagging remuneration structures that pay the intermediary regardless of copier outcome.
A per-trade cut would earn most from the users doing worst. Per-trade and hybrid models exist in
the code for operators who choose them, and each states its own conflict in the API and UI.

Commission is collected at withdrawal, which is the reason the managed wallet is custodial: a user
who realizes profit and leaves cannot otherwise be billed. That convenience carries a licensing
obligation — see the custody section of `README.md` — and the master-key-in-environment design is a
known gap pending a KMS.

## 8. Go-live dependencies

- Obtain official partner documentation/access for Axiom or Fomo, or choose direct user-authorized onchain execution as a separate product decision.
- Establish historical coverage and a reliable live stream, with a known latency/cost budget. The discovery collector is a free prototype feed, not a complete or guaranteed one: a dropped connection loses observations, and no gap is recorded as an absence of trading.
- Build and validate protocol decoders against real transactions across supported versions, migrations and quote assets.
- Measure follower outcomes prospectively. The copyability simulation is retrospective and optimistic; no predictive-performance claim is justified by this build.
- Replace the quadratic replay with incremental per-wallet state before any dataset larger than the fixtures.
- Integrate actual payment verification and subscription lifecycle events, or, under the commission
  model, a treasury sweep: commission is currently accrued and withheld from the withdrawable
  balance but is not swept to a platform address.
- Replace the environment-variable custody master key with a KMS or HSM before holding real funds.
- Work the outstanding `EVIDENCE.md` items: C4 score rebuild (blocked on C10's harness, since C4
  requires a fitted and held-out formula rather than a hand-tuned one), C10 split-half evaluation,
  C11 metric discipline and C12 temporal splits. C5 (liquidity velocity), C6 (bot share),
  C7 (pre-graduation exit), C8 (deployer→sniper gate) and C9 (capacity) have landed.
- Resolve pool liquidity on multi-hop routes. The counterparty heuristic is ambiguous there, so most
  routed swaps still record unknown liquidity and cannot be copied.
- Complete operational/security work described in `ARCHITECTURE.md` before accepting public traffic or real trading authority.
