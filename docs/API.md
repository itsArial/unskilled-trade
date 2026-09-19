# API contract

Base URL: `http://127.0.0.1:8080/api`. JSON request/response bodies. Errors return a non-2xx HTTP status and `{"error":"message"}`; framework-level malformed JSON errors can be text. Maximum body size is 4 MiB.

Browser authentication uses the HttpOnly `session` cookie. Programmatic requests use `Authorization: Bearer YOUR_KEY`. The administrator routes require an administrator browser/session token; a personal API key is not sufficient, even if its owner is an administrator.

## Routes

| Method | Path | Access | Behavior |
| --- | --- | --- | --- |
| GET | `/health` | Public | Status and explicit `live_available: false` |
| POST | `/auth/register` | Public | `{email,password}`; passwords 12–128 characters |
| POST | `/auth/login` | Public | `{email,password}`; returns session cookie; account-based failure throttle |
| POST | `/auth/logout` | Signed in | Revokes browser sessions for the account |
| POST | `/auth/wallet/challenge` | Public | `{address,chain}`; chain is `solana` or `ethereum`. Returns `{nonce,message,expires_in}` |
| POST | `/auth/wallet/verify` | Public | `{address,chain,nonce,signature}`; verifies, signs in, provisions a managed wallet |
| GET | `/funds` | Signed in | Managed wallet address, live balance, commission owed, withdrawable amount, fee model |
| GET | `/fees` | Signed in | Fee schedule, cost model, high-water mark and the last 100 ledger rows |
| POST | `/funds/withdraw` | Signed in | `{destination,lamports}`; settles commission, then sends one System transfer |
| GET | `/funds/withdrawals` | Signed in | Last 50 withdrawal attempts with status and signature |
| POST | `/funds/export-key` | Signed in | `{confirm:"EXPORT"}`; returns a **copy** of the key. Custody continues |
| POST | `/account/close` | Signed in | `{confirm:"CLOSE"}`; releases the private key once and ends custody |
| GET | `/me` | Signed in | Identity, role, expiry, enabled flag and budget |
| GET | `/overview` | Signed in | Wallet summaries, recent events, private watchlist and private paper positions; detailed histories/curves omitted when unsubscribed |
| GET | `/wallets/{address}` | Active subscription | Full analytics and supplied history; 404 if no history |
| GET | `/wallets/{address}/copyability` | Active subscription | Follower-adjusted results across the delay ladder, at the caller's order size |
| GET | `/wallets/{address}/capacity` | Active subscription | How much money this wallet's edge can carry at your order size |
| GET | `/tokens/{mint}/velocity` | Active subscription | Swaps taken to reach each liquidity level |
| GET | `/wallets/{address}/authenticity` | Active subscription | Entry timing, funding graph and the resulting block reason |
| POST | `/watches` | Signed in | `{wallet}`; add public address/fixture identifier to personal watchlist |
| DELETE | `/watches/{wallet}` | Signed in | Remove from personal watchlist |
| POST | `/settings` | Signed in; subscription to enable | `{enabled,budget,mode:"paper"}`; 0.01–10 SOL; other modes return 409 |
| POST | `/paper/replay` | Active, enabled subscriber | Incremental historical paper replay; returns opened/closed counts |
| POST | `/positions/{id}/close` | Owner | Close own open paper position using latest supplied token price |
| GET | `/connectors` | Public | Actual readiness of Axiom/Fomo/Pump integration targets |
| GET | `/plans` | Public | Active locally configured plans |
| POST | `/keys` | Active subscription | Rotate personal API key; clear key shown once, expires in 30 days |
| DELETE | `/keys` | Signed in | Revoke personal API key |
| GET | `/admin` | Admin session | Users, plans, last 100 logs/payments, total recorded revenue in EUR cents |
| POST | `/admin/users/{id}/ban` | Admin session | `{banned}`; cannot ban administrators; revokes access and disables execution |
| POST | `/admin/plans` | Admin session | Upsert `{id,name,price_cents,days,active}` |
| POST | `/admin/subscriptions` | Admin session | `{user_id,plan_id,reference,paid}`; unique verified payment or grant reference |
| POST | `/admin/import` | Admin session | Array of 1–10,000 normalized trade events; no automatic execution |
| POST | `/admin/events` | Admin session | One recent normalized trade event; distribute paper evaluation to eligible followers |
| GET | `/admin/fees/verify` | Admin session | Checks the configured fee account is a real token account that can actually receive fees |
| POST | `/admin/platform` | Admin session | `{copy_trading:bool}`; switches copy trading off or on platform-wide |
| POST | `/admin/fees/settle` | Admin session | Runs performance settlement for every unbanned account |
| POST | `/admin/ticks` | Admin session | Array of 1–1,000 `{token,price_sol,timestamp}`; records prices and evaluates tick exits |
| POST | `/admin/rpc/redecode` | Admin session | Re-runs the current decoder over every archived raw transaction; adds newly decodable trades, never rewrites existing ones |
| POST | `/admin/rpc/backfill` | Admin session | `{wallet,pages?}`; walks up to 25 pages of real history and reports whether the wallet now clears the paper gates |
| POST | `/admin/rpc/sync` | Admin session | `{wallet,before?:string|null}`; collect a 20-signature RPC page |
| GET | `/admin/discovery` | Admin session | Collector state, observation count and the top 100 candidate wallets by observed SOL volume |
| POST | `/admin/cluster/index` | Admin session | `{wallet}`; build funding edges from already-archived raw transactions. No network access |
| POST | `/admin/discovery/promote` | Admin session | `{wallet}`; mark a candidate for history collection. Grants no eligibility |
| POST | `/admin/risk/token` | Admin session | `{mint}`; collect and cache a token safety assessment |
| POST | `/swap/quote` | Signed in | `{input_mint,output_mint,amount,slippage_bps}`; indicative price, route and fee |
| POST | `/swap/build` | Signed in | Same body plus optional `taker`; returns an **unsigned** transaction for the user's wallet |
| POST | `/swap/submitted` | Signed in | `{id,signature?,submitted}`; records what the wallet did |
| GET | `/swaps` | Signed in | Last 100 swaps for the account |
| GET | `/market/{mint}` | Signed in | Cached market data for the deepest pool, with its age |
| GET | `/market/{mint}/candles` | Signed in | OHLCV for the deepest pool; `?tf=` is one of `5m`, `1h`, `4h`, `1d` |
| GET | `/portfolio` | Signed in | On-chain holdings for the linked wallet, priced where possible |
| GET | `/tokens/{mint}/holders` | Signed in | Largest token accounts plus traders observed on this token |
| GET | `/feed` | Signed in | Trades by followed wallets, and your own positions marked to the last tick |
| GET | `/search` | Signed in | `?q=`; tokens, wallets and X handles we have observed. A pasted address resolves directly |
| GET | `/discover` | Signed in | Recently profiled tokens plus launches this service observed |
| GET | `/tokens` | Signed in | Mints observed by this service, newest first; repeat mints by one deployer collapse to one row with a count |
| GET | `/tokens/{mint}` | Signed in | Token identity, social reputation, cached risk and last observed price |
| POST | `/admin/social/resolve` | Admin session | `{handle}`; metered X lookup, cached per account |
| GET | `/tokens/{mint}/risk` | Active subscription | Cached assessment, block reason, collection time and staleness |

## Wallet sign-in

Two steps. `POST /auth/wallet/challenge` returns the exact string to sign; `POST /auth/wallet/verify`
redeems it. Solana signatures are base58 ed25519 over the raw message bytes; Ethereum signatures are
`0x`-prefixed 65-byte `personal_sign` output, from which the signer is recovered and compared.

The nonce is single-use and is deleted **as part of verification**, so an invalid signature still
burns it and two concurrent submissions cannot both succeed. The message carries the request origin,
the address and a 300-second expiry, and states in plain language that it approves no transaction,
transfer or allowance.

Signing in with a wallet still provisions a *separate* managed wallet for trading. The login wallet
is identity only; this service never holds its key and never asks for it.

## Funds

`GET /funds` returns `address`, `deposit_address`, `custody_enabled`, `withdrawals_enabled`,
`balance_lamports`, `balance_sol`, `balance_error`, `realized_pnl_sol`,
`commission_owed_lamports`, `withdrawable_lamports`, `reserved_lamports`, `fee_model`,
`conflict_note` and `access_model`.

A balance the provider could not return is `null` with `balance_error` set. It is never rendered as
zero: reporting "we could not ask" as "you have nothing" would be a lie about someone's money.

`withdrawable_lamports` is `balance − commission owed − rent-exempt minimum (890,880) − fee reserve
(5,000)`, saturating at zero.

`POST /funds/withdraw` settles outstanding commission first, then builds, signs and broadcasts one
System transfer. The attempt is written to the ledger **before** broadcasting, because a send that
times out may still land. A timeout returns 502 with status `unknown`, not `failed`; retrying sends
a second payment, so reconcile the signature first.

### Export versus close

`POST /funds/export-key` hands the owner a copy of their private key, at any time. This matches how
the incumbents work — Axiom issues an exportable seed phrase at signup through Turnkey, Trojan
exports on request — so nobody is locked out of their own funds by the platform's convenience.

**An export is not an exit.** `custody_ended` is `false` and the warning says plainly that this
service still holds the key and can still sign. Control becomes *shared*. Sole control requires
moving the funds to a wallet this service never generated, or closing the account.

`POST /account/close` requires `{confirm:"CLOSE"}` and no open positions. It returns the base58
64-byte keypair once, revokes all sessions and empties the stored ciphertext. There is no second
copy.

## Commission

`GET /fees` returns the `schedule`, the `cost_model`, the account's `high_water_lamports`,
`total_charged_lamports` and the ledger. Every model carries a `conflict_note` stating plainly whose
interest it serves.

The default is a **performance fee**: a share of realized profit above the account's own previous
peak, netted across all closed positions, charged at most once per epoch. A losing period costs
nothing and a recovery is not charged twice. `per_trade` and `hybrid` are available and both declare
that they pay the platform more when the user trades more.

Amounts are integer lamports. `reference` is the idempotency key, so a replayed settlement or
position event cannot bill twice.

## Trade schema

```json
{
  "id": "UNIQUE_SIGNATURE:INSTRUCTION_INDEX",
  "wallet": "PUBLIC_WALLET_ADDRESS",
  "token": "TOKEN_MINT_ADDRESS",
  "symbol": "TOKEN",
  "side": "buy",
  "quantity": 100000,
  "price_sol": 0.00001,
  "fee_sol": 0.000005,
  "timestamp": 1789730000,
  "liquidity_sol": 1500
}
```

`routed` is optional: `true` when the venue was reached through an aggregator, `false` on a direct
invocation, absent when the source does not report it. Absent means *not observed*.

Ids may be up to 256 characters. A real Solana signature is 88 characters and an address 44, so a
natural `source:signature:wallet` id is around 137; the other string fields remain capped at 128.

Quantity is token units, not raw integer token amount. Price is SOL per token. Fee is total SOL fee attributable to this trade and must not also be embedded in the price. If fees are already embedded in a normalized price, don't charge them twice. The optional RPC converter embeds protocol fees in the effective price and reports the separately observed network fee in `fee_sol`.

The example timestamp is illustrative. `/admin/events` requires a current Unix-seconds timestamp, no older than 120 seconds and no more than 5 seconds ahead. History imports may be older but not over 60 seconds into the future. Use a unique ID per event; duplicate IDs are ignored, not updates. Preserve source records and provenance outside the normalized object if your importer supplies them.

Liquidity zero means unknown; it prevents new paper entries. Liquidity is trusted administrator-supplied data, not automatically verified by the API. Do not assume a provider's reported market cap is liquidity.

Example import:

```bash
curl -c /tmp/unskilled-session.txt \
  -H 'Content-Type: application/json' \
  --data-binary @login.json \
  http://127.0.0.1:8080/api/auth/login

curl -b /tmp/unskilled-session.txt \
  -H 'Content-Type: application/json' \
  --data-binary @fixtures/demo-trades.json \
  http://127.0.0.1:8080/api/admin/import
```

`login.json` contains your administrator email/password; protect and remove it after use. The browser administration page avoids needing this file.

Example customer query:

```bash
curl http://127.0.0.1:8080/api/wallets/PUBLIC_WALLET_ADDRESS \
  -H "Authorization: Bearer $UNSKILLED_API_KEY"
```

## Subscription records

`paid:false` creates complimentary access with a zero-valued payment record. `paid:true` records the active plan's EUR price as already verified externally. Neither operation charges money. Duplicate references are rejected transactionally and do not extend expiry a second time. Duration extends from the later of now and the existing expiry.

Do not expose `/admin/subscriptions` as a public payment callback. A real processor adapter must verify signed events and map immutable invoice IDs before calling the entitlement logic.

## Price ticks

`POST /admin/ticks` accepts 1–1,000 observations of `{token, price_sol, timestamp}`. Prices must be
positive and finite; timestamps must be positive and no more than 5 seconds ahead. Returns
`{recorded, closed[], note}`, where each `closed` entry carries `id`, `user_id`, `reason`, `exit`
and `pnl`.

Unlike `/admin/events`, a tick can **never open a position**. It can only manage or close exposure
that already exists. Duplicate `(token, timestamp)` pairs are ignored rather than overwritten, and
closing is idempotent, so replaying a tick batch cannot double-count P&L.

Exit reasons are described in `METHODOLOGY.md`. See `EXIT_HARD_STOP_PCT`, `EXIT_TRAILING_PCT` and
`EXIT_TIME_STOP_SECONDS`.

## RPC pagination

Collection returns `scanned`, `archived`, `decoded`, `inserted`, `unavailable`, `next_before`, and a coverage statement. Paste `next_before` into the next request to page backward. An empty page ends available history. The node's available history may itself be incomplete.

The endpoint is administrator-only, has a 30-second per-wallet cooldown and one concurrent collection per process. HTTP 429 responses are retried with bounded backoff. A collection can still fail; it is not a continuous stream. Do not expose the RPC URL as a user-controlled request field; the endpoint is configured on the server to avoid SSRF.

## Candidate discovery

`GET /admin/discovery` returns `running`, `observations` and `candidates`. Each candidate carries
`wallet`, `first_seen`, `last_seen`, `buys`, `sells`, `sol_volume`, `max_trade_sol`,
`distinct_tokens`, `promoted` and `source`.

These are size observations from a public firehose, not performance. A candidate has no analytics
until its history is collected: `GET /wallets/{address}` returns 404 for a promoted wallet with no
imported or RPC-collected trades. Ordering is by observed volume and is not a ranking of skill.

Upstream bonding-curve reserves are *virtual*. They are recorded under their own name and are never
written into `liquidity_sol`, because a virtual reserve cannot be sold into. Discovery therefore
does not relax the liquidity gate in `METHODOLOGY.md`.

The collector is disabled unless `DISCOVERY_ENABLED=true`. It deduplicates by transaction signature,
so a reconnect that replays messages cannot inflate a wallet's counters, and it prunes observation
rows older than seven days.

## Copyability

`GET /wallets/{address}/copyability` returns `{wallet, order_sol, cost_bps, max_wait_seconds,
leader_realized_pnl_sol, decay[], flags[]}`. `order_sol` is the caller's own configured paper order
size, so two users can get different answers for the same wallet — which is correct, because
copyability depends on size.

Each `decay` entry carries `delay_seconds`, `attempts`, `entered`, `resolved`, `unknown_liquidity`,
`unfilled_entries`, `unresolved_exits`, `realized_pnl_sol`, `win_rate`, `expectancy_sol` and
`median_entry_slippage_pct`. See `METHODOLOGY.md` for what each excluded case means.

`leader_realized_pnl_sol` sits beside the follower results deliberately. The gap between them is the
number this product exists to show.

## Authenticity

`GET /wallets/{address}/authenticity` returns `buys`, `early_entries`, `early_entry_pct`,
`early_window_seconds`, `funders`, `co_funded_wallets`, `funded_by_counterparty`,
`funding_graph_observed`, `flags` and `blocked`.

`blocked` is non-null when the wallet's entry timing is not reproducible by a follower. Funding
findings never populate it: a shared funder is routinely innocent, so it is surfaced for review
rather than enforced. `funding_graph_observed: false` means no raw transactions have been archived —
**unknown, not clean**.

`POST /admin/cluster/index` derives edges from transactions already stored by `/admin/rpc/sync`. It
makes no network requests of its own; widen coverage by collecting more history first.

## Search

`GET /search?q=` searches what this service has observed — not every token on Solana, and it says
so. A valid base58 address short-circuits the search and comes back as `exact`, classified as a
wallet if it has traded in our record and a token otherwise, because somebody pasting 44 characters
already knows what they want.

**Every branch returns every key**: `exact`, `tokens`, `wallets`, `handles` and `note`. `tokens`,
`wallets` and `handles` are always arrays, empty when there is nothing to report. The address
branch once omitted `handles` and the interface called `.map` on nothing, which unmounted the whole
page. A response that leaves a key out asks the caller to guess, and the caller guessed wrong.

## Market data

`GET /market/{mint}` returns `market`, `age_seconds` and a note. Every numeric field is nullable: a
figure the provider did not report is `null`, never `0`.

**Only pools where the mint is the base token count.** The provider returns every pool the mint
appears in, on either side, and in a pool where it is the *quote* asset `baseToken`, `priceUsd` and
`marketCap` all describe a different token. Those pools are discarded rather than reinterpreted —
without this, asking for USDC returns whatever trades against it.

The deepest qualifying pool is reported and the rest are counted in `pools`, never summed. Summing
liquidity across pools would overstate what a single order can reach. A pool with unreported
liquidity only wins when nothing else is available, because unmeasurable is not deep.

Warnings cover thin liquidity in money terms, split depth, unreported liquidity, and a token with no
website or social account.

Fetching a market also feeds the social ledger: an X handle learned here counts toward reuse exactly
as one learned from a launch does, which extends the signal to tokens that never passed through the
creation stream.

## Portfolio, holders and feed

`GET /portfolio` reads balances straight from the chain for the wallet linked to the account —
both token programs, since a wallet can hold the same asset under either. Amounts are carried as
`raw_amount` strings plus `decimals`; a u64 balance does not survive a JSON number, and a balance is
the one figure a user checks against their own wallet.

A holding we cannot price is **unpriced, not worthless**: `value_usd` is `null` and
`unpriced_tokens` counts them, so the total never quietly understates what someone owns. A failed
balance lookup is reported in `holdings_error` rather than rendered as an empty wallet.

`GET /tokens/{mint}/holders` returns `holders`, `supply` and `top_traders`. Holders are **token
accounts, not people** — one owner can hold through several, and the largest entry is usually a pool
rather than a person. `share_pct` is `null` when supply is unknown, because a percentage of an
unknown total is not a percentage, and is clamped at 100 so a stale supply cannot report 340%.
`top_traders` covers only wallets this service has observed, so it is not a global ranking.

`GET /feed` returns trades by followed wallets and the caller's own positions. An open paper
position is marked against the **last observed tick** and reported as `unrealized_pnl_sol`, never
folded into realized P&L. A position with no tick shows no mark rather than a stale one.

## Candles

`GET /market/{mint}/candles?tf=1h` returns `candles` (oldest first), `pool`, `timeframe`,
`age_seconds`, the supported `timeframes` and a note.

Candles are per **pool**, not per token, because a price only exists inside a market. The deepest
pool is charted and the others are ignored rather than averaged — blending pools of different depth
invents a price nobody could trade at.

A malformed bar is dropped rather than drawn: a high below its low, a body outside its wick, a
nonpositive price or a nonfinite number. A broken bar is worse than a missing one because on a chart
it is indistinguishable from a real move.

Only the four listed timeframes are accepted; the value is interpolated into the provider's URL, so
an unknown key is refused rather than passed through.

## Swaps

Trading is **non-custodial**. `/swap/build` returns a base64 versioned transaction with an empty
signature slot; the user's own wallet signs and broadcasts it. This service never holds the key that
signs a trade.

**The quote is never accepted from the caller.** A quote fixes the route, the amounts and the
platform fee, so the client sends parameters and the server fetches the quote itself immediately
before building. A tampered quote cannot reach the router.

`taker` defaults to the Solana wallet the account signed in with, falling back to its managed
wallet, which is provisioned on first use if absent. Supplying a `taker` that is not linked to the
account is refused — you cannot build a transaction for someone else's address.

Refusals happen before any network call: identical mints, zero or implausible amounts, zero
slippage, and slippage above `MAX_SLIPPAGE_BPS` (the message names both your figure and the cap).

The quote summary carries `in_amount`, `out_amount`, `minimum_out`, `price_impact_pct`,
`platform_fee`, `platform_fee_bps`, `route` and `warnings`. Amounts are parsed as integers, never
floats, so large token amounts do not lose precision. A route of more than two pools, a price impact
at or above 5%, or an unreported impact each raise a warning.

`platform_fee_bps` is `0` unless `PLATFORM_FEE_ACCOUNT` is set. The service does not request a fee
it has nowhere to receive.

A misconfigured fee account does not error at quote time — the router simply refuses the fee and
revenue quietly stays at zero. `GET /admin/fees/verify` checks the account exists, is a token
account, and names its mint, so that failure is visible rather than silent. For reference, peer
platforms charge 0.5% to 1% per swap: Photon 0.5%, Axiom about 1% falling to 0.75% at its top tier,
GMGN 1% flat.

A swap that is built but never reported back stays `built`. That is honest: we do not know whether
it landed. Only the client can tell us, through `/swap/submitted`.

## The observed launch list

`GET /tokens` returns the mints this service saw created, newest first, as `tokens` plus a `note`.
Each row carries `mint`, `name`, `symbol`, `twitter`, `first_seen`, `creator`, `handle_tokens` and
`copies`.

`copies` exists because one deployer minting the same name over and over is the most common thing
on the creation stream: twelve identical *YO / LAST COIN 3.3 MILL* mints came from a single address
inside forty seconds. Those are twelve distinct mints, not duplicated rows, but listing all twelve
buries every other launch. A batch is therefore collapsed to its **newest** mint and `copies` says
how many there were. Grouping is by `(creator, symbol, name)`; a mint whose creator is unknown
groups only with itself, because without a deployer two coins sharing a name are just two coins
sharing a name.

`copies` is worth reading as a signal in its own right. A deployer who mints the same coin twelve
times is telling you something about the coin.

## Token socials

`GET /tokens/{mint}` returns `token` (name, symbol, uri, creator, twitter, telegram, website),
`social`, `risk` and `last_price_sol`.

`social` carries `handle`, `tokens_promoted`, `previous_mints`, `user_id`, `previous_handles`,
`followers`, `account_created`, `identity_resolved` and `flags`.

Two signals, with different costs and different strength:

- **Reuse across mints** is derived from this service's own record. Free, and it strengthens the
  longer the collector runs. It is a *lower bound*: a handle absent from our record is unknown, not
  new.
- **Rename detection** requires the account's immutable numeric id, which only X can supply. X
  removed its free tier in February 2026 and meters reads, so resolution happens once per account
  through `POST /admin/social/resolve` and is cached. Without it `identity_resolved` is `false` and
  a rename cannot be ruled out — unknown, not clean.

A handle is mutable and an id is not. Without the id we can show that one handle fronted many
tokens; we cannot show that one account wore many handles.

## Velocity and capacity

`GET /tokens/{mint}/velocity` reports, for each of 10/25/50/85 SOL, how many observed swaps and how
many seconds it took to first reach that depth. Published work found that reaching a given depth in
*fewer* trades predicted graduation better than any other variable tested.

Counts come from our own observations, so they are a **lower bound**: swaps we never saw make a
token look faster than it was. A trade carrying unknown liquidity counts toward the total — it
happened — but cannot cross a threshold, because unknown depth is not evidence of reaching one.

`GET /wallets/{address}/capacity` answers the question no other gate asked: how much capital the
edge carries. It reports `eligible_entries`, `capped_by_liquidity`, the median and smallest order
the policy would actually have allowed, `below_cost_floor` (entries too small to pay for their own
fixed costs), `peak_concurrent_sol` for one follower, and that figure multiplied across 1/10/100/1000
followers. The multiplication is the crowding case — everyone taking the same size at the same
moment — not an average.

`regime` gives the median trades-to-cross of the tokens this wallet buys, so a wallet that only
enters slow-filling tokens is distinguishable from one that enters fast ones.

## Deployer funding

`authenticity` now also returns `deployer_funded` and `deployer_funding_enforced`. A hit requires
all three, from our own records: the token's deployer is known, that deployer sent SOL directly to
the wallet, and the transfer landed within `DEPLOYER_FUNDING_WINDOW` *before* the wallet bought that
deployer's token. A wallet buying a token it deployed itself is also a hit.

Only this narrow case gates, and only when `ENFORCE_DEPLOYER_FUNDING=true`. Shared funders, exchange
edges and co-funding sets stay report-only, because those shapes are routinely innocent. Hits are
reported whether or not enforcement is on, so you can see what enabling it would cost before you
enable it.

## Token risk

`POST /admin/risk/token` fetches one report from the configured provider, evaluates the gate, and
caches the decision. The provider URL is server-configured and is never a request field.

`GET /tokens/{mint}/risk` returns `{assessment, blocked, checked, stale, note}`. `blocked` is `null`
when the token cleared the gate and a sentence naming the defect otherwise. A 404 means no
assessment has been collected — which is *unknown*, not *clear*.

`assessment` carries `score` (upstream 0–100, higher is riskier), `rugged`, `mint_authority`,
`freeze_authority`, `metadata_mutable`, `lp_locked_pct` (the worst observed market, not an average),
`top_holder_pct`, `holders`, `creator` and up to twelve named `risks`.

An absent authority field reads as **present**. A report that does not mention an authority is not
evidence that it was revoked.

## Paper refusals

`/paper/replay` and `/admin/events` return a `refusals` object mapping a reason sentence to the
number of followed buys it rejected. Reasons currently issued:

- `Supplied liquidity is unknown or below 100 SOL`
- `Order size exceeds 0.1% of supplied liquidity`
- any token risk block reason, or the unknown/stale messages under `TOKEN_RISK_REQUIRED`
- `Source wallet has fewer than 20 prior matched sells`
- `Source wallet has incomplete cost basis in prior history`
- `Source wallet ranking heuristic is below 65`
- `Source wallet enters within Ns of a token's first trade on P% of buys`
- `Open exposure cap reached`
- `Daily realized loss guard is active`

A buy from an unfollowed wallet, or a non-buy event, is not a refusal and is not counted.

## The copy-trading switch

`POST /admin/platform` with `{copy_trading:false}` disables the feature platform-wide. It is a real
switch, not a hidden menu item:

- `/settings` refuses to enable paper execution (409),
- `/paper/replay` refuses (409),
- the follower fanout returns zero, which also closes the **live collector** path — that reaches
  fanout without passing through a route, so the check lives in the function rather than only at
  the edges,
- every account that had it enabled is stood down, so flipping the switch back does not silently
  resume trading for people who have forgotten they opted in.

`GET /me` reports `copy_trading`, and the interface removes the navigation entry rather than
offering a page that refuses.

**Swapping is unaffected.** The two are separate products and the switch does not touch the trading
path; a test asserts that.

## Automatic paper event distribution

A recent normalized event is committed together with eligible follower decisions. Eligibility requires subscription validity, no ban, execution enabled, and following the event wallet. Each follower still passes the policy in `METHODOLOGY.md`. An event can be accepted with zero orders. No historical trades are executed as a side effect of recent-event ingestion.

There is no external webhook listener configured, no signing key, and no real transaction broadcasting in this path. A supported upstream adapter must call it with validated events. The endpoint is useful for integration development and deterministic paper operation.
