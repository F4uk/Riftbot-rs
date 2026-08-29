# P2 Recorder & Replay Plan

Scope: Recorder and deterministic Replay only. This plan begins from the Gate 1-approved `main`
commit `f57febc4672a4cf1b501b6429254ec2ecdcb310b` and stops at GPT Gate 2.

## Task 1 — Versioned schema and compatibility boundary

- Define one explicit schema version and a deterministic line-oriented recording container.
- Record normalized public market books, explicit feed connection transitions, and deterministic
  feed-health observations.
- Define future account, order, and fill record shapes without connecting them to execution.
- Reject unknown versions, unknown fields, invalid identifiers/numerics, non-canonical books,
  sequence gaps, and incomplete files.
- Exclude credentials, account credentials, raw private payloads, and configuration secrets.

Verification: schema unit tests, invalid-version/data tests, format and Clippy.

## Task 2 — Buffered recorder and integrity

- Put a bounded `try_send` channel on the producer path so recording never waits for storage.
- Persist accepted events on one background writer in deterministic FIFO sequence.
- Add per-event SHA-256 checksums and a complete-file SHA-256 trailer.
- Make buffer pressure explicit to the caller and make shutdown drain, flush, and sync every
  accepted event before returning.

Verification: round trip, corruption/truncation rejection, bounded-buffer behavior, and shutdown
flush tests.

## Task 3 — Offline deterministic replay

- Parse and validate the complete recording before applying any event.
- Reconstruct books through `MarketNormalizer` and apply books/transitions through `BookStore`.
- Use only recorded transition, exchange, receive, and observation timestamps.
- Return a comparable replay event sequence and final normalized feed state.
- Expose an offline-only replay safety marker; do not accept or reference an execution client.

Verification: replay twice equality, timestamp preservation, reconnect/recovery ordering,
caller-time stale calculation, invalid-data rejection, and no-live-execution test.

## Task 4 — Gate 2 evidence

- Run repository policy, formatting, locked Clippy, locked all-feature tests, and pinned adapter
  compilation.
- Update `CURRENT_STATE.md`, create `docs/stages/P2_REPORT.md`, and create
  `docs/gates/GATE_2_REVIEW.md` with exact results and compatibility/security notes.
- Push `codex/p2-recorder-replay`, wait for hosted CI, and stop before P3.

## Explicit exclusions

No SpreadEngine, executable edge, FairValue/midline, CJ Grid, new risk behavior, order submission,
live execution, P3 work, custom venue transport, or Nautilus core modification.
