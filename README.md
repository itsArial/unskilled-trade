# Unskilled Trade

A local-first wallet intelligence workspace with a **Rust/Axum backend** and a **React/TypeScript frontend**. Dark responsive interface, persistent account management, wallet analysis, API keys, paper strategy execution, and administration.

**This is a working research/paper-trading application, not a completed live copy-trading service.** Axiom/Fomo account authorization, live order signing, a global market feed, calibrated predictions, and payment checkout are not connected. Wallet discovery, token safety checks, follower-adjusted copyability, tick-evaluated exits and authenticity checks are implemented against free public sources; none of them establishes that copying is profitable, only whether it is measurable. The UI and API expose those limits explicitly. No real funds are moved. No paid data provider is required.

## Run

Requirements: a current Rust toolchain, Node.js 22+, npm, Python 3 (only for the optional bootstrap helper).

```bash
python3 scripts/setup.py
cd frontend
npm ci --include=dev
npm run build
cd ../backend
cargo run --release
```

Open **http://127.0.0.1:8080**. The Rust server also serves the built frontend.

A `.env` has already been created in this workspace with a random administrator password and restrictive file permissions. Sign in as `admin@unskilled.local`; read `ADMIN_PASSWORD` in `.env`. No shared/default production password is embedded in code. `.env` is ignored by Git. On a fresh checkout, `scripts/setup.py` creates it without printing the password.

The administrator is created only if its email is absent from the database. Changing `.env` does not silently reset an existing account password or elevate an existing member. Changing the email to a previously unused address creates a new administrator on restart. Password recovery is not yet implemented.

The database starts empty. The public preview uses clearly labeled synthetic data. To exercise the complete workflow:

1. Sign in as administrator and open **Administration**.
2. Click **Load synthetic demo history**, or import your own normalized JSON.
3. Open **Smart wallets**, inspect a dossier, and follow a wallet.
4. Open **Copy trading**, enable paper execution, and select **Replay history**.
5. Inspect paper positions, P&L, exit reasons, and audit logs.
6. Register a separate member account. An administrator can grant complimentary access or record independently verified payment. Members cannot self-activate paid access.

Demo data remains visibly labeled after import. Use a separate `DATABASE_PATH` for real data. Do not mix sample events into a production dataset.

For frontend development, run `cargo run` from `backend` and `npm run dev` from `frontend` in separate terminals. Vite proxies `/api` to port 8080. No external font or image requests are needed.

## Implemented

| Area | Behavior |
| --- | --- |
| Accounts | Registration, Argon2 password hashing, expiring HttpOnly sessions, same-site cookies, login throttling, logout |
| Wallet sign-in | Phantom (Solana, ed25519) and MetaMask (Ethereum, EIP-191). Domain-bound single-use nonce consumed by the verifying statement; the signed message states it authorizes no transaction |
| Managed wallets | Every account gets a Solana wallet. Keys are XChaCha20-Poly1305 sealed at rest, never logged or listed, and **exportable by the owner at any time** — the model Axiom and Trojan use. An export is a copy and says so; closing the account hands the key over and destroys our ciphertext |
| Funds | Deposit address, live balance, withdrawal to any Solana address as a single System transfer, reserves withheld for rent exemption and network fee, attempt recorded before broadcast |
| Commission | Performance fee on new profit above the account's own high-water mark, netted across all closed positions, integer lamports, idempotent per epoch; per-trade and hybrid models available and labelled with their conflict |
| Administration | Bootstrap admin from `.env`, member bans, immediate session revocation, audit log, plan creation/update/deactivation |
| Subscription records | Active-until checks, unique payment references, transactional grants, complimentary access excluded from revenue |
| Wallet history | Validated normalized imports, deduplication, raw transaction archive for optional RPC collection |
| Analytics | FIFO realized P&L including supplied fees, round-trip win rate grouped by acquisition lot, matched-sell win rate, Wilson interval, profit factor, realized drawdown, holding times, largest-win concentration, losing streak, complete event history |
| Copyability | Follower-adjusted P&L at a configurable delay ladder, per-user order size, fixed-plus-proportional costs both sides, liquidity-capped sizing, worst-case bound marking unresolved exits at total loss, resolution rate, explicit unfilled/unknown-liquidity counts, entry slippage, alpha-decay flags |
| Authenticity | Early-entry rate against each token's first observed trade, funding-graph edges from archived transactions, co-funded wallet sets, funder-traded-first detection; timing gates entry, funding is reported for review only |
| API access | One rotating, hashed API key per user, 30-day expiry, detailed analysis gated by subscription, admin access denied to API keys |
| Paper execution | Watchlists, minimum history/score/liquidity gates, token safety gate, allocation and exposure caps, realized daily loss guard, leader-sell/price-drop/liquidity exit triggers, named refusal reasons for every rejected entry |
| Token safety | Cached pre-entry assessment: rugged flag, mint/freeze authority, metadata mutability, worst-market LP lock, upstream risk score, named risks; unknown is never treated as safe |
| Tick exits | Independent price path; hard stop from entry, trailing stop from the best price observed while held, time stop; idempotent, never opens a position, never rewrites history; fed by admin ingestion and the discovery collector |
| Event distribution | Admin-only normalized recent-event ingestion distributes paper decisions to enabled, subscribed, unbanned followers transactionally; repeated IDs do not duplicate orders |
| Historical replay | Explicit action, only earlier events used for selection, per-user event deduplication and chronological watermark |
| Live paper runs | Optional: firehose trades become analyzable history and drive paper positions on real market data. Curve liquidity derived as `virtual − 30 SOL`; migrated pools record unknown. Nothing is signed |
| Discovery | Optional public-firehose collector: tracks new mints, records wallets trading at or above a configured size, deduplicates by signature, prunes after seven days; admin review and promotion |
| Live copy feed | One `logsSubscribe` per followed wallet against any Solana RPC websocket; volume scales with the follow list, not the market, so it fits a free tier. Subscriptions reconcile as users follow and unfollow; reverted transactions never become history |
| Swap decoding | Direct and aggregator-routed Pump/PumpSwap swaps, measured as the wallet's net position change; pool liquidity inferred from the counterparty's balance where unambiguous; direct-vs-routed recorded per trade and reported as a per-wallet share |
| Archive replay | `/admin/rpc/redecode` re-runs the current decoder over everything archived, so a better decoder recovers history it previously could not read |
| Search | One box for coins, wallets and X handles; a pasted address resolves straight to its page. It is the only address field, so Discover opens on the market rather than on a form |
| Launch list | Repeat mints of one name by one deployer collapse to the newest, carrying the count. Twelve identical mints from a single address inside forty seconds is the ordinary case on this stream, and the count is the signal |
| Trading | Non-custodial buy and sell through Jupiter; the server builds an unsigned transaction and the user's wallet signs it. Quote shows route, worst-case fill, price impact and fee before committing |
| Token page | Candlestick chart from the deepest pool, market stats, holders, traders observed on that token, the X account behind it and its reuse history, and the cached safety verdict |
| Portfolio | On-chain balances across both token programs, priced where a price exists; unpriced holdings are counted, never valued at zero |
| Feed | Trades by followed wallets, with your own positions marked to the last observed tick as unrealized |
| RPC collection | Optional manual paginated fetch from configurable Solana RPC; raw finalized transaction archival; conservative direct Pump/PumpSwap native-SOL observations |
| Interface | Desktop/mobile navigation, wallet dossiers with copyability and authenticity panels, chart ranges, search/sort/filter, strategy controls, connections, API access, billing and administration |

All locally configured subscription plans currently unlock the same access class. Names/prices are editable; differentiated quotas and entitlements are future work. Revenue is the sum of administrator-recorded verified payments, not bank reconciliation or automatic collection.

## Configuration

See [.env.example](.env.example). Paths are relative to the backend working directory when following the run instructions.

- `ADMIN_EMAIL`, `ADMIN_PASSWORD`: bootstrap administrator.
- `DATABASE_PATH`: SQLite file, default `unskilled.db`.
- `BIND_ADDRESS`: default `127.0.0.1:8080`.
- `COOKIE_SECURE`: set `true` behind HTTPS; local HTTP uses `false`.
- `FRONTEND_DIST`: default `../frontend/dist`.
- `SOLANA_RPC_URL`: default free shared mainnet endpoint. Used only on explicit administrator collection requests.
- `DISCOVERY_ENABLED`, `PUMPPORTAL_WS_URL`, `DISCOVERY_MIN_SOL`, `DISCOVERY_TRACKED_TOKENS`: candidate-wallet collector. Disabled by default. Discovered wallets are observations, not recommendations, and are not eligible for paper execution until history is collected.
- `RUGCHECK_BASE_URL`, `TOKEN_RISK_REQUIRED`, `TOKEN_RISK_MAX_SCORE`, `TOKEN_RISK_MIN_LP_LOCKED`, `TOKEN_RISK_MAX_AGE`: token safety gate. Set `TOKEN_RISK_REQUIRED=true` for real data so that a token with no collected assessment cannot be entered.
- `COPY_COST_BPS`, `COPY_MAX_WAIT_SECONDS`: follower simulation cost per side and the window after which an order counts as unfilled.
- `EXIT_HARD_STOP_PCT`, `EXIT_TRAILING_PCT`, `EXIT_TIME_STOP_SECONDS`: tick-evaluated exits, which fire on observed price alone without a leader event.
- `EARLY_ENTRY_WINDOW_SECONDS`, `MAX_EARLY_ENTRY_PCT`, `ENFORCE_AUTHENTICITY`: refuse to copy wallets whose entry timing a follower cannot reproduce.
- `CUSTODY_MASTER_KEY`, `WITHDRAWALS_ENABLED`: managed wallets. Unset disables custody; it never falls back to storing keys in the clear.
- `ACCESS_MODEL`, `FEE_MODEL`, `FEE_PERFORMANCE_BPS`, `FEE_TRADE_BPS`, `FEE_TRADE_FIXED_LAMPORTS`, `FEE_EPOCH_SECONDS`, `FEE_MIN_SETTLEMENT_LAMPORTS`: how the platform is paid.
- `COPY_COST_FIXED_LAMPORTS`, `COPY_FAILURE_RATE`: the fixed component of network cost, and the expected failed attempts per successful trade.
- `DISCOVERY_INGEST_TRADES`, `MIN_LIQUIDITY_SOL`, `EXIT_GRADUATION_PROGRESS`: run paper strategies against live real market data. The 100 SOL liquidity default is unreachable on a Pump.fun curve, which migrates near 85 SOL.
- `WATCH_ENABLED`, `SOLANA_WS_URL`, `WATCH_MAX_WALLETS`: the live copy feed. Leave `SOLANA_WS_URL` unset to derive it from `SOLANA_RPC_URL`.
- `PUMPPORTAL_API_KEY`: only needed for the trade firehose. Token creations are free; trade subscriptions are refused without a key on a wallet funded with at least 0.02 SOL.
- `APP_ORIGIN`: optional exact browser origin for a reverse-proxy/dev arrangement when it differs from the request Host. No wildcard CORS policy is installed.

## Validation

```bash
cargo test --manifest-path backend/Cargo.toml
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
cd frontend
npm run build
npx playwright install chromium
npx playwright test
```

Browser tests launch a separate backend on port 8081 with an in-memory test database. They exercise preview/mobile navigation, admin login, history ingestion, following, paper execution, API key rotation, plan management, and member access restrictions. Screenshots: [desktop](docs/screenshots/desktop.png), [mobile](docs/screenshots/mobile.png).

## Custody, in plain words

Peer platforms generate a wallet for the user and make it exportable — Axiom does this through
Turnkey with a seed phrase issued at signup. That is the model here too, with one difference stated
openly: our copy of the key persists after an export, so control is shared until the account is
closed or the funds are moved elsewhere. Turnkey-style enclave key management would remove that gap
and is the right destination.


If `CUSTODY_MASTER_KEY` is set, this service creates and holds a Solana private key for every
account. That makes it a **custodian of other people's money** — a licensable activity (CASP under
MiCA in the EU, money transmission in most US states), not an implementation detail. Nothing in this
repository discharges that obligation.

What the code does provide: keys are sealed with XChaCha20-Poly1305 and never stored, logged or
returned in the clear; the only operation that can move funds is a single System transfer to a
user-named address; and closing an account hands the key over once and destroys the stored
ciphertext, after which the service cannot sign for that wallet.

What it does not provide: the master key lives in the process environment, so anyone with host
access can decrypt every user's funds. Production needs an external KMS or HSM with per-operation
authorization. This is a real gap and it is not hidden.

## Read next

- [Instructions for AI agents working in this repository](AGENTS.md)
- [Published evidence and the resulting change list](docs/EVIDENCE.md)
- [Integration feasibility and research](docs/RESEARCH.md)
- [Analytics methodology and simulation limits](docs/METHODOLOGY.md)
- [API contract and event formats](docs/API.md)
- [Architecture and live-service work remaining](docs/ARCHITECTURE.md)

Neither source-wallet profits nor simulated results establish a profitable copying strategy. This implementation intentionally does not collect Axiom/Fomo passwords, browser session cookies, or wallet private keys.
