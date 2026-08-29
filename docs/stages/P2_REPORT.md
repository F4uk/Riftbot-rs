# P2 Recorder & Replay Report

Status: implemented on `codex/p2-recorder-replay`; stopped for GPT Gate 2

## Published implementation

- Green P1 `main` base: `f57febc4672a4cf1b501b6429254ec2ecdcb310b`
- Versioned schema and buffered recorder:
  `a53ae344243f2b14c1789f0825438cfaabcfc47a`
- Offline deterministic replay and tests:
  `9397b00ecce031237421533af4d74adcfe35d2f4`
- Final P2 implementation / real recording validation:
  `afdfe07e19751432643e513b64de934b86043651`
- Hosted CI for final P2 implementation:
  [run 33224328299](https://github.com/F4uk/Riftbot-rs/actions/runs/33224328299), conclusion
  `SUCCESS`

## Implemented

- Schema v1 records canonical normalized public books, explicit transport connection transitions,
  and deterministic health observations from the P1 domain boundary.
- Books preserve venue, instrument, version, `exchange_ts`, and `receive_ts`. Derived book age is
  excluded and recalculated from a recorded observation/replay time.
- Future account, order, and fill structs define typed audit shapes but are deliberately absent
  from the active `RecordedEvent` enum. P2 cannot produce or consume them as executable actions.
- The producer hot path validates then uses bounded `try_send`; it performs no file I/O and never
  waits for capacity. Full capacity returns an explicit `BufferFull` error rather than dropping
  silently.
- One background writer assigns contiguous FIFO sequences and writes deterministic JSONL.
- Shutdown closes input, drains every accepted event, writes the trailer, flushes, calls
  `sync_all`, and joins before reporting success.
- Each event has a SHA-256 over its canonical sequence/event payload. The mandatory trailer records
  schema version, event count, and SHA-256 over the exact header/event bytes.
- Replay validates the complete file before applying any event, revalidates books through
  `MarketNormalizer`, and applies books/transitions through a fresh `BookStore`.
- Replay exposes only `OfflineMarketDataOnly`; it accepts no execution client, callback, or order
  path and imports no execution module.

## Schema compatibility policy

The only supported schema is version `1`. Readers fail closed on every unknown header or trailer
version, mismatched versions, unknown fields or variants, malformed JSON, missing final newline or
trailer, count/sequence gaps, checksum mismatch, invalid domain values, or inconsistent health
assertions. Any future semantic or required-field change requires a new schema version and an
explicit reviewed reader/migration; v1 readers will not guess, downgrade, or partially replay it.

## Determinism and time

- No wall-clock API is used in `recording::schema`, `recording::recorder`, or
  `recording::replay`.
- Connection transitions use recorded `transition_ts`; books preserve recorded exchange/receive
  times; stale checks use recorded health `observed_at`.
- Replay output contains the ordered normalized event sequence and sorted final feed state, both
  deriving `Eq`. Replaying the same file twice is asserted byte-input-to-equal-report.
- Gate 1 recovery semantics are preserved: only a book whose `receive_ts` is strictly later than
  the explicit `Connected` transition clears recovery.

## Tests

`cargo test --locked --all-targets --all-features` passed 38 tests and failed 0: 27 existing
library tests plus 11 P2 integration tests; the connectivity binary has 0 unit tests.

The P2 tests cover:

- record-to-replay round trip;
- identical replay twice and identical final/event output;
- preserved venue, instrument, version, exchange timestamp, and receive timestamp;
- preserved disconnect, reconnect, Connected-awaiting-recovery, and recovery-book sequence;
- deterministic stale calculation from a recorded observation time;
- rejection of unsupported schema versions;
- rejection of truncated and checksum-corrupt data;
- rejection of invalid crossed normalized/domain books;
- mismatch rejection for recorded health assertions;
- offline-only/no-live-execution capability;
- shutdown flush and FIFO preservation for every accepted buffered event.

## Real public recording validation

Command:

`cargo run --locked --features p1-connectivity --bin p1-connectivity -- record-validate <ignored-local-path>`

Result: `PASS`. The official pinned Hyperliquid and Lighter transports discovered 3 active
Entropy/io, 103 active trade.xyz/xyz, and 214 active Lighter perpetuals. For `SNDK`, the probe
recorded 3 initial plus 1 recovery Entropy books, 3 plus 1 trade.xyz books, and 60 plus 1 Lighter
books. Both official reconnect requests were accepted, both emitted `Reconnected`, and all three
feeds ended `connected + fresh`.

The complete live file contained 87 events with content SHA-256
`6edecbbe23ac3f8763bc0d8faa731dd9a61421e0865ca08cb1dc8bda89e52332`. Replaying it twice
produced identical reports with three final feeds. Every recovery receive timestamp remained
strictly later than its applicable Connected transition.

The recording is an ignored local validation artifact and is not committed.

## Verification

| Command | Result |
|---|---|
| `python scripts/ci_policy.py all` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --all-targets --all-features` | 38 passed; 0 failed |
| `cargo check --locked --features nautilus-adapters` | pass |
| real three-venue `record-validate` | pass; 87 events; replay twice identical |

Hosted run `33224328299` repeated policy, formatting, locked Clippy, all 38 tests, and pinned
adapter compilation successfully for the final P2 implementation commit.

## Fault scenarios and security

- Files are capped at 256 MiB before replay and must be complete and internally consistent.
- Existing destinations are never overwritten by recorder creation.
- Buffer saturation and stopped-worker states are explicit errors.
- Identifiers and fixed-decimal positive price/quantity validation still run during deserialization
  and normalization.
- The schema has no credential, API key, private key, account credential, or raw private-payload
  field. Future account schema intentionally omits account identity; recordings remain ignored by
  Git and repository policy forbids tracking the `recordings/` directory.

## Known limitations

- Schema v1 records public market/feed state only. Future account/order/fill types are contracts,
  not active recorded events.
- There is no compression, rolling file policy, recovery of partial files, or forward-compatible
  best-effort replay; incomplete data is intentionally rejected.
- Recorder metrics and operational rotation belong to later observability/readiness work.

## Scope audit

No SpreadEngine, executable edge, FairValue/midline, CJ Grid, new risk behavior, order submission,
live execution, P3 behavior, custom venue client, or Nautilus core change was added.

P2 stops here for GPT Gate 2.
