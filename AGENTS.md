# AGENTS.md

Instructions for AI agents working in this repository. Read this before writing code.

## What this project is

A **Solana memecoin trading platform**. Swaps route through Jupiter and are signed by the user's own
wallet; copy trading and wallet intelligence are the differentiating layer on top, not the product.
Rust/Axum backend, React/TypeScript frontend, SQLite WAL persistence.

**Copy trading can be switched off platform-wide** from the admin panel, and when it is, every path
refuses — including the live collector, which reaches `fanout` without a route. Enforce it in the
function, never only at the edges.

**Swaps are real and non-custodial.** `swap.rs` builds an unsigned transaction and the user's wallet
signs it; we never hold that key. **Copy trading is still paper-only** — the paper engine does not
place swaps, and `live_available` is `false`. Do not wire the copy engine to `swap.rs` without an
explicit, separate request: automated trading on someone's behalf is a different product and a
different legal position from a user pressing buy.

**It does move funds, in exactly one way.** When `CUSTODY_MASTER_KEY` is set the service holds a
Solana private key per account and can send a withdrawal. That single path lives in `transfer.rs`.
Treat everything it touches as production money code.

The product thesis is *measurable copyability after costs*: anyone can rank wallets by profit, and
Axiom, Solana Tracker and Kolscan already do. The differentiator is answering "what would **I** have
made copying this wallet, after fees, at my order size, at my latency" — plus refusing to copy
wallets whose profit comes from insider position, coordination, or MEV rather than skill.

## Non-negotiable rules

1. **Never fabricate an observation.** If data is missing, unavailable, or ambiguous, record it as
   missing. Never substitute a default, a zero cost basis, or an interpolated price. The FIFO
   accountant excludes unmatched sells from P&L rather than treating them as free acquisition — every
   new code path must be equally conservative.
2. **Unavailable is not zero.** A balance, risk report or price the provider could not return is
   reported as unavailable with the reason. Rendering it as zero is a lie about someone's money or
   safety, and it is the specific mistake this codebase keeps catching in review.
3. **Never claim predictive power.** The 0–100 score is an uncalibrated ranking heuristic. Do not
   rename it to "confidence", "probability" or "win chance" in code, API responses, UI text or docs
   until a calibration record exists.
4. **Never ask a user for a secret.** No private keys, no exchange passwords, no exported browser
   session cookies from Axiom/Fomo/any platform. Note the asymmetry with custody: the service
   *generates* the keys it holds. A wallet sign-in proves control of an address and must never
   request a key, a transfer or an allowance.
5. **Money is integer lamports.** `fees.rs` and `transfer.rs` do not inherit the `f64` debt listed
   below. Anything that computes an amount owed, held or sent uses integers and saturating
   arithmetic.
6. **Never accept a quote, route or transaction from the client.** These fix amounts, routing and
   our fee. The server builds them from validated parameters. `swap.rs` is the reference.
7. **Anything that moves funds stays narrow.** `transfer.rs` expresses exactly one System transfer
   with one signer. Do not generalize it, do not add instruction-data parameters, and do not let
   another module construct transactions.
8. **A failed send is `unknown`, not `failed`.** Record the attempt before broadcasting and
   reconcile by signature. Never retry a withdrawal by rebuilding it: a fresh blockhash is a second
   payment.
9. **Never build on reverse-engineered platform APIs.** Axiom and Fomo have no official public
   developer API. Third-party clients that replay their private endpoints are out of scope.
10. **Server-side secrets stay server-side.** Provider URLs and API keys are read from the
   environment and must never be accepted as request fields (SSRF). `sync_rpc` is the reference.
11. **Admin-only for anything that ingests.** Ingestion, import and collection routes require an
   administrator *session*; a personal API key is never sufficient.
12. **Keep the honesty in the docs.** `README.md` and `docs/RESEARCH.md` state limits plainly. If
   you implement something they list as missing, move it — do not delete the caveat and do not
   overstate what landed.

## Layout

| Path | Contents |
| --- | --- |
| `backend/src/main.rs` | Routes, access control, schema, persistence, paper policy, event fanout |
| `backend/src/analytics.rs` | Pure wallet accounting: FIFO, descriptive statistics, copyability |
| `backend/src/rpc.rs` | Optional Solana RPC collection, conservative swap decoding |
| `backend/src/discovery.rs` | Candidate-wallet discovery from public trade firehoses |
| `backend/src/risk_token.rs` | Token-level safety gate (authorities, LP, concentration) |
| `backend/src/cluster.rs` | Funding-graph clustering, snipe-rate, authenticity scoring |
| `backend/src/custody.rs` | Managed wallet generation, sealing at rest, one-shot release |
| `backend/src/wallet_auth.rs` | Phantom/MetaMask sign-in message, signature verification |
| `backend/src/fees.rs` | Commission models and the shared trading-cost function |
| `backend/src/transfer.rs` | The only code that moves real funds: one System transfer |
| `backend/src/ticks.rs` | Independent price ticks and tick-evaluated exits |
| `backend/src/watch.rs` | Live copy feed: one `logsSubscribe` per followed wallet |
| `backend/src/swap.rs` | Non-custodial swap execution through Jupiter |
| `backend/src/social.rs` | Token X identity, handle reuse and rename detection |
| `backend/src/market.rs` | Cached price, depth, turnover and links per token |
| `backend/src/chart.rs` | OHLCV candles for the deepest pool |
| `backend/src/holdings.rs` | On-chain balances and largest token accounts |
| `backend/src/velocity.rs` | Liquidity velocity and carrying capacity |
| `backend/src/tests.rs` | Route-level integration tests against in-memory SQLite |
| `frontend/src/main.tsx` | Entire UI: routes, views, forms, charts, API calls |
| `frontend/src/styles.css` | Hand-written responsive CSS. No UI kit, no CDN fonts |
| `frontend/tests/` | Playwright user workflows |
| `docs/` | `EVIDENCE.md`, `RESEARCH.md`, `METHODOLOGY.md`, `ARCHITECTURE.md`, `API.md` |
| `fixtures/demo-trades.json` | 360 deterministic synthetic events. Not real performance |

## Code conventions

**Rust.** Edition 2024. The existing style is deliberately dense: long SQL on one line, `?`
propagation everywhere, no intermediate variables that are used once. Match it.

- Errors: return `ApiError(StatusCode, String)`. Use `bad("…")` for 400. Messages are user-facing
  sentences, not debug output. Database errors are logged and replaced with a generic message.
- Every new table goes in the `execute_batch` in `setup()` with `CREATE TABLE IF NOT EXISTS`. There
  is no migration framework; additive-only changes keep existing databases working.
- Anything touching money or positions runs inside a `db.transaction()`. Event handling must be
  idempotent — `INSERT OR IGNORE` on a natural key, and check the affected-row count.
- Pure logic goes in its own module with inline `#[cfg(test)] mod tests`. Route behavior is tested in
  `tests.rs`. Both are expected for a new feature.
- Comments explain *why a limit exists*, not what the line does. See the header of `rpc.rs`.

**Outbound HTTP.** Build a `reqwest::Client` with an explicit timeout, retry `429` with bounded
backoff, space requests, and never let a provider failure become a fabricated success. Copy the
`call()` helper in `rpc.rs`.

**TypeScript.** One file. Match its existing component and fetch patterns. No new dependencies
without a reason that survives `npm audit`.

`@solana/web3.js` is the one runtime dependency beyond React. It is needed to deserialize the
versioned transaction Jupiter returns so the wallet can sign it. It pulls a transitive `stream-json`
advisory through `jayson`, its RPC client; we never construct a `Connection`, and the shipped bundle
contains no `stream-json`, which was verified by grepping `dist`. `npm audit fix --force` would
"fix" this by downgrading web3.js to 0.0.3 and must not be run. A `uuid` override pins the patched
line of the other advisory.

**Charts follow the `dataviz` skill.** Load it before writing any chart code. The up/down candle
pair is validated per theme with `scripts/validate_palette.js` against that theme's surface — Void
passes every check at CVD ΔE 22.1. A legend is always present so colour alone never carries
identity, the hover crosshair and readout are standard rather than optional, and grid and axes stay
recessive. Signal's lime sits above the dark lightness band; that is a known property of the legacy
palette, kept deliberately.

**Colour is never hardcoded.** Every colour resolves to one of the nineteen tokens defined on
`:root` in `styles.css`; a theme is a redefinition of those. This includes inline SVG fills and
strokes in `main.tsx`, which the CSS sweep cannot reach. Corners are square everywhere by a global
reset — do not add a `border-radius`.

## Validation — run before reporting work complete

```bash
cargo test --manifest-path backend/Cargo.toml
cargo fmt --check --manifest-path backend/Cargo.toml
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
cd frontend && npm run build && npx playwright test
```

`make check` covers fmt, clippy and the frontend build. Clippy runs with `-D warnings`; warnings are
build failures here.

## Read the evidence before changing an algorithm

`docs/EVIDENCE.md` records what the published literature establishes, and Part III is a prioritised
change list with acceptance criteria. Two rules from it bind directly:

- **Part IV lists claims that must not be made.** The only clean out-of-sample test of copying top
  wallets landed *below* breakeven. Do not describe any signal as predicting profit.
- **C4 forbids hand-tuning the score.** Any new scoring formula must be fitted and held out. The
  round-trip win rate was substituted into the existing formula because C1 instructs it; the weights
  themselves must not be adjusted by eye.

Landed: C1, C2, C3, C5 (liquidity velocity), C6 (bot share), C7 (pre-graduation exit), C8
(deployer→sniper gate, opt-in), C9 (capacity).

Outstanding: C10 (split-half harness), C11 (metric discipline), C12 (temporal splits), and C4
(score rebuild) — which is **blocked by C4's own wording**: the formula must be fitted and held
out, so C10 has to exist first. Do not hand-tune the weights to close it.

## Test against real data, not only fixtures

Two bugs in this repo survived a full synthetic test suite and were caught only by live mainnet
transactions:

- `Trade::validate()` capped every string at 128 characters, but a real
  `source:signature:wallet` id is ~137. **Every genuine RPC observation was silently discarded**
  while every short-fixture test passed.
- The decoder required a direct top-level Pump call, which rejected the majority of real volume
  because it routes through aggregators.

`fixtures/routed-swap.json` is a real captured mainnet swap, kept as a regression test. When you
change the decoder or the trade schema, run it against real data before believing the suite.

**A mock that is kinder than the server hides the bug it was meant to find.** The browser test for
the top search mocked `/api/search` and supplied a `handles` key that the real address branch did
not return. The suite stayed green while pasting an address unmounted the entire application. When
a browser test asserts something about a response shape, route that case to the real endpoint
(`route.fallback()`) instead of inventing the reply.

Two rules follow, and both are cheap:

- **Every branch of a handler returns every key**, with empty arrays rather than omissions. There
  is a contract test for `/search`; extend that pattern when a handler grows a second branch.
- **The interface never assumes a key exists.** Index defensively (`(x.items||[])`), and remember
  that a blank page tells the user nothing and tells us nothing either. A `Boundary` around the
  app turns a render failure into a named, recoverable message; do not remove it.

## Known debt — do not be surprised by these

- `f64` is used for research metrics and paper P&L. Fees and transfers use integers. Production accounting needs fixed-point arithmetic keyed to mint decimals.
- Historical replay recomputes prior wallet history per event and is **quadratic** in history length.
  Fine for fixtures, unusable at scale. Replace with incremental per-wallet state before scaling.
- Ordering is `(timestamp, id)`. Real ingestion needs slot, transaction index and instruction index;
  same-second ordering is currently undefined.
- Wilson intervals assume independent Bernoulli trials. Correlated trades and split exits violate
  this. Cluster by token/day before presenting any interval as rigorous.
- One process, one mutex-protected SQLite connection. Correct, not concurrent.

## External data sources

Free or free-tier, no reverse engineering. Configure via environment; never hardcode keys.

| Source | Use | Notes |
| --- | --- | --- |
| Solana JSON-RPC | Transaction history, funding graph, balances, transfers | Shared endpoint is rate-limited, prototype only |
| PumpPortal WebSocket | Pump.fun/PumpSwap trade firehose | `subscribeNewToken` is free. **`subscribeTokenTrade` needs an API key on a wallet funded with ≥0.02 SOL** — verified against the live feed, and it refuses via a `{"message": …}` frame, not an error |
| RugCheck | Token authorities, LP status, concentration | Free, no key required |
| Solana Tracker | Leaderboards, wallet PnL, whale/KOL rooms | 2,500 requests/month free |

Treat every third-party number as **untrusted input**: validate it, store its provenance, and
re-derive anything that matters with our own FIFO engine before showing it to a user.

## Definition of done

A change is complete when it has: inline unit tests for pure logic, a route test if it adds a route,
a passing `make check`, an `API.md` entry if it changes the contract, a `METHODOLOGY.md` entry if it
changes a number a user sees, and an honest `README.md` status line. Moving an item out of the
"missing" list in `RESEARCH.md` requires the implementation to actually cover it — partial coverage
stays listed with the gap named.
