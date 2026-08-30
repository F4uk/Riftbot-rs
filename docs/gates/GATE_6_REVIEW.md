# GPT Gate 6 Review Package

Decision requested: review P6 V1 deterministic execution safety only. Do not authorize or infer P7
from this package.

## Review inputs

- Governing semantics: `PROJECT_TASKBOOK.md` V2.1 plus the user-authorized P6 contract
- Frozen plan: `docs/plans/P6_PLAN.md`
- Stage report: `docs/stages/P6_REPORT.md`
- Intent/evidence boundary: `src/domain/execution_intent.rs`
- Child/basket state and serde boundary: `src/execution/state_machine.rs`
- Pure coordinator, commands, events, journal, and reservations: `src/execution/coordinator.rs`
- Residual recovery planner: `src/execution/residual.rs`
- Deterministic fault harness: `tests/p6_execution.rs`
- P5 green merge base: `97d688879399fe8951030103da95f66a8eaa7380`
- Green merged-main CI:
  [run 33290212335](https://github.com/F4uk/Riftbot-rs/actions/runs/33290212335), conclusion
  `SUCCESS`
- P6 implementation commit: the commit containing this package; final SHA is reported after push
- Hosted P6 CI: required before handoff and reported with the final SHA
- Tests: 193 passed, 0 failed; 48 are P6 deterministic execution/fault tests

## Gate checklist

- [x] `ExecutionIntentPurpose` distinguishes `IncreaseRisk`, `ReduceRisk`, `ResidualHedge`, and
  `EmergencyFlatten`.
- [x] Normal intent and serde bind intent/decision/pair/symbol identity, complete P4 source,
  `RiskContext`, `RiskAssessment`, regime, kill state, P4 sizes, P5 authorization, P3 size basis,
  measurement age/cap, and measurement fingerprint.
- [x] P4 `FlattenForReversal` is `ReduceRisk` execution and preserves the original P4 decision.
- [x] Increase requires P5 `Approve`, P5/P4 `IncreaseRisk`, positive authorization, `Ready`, and a
  non-restrictive regime; typed restrictive states cannot be bypassed.
- [x] Reduction consumes valid `Approve`/`ReduceOnly`/`FlattenRequired` authorization, proves
  movement toward lower route exposure, uses reduce-only where supported, and does not require
  positive entry edge.
- [x] Residual hedge construction is coordinator-internal, references parent/fill evidence, is
  bounded by actual unmatched filled exposure, and strictly lowers absolute residual.
- [x] Emergency flatten requires typed P5 flatten authority and signed known-position evidence,
  strictly approaches zero, and cannot cross into a larger opposite position.
- [x] Increase preflight adds P5 measurement age to elapsed caller logical time and rejects stale,
  future, regressive, or expired authorization before dispatch.
- [x] Preflight freezes exact books/health/evaluation, reuses P3 spread/depth/opportunity paths at
  planned quantity, and rechecks current positive economics, finite guards, and slippage.
- [x] Fixed-decimal lot quantization rounds down and cannot exceed P5 per-leg authorization, P4
  proposal, P3 requested quantity, either measured leg, or safe matched-notional cap.
- [x] Only `MarketableLimit` and `ImmediateOrCancel` exist; there is no unbounded market fallback.
- [x] Business state is Nautilus-independent. P6 provides only an `ExecutionPort` trait and fake
  test port; no real implementation or network path exists.
- [x] Exactly two normal strategy children are prepared and journaled before both submit commands
  are returned in one action batch.
- [x] Child identity is stable by root intent, leg index, and generation. Timeout becomes
  `Unknown`; no timeout path creates a replacement.
- [x] The full frozen basket and cancel state sets are explicit. `Complete` requires terminal
  children, no unknown state, residual tolerance, and no outstanding residual-changing action.
- [x] Actual cumulative fills, never requested quantity, drive signed residual. Average-price /
  notional arithmetic and side-aware fill guards are revalidated.
- [x] Duplicate fills are idempotent; conflicting duplicate IDs and regressive cumulative data fail
  safe. Late and out-of-order events remain exposure-visible.
- [x] The critical filled-leg plus opposite-`Unknown` plus late-fill scenario cannot double hedge.
- [x] Recovery waits for authoritative initial state, uses finite price bounds, advances by stable
  bounded generation, and ends in `FailedSafe`/`FlattenRequired` when it cannot improve.
- [x] Cancel requested, confirmed, rejected, and unknown are distinct; a fill after cancel still
  updates actual residual.
- [x] One active increase reservation per pair is acquired before dispatch, blocks a second
  increase, overlays into `EffectiveInventory`, and is not freed by ambiguity or recovery.
- [x] A late fill reopening a completed canceled increase basket reacquires reservation visibility.
- [x] Intent/preflight/child/command/event/fill/cancel/timeout/transition/residual/recovery/terminal
  evidence is appended through a journal boundary; journal failure returns no command batch.
- [x] Serde cannot forge enlarged intent authority, incoherent child/fill accounting, impossible
  basket completion, unknown-free reconciliation, or failed-safe state without restrictive
  authority.
- [x] Same logical command/event sequence yields identical coordinator state, journal, and
  reservation state.
- [x] Repository policy, formatting, locked Clippy, 193 tests, and pinned adapter compilation pass.
- [x] No credentials, live/tiny-live trading, P7 reconciliation, transfer, maker, multi-leg,
  distributed infrastructure, custom venue client, or Nautilus core change is present.

## Required fault-injection review

1. Replay the full-fill happy path and verify the two initial submits share one batch and are
   journaled first.
2. Inject reject, partial fill, delayed acknowledgement, timeout, cancel reject/unknown, duplicate
   fill, conflicting fill, cumulative regression, and late fill in different orders.
3. Focus on one-leg fill plus other-leg `Unknown`: verify recovery is forbidden until authoritative
   truth arrives and a later opposite fill does not cause double hedging.
4. Exhaust recovery attempts and verify terminal `FailedSafe` plus `FlattenRequired`, with no
   further command.
5. Mutate serialized P4/P5 identity, authorization, purpose, legs, evidence, fill arithmetic,
   residual, and basket state; all impossible shapes must fail closed.
6. Move preflight past P5 freshness/intent expiry and move executable prices/depth/economics beyond
   the frozen bounds; neither initial submit may escape.
7. Attempt concurrent increases for one pair and verify the reservation appears in the next
   `EffectiveInventory` input.
8. Search P6 for wall-clock use, private credential access, venue transport, real port
   implementation, market-order fallback, or P7 startup/account reconciliation; none should exist.

P6 ends at this review package. Do not begin P7.
