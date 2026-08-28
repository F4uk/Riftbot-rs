# P1 Connectivity Report

Status: Gate 1 `PASS WITH FIXES`; corrective feed-health patch applied; P1 remains stopped

## Published implementation

- Branch: `codex/p1-connectivity`
- Normalization and health commit:
  `51123fa23be72744600f11fad6f83ae9a90705c4`
- Official connectivity and evidence commit:
  `d3a9898a7526149401b17b0157af10203372ac12`
- Gate 1 feed-health fix commit:
  `ff7787bf825f61082db6a4a44955b20bc327100c`
- Pinned Nautilus revision:
  `e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c`
- Hosted CI run: [33167667553](https://github.com/F4uk/Riftbot-rs/actions/runs/33167667553),
  conclusion `success` for `d3a9898a7526149401b17b0157af10203372ac12`.

## Official adapter validation

The manual `p1-connectivity` binary is compiled only with the `p1-connectivity` feature. It uses
the pinned official `nautilus-hyperliquid` and `nautilus-lighter` HTTP and WebSocket clients. It
does not contain a venue REST/WS implementation and does not construct an execution client.

Live Mainnet discovery on 2026-08-28 returned:

| Surface | Active perpetuals discovered |
|---|---:|
| Entropy `io` HIP-3 | 2 |
| trade.xyz `xyz` HIP-3 | 103 |
| Lighter Mainnet | 213 |

The only base common to all three discovered surfaces was `SNDK`. The V1 configuration therefore
selects exactly one symbol, `SNDK`, and the two-venue pair `entropy` plus `lighter`. The
`xyz:SNDK-USD-PERP.HYPERLIQUID` book remains part of P1 connectivity validation, but no second V1
symbol or pair is configured.

## Live book and timestamp evidence

The live probe subscribed through the official adapters to depth-10 books for:

- `io:SNDK-USD-PERP.HYPERLIQUID`
- `xyz:SNDK-USD-PERP.HYPERLIQUID`
- `SNDK-PERP.LIGHTER`

Before reconnect it accepted 3 Entropy snapshots, 3 trade.xyz snapshots, and 65 Lighter
snapshots. After forced reconnect it required and accepted one new snapshot on every feed. Every
accepted book had non-zero `exchange_ts` and local `receive_ts`; final receive timestamps advanced
to `1787916706881889800`, `1787916706892659100`, and `1787916707074414000` respectively.

At the end of the probe all three feeds were `connected` and `fresh`, with observed ages of 193 ms,
182 ms, and 0 ms. Full discovery candidates, instrument IDs, prices, timestamps, health, and
reconnect results are stored in `docs/evidence/P1_CONNECTIVITY.json`.

## Disconnect and reconnect

- Hyperliquid's official transport accepted `request_reconnect`, emitted `Reconnected`, restored
  subscriptions, and delivered recovery books for both HIP-3 feeds.
- Lighter's official `SocketControl` reconnect handle returned `Accepted`, emitted `Reconnected`,
  restored the depth subscription, and delivered a recovery book.
- `BookStore` marks disconnect/reconnect as `awaiting_recovery`; merely reconnecting the socket is
  insufficient for healthy status. A strictly newer normalized book is required.

The first Lighter probe used the lower-level client's no-override URL and received an HTTP upgrade
failure. Inspection showed that the official data config adds the venue-required `readonly=true`
query. The probe was corrected to resolve its URL with `LighterDataClientConfig::ws_url`; the full
validation then passed. This was probe integration misuse, not an official adapter gap, so no
Adapter Gap Report is required.

## Gate 1 feed-health correction

Gate 1 identified that the original `BookStore::update` promoted every accepted book to
`Connected` and cleared its recovery flag. That allowed delayed data to bypass an explicit
transport transition. The corrective state machine now behaves as follows:

- `BookStore::update` never changes transport connection state. Only
  `set_connection_state(..., Connected, ...)` can establish `Connected`.
- Connecting, disconnected, and reconnecting feeds retain an explicit recovery barrier regardless
  of accepted in-flight books.
- Entering `Connected` after any non-connected state moves the recovery barrier to that explicit
  transition timestamp. Connected alone therefore remains `AwaitingRecovery`.
- Only a book with `receive_ts` strictly greater than the recovery barrier can clear it, and only
  while the transport is already `Connected`.
- A book accepted before any connection lifecycle does not synthesize health or connection state.

Six named regression tests cover disconnected and reconnecting books, Connected-before-recovery,
pre-transition and post-transition books, and the combined `healthy_book` gate. All timestamps are
still supplied by the caller; `BookStore` uses no wall clock.

The three-venue live validation was rerun after this correction. Hyperliquid and Lighter again
accepted forced reconnect requests and emitted `Reconnected`; all three SNDK feeds received a
post-transition recovery book and ended `connected + fresh`. The rerun observed 3 initial and 1
recovery snapshot on each HIP-3 feed, and 87 initial plus 1 recovery snapshot on Lighter.

## Implemented P1 behavior

- `MarketNormalizer` converts raw fixed-decimal levels into typed books, canonicalizes bid/ask
  ordering, and fails on empty sides, non-positive levels, duplicate prices, crossed/locked books,
  or missing timestamps.
- The Nautilus bridge converts official adapter `OrderBookDepth10` events and removes only their
  zero-sized fixed-array placeholders.
- `BookStore` accepts only increasing local versions and non-regressive receive timestamps; data
  never promotes transport state.
- Book age is calculated from caller-supplied `now`, making freshness deterministic and replayable.
- Feed connection and freshness are separate, auditable states. `healthy_book` fails closed for
  missing, stale, disconnected, reconnecting, or not-yet-recovered feeds.

## Verification

Local verification at the implementation head:

| Command | Result |
|---|---|
| `python scripts/ci_policy.py all` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --all-targets --all-features` | 27 library tests passed; 0 failed; connectivity binary has 0 unit tests |
| `cargo check --locked --features nautilus-adapters` | pass |
| `cargo run --locked --features p1-connectivity --bin p1-connectivity -- discover` | pass |
| `cargo run --locked --features p1-connectivity --bin p1-connectivity -- validate` | pass after Gate 1 fix; all three feeds recovered healthy |

Corrective hosted run
[33212266899](https://github.com/F4uk/Riftbot-rs/actions/runs/33212266899) repeated the policy,
formatting, locked clippy, 27 locked tests, and pinned adapter compile checks successfully for
`ff7787bf825f61082db6a4a44955b20bc327100c`.

## Scope audit

- No SpreadEngine behavior, fair value, midline, CJ Grid, recorder, strategy, risk, trading, order
  submission, private endpoint, or P2 work was added.
- No Nautilus core code was modified.
- No custom venue REST/WS client was introduced.
- No secret, private key, API key, `.env`, or build artifact is tracked.

P1 stops here for GPT Gate 1.
