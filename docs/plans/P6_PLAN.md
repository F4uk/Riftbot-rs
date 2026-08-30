# P6 V1 Execution Basket Coordinator Plan

Status: authorized after GPT Gate 5 PASS

## Verified starting point

- Approved P5 commit merged to `main`: `97d688879399fe8951030103da95f66a8eaa7380`
- Green hosted `main` CI:
  [run 33290212335](https://github.com/F4uk/Riftbot-rs/actions/runs/33290212335), conclusion
  `SUCCESS`
- P6 branch: `codex/p6-execution-basket`

## Safety and scope freeze

P6 is a deterministic execution-safety stage. It may create validated execution intents, prepare
commands, journal commands before dispatch, and consume simulated execution events. It must not
connect to a private venue endpoint, load credentials, submit a real order, perform tiny-live
validation, or implement P7 startup/account reconciliation. The existing Nautilus edge remains a
compile-only boundary; all P6 behavior is exercised through a deterministic fake execution port.

V1 supports exactly two initial strategy legs and these intent purposes:

- `IncreaseRisk`: approved P4/P5 risk increase only;
- `ReduceRisk`: P4 `ReduceRisk` or `FlattenForReversal`, preserving the source action/reason;
- `ResidualHedge`: coordinator-internal recovery backed by actual parent-basket fills;
- `EmergencyFlatten`: safety action backed by supplied known-position evidence.

Recovery priority is risk reduction, then execution quality, then profit. No unbounded market order,
blind timeout retry, unbounded recovery loop, custom venue client, distributed lock, transfer,
maker strategy, or 1:N/N:M execution is in scope.

## Frozen contracts

1. Harden `ExecutionIntent` construction and serde around decision identity, embedded
   `RiskAssessment`, risk context, P4 source identity, purpose compatibility, authorized
   matched-notional-per-leg, finite side-aware price guards, expiry, and recovery evidence.
2. Normal intents bind the source `InventoryDecision`; decision IDs and symbol/route facts must
   agree. `IncreaseRisk` requires `RiskDecision::Approve`, an `IncreaseRisk` P5 action, positive
   authorization, normal risk authority, and P3 measured-size facts. `ReduceRisk` accepts valid
   P5 reduction authorization under `Approve`, `ReduceOnly`, or `FlattenRequired` without an entry
   edge requirement.
3. Recovery intents are constructed only by dedicated evidence-taking constructors. A residual
   hedge references parent intent and fill evidence, cannot exceed unmatched filled exposure, and
   must strictly reduce absolute residual. Emergency flatten is bounded by a known signed position
   and cannot cross through zero.
4. Increase preflight uses caller-supplied logical time only. P5 measurement age plus elapsed time
   since P5 evaluation must remain within the frozen P5 maximum; future/regressive times fail
   closed. Preflight freezes healthy books, remeasures actual planned quantities through the P3
   spread/depth path, enforces finite price/slippage guards, and aborts before either command if
   current increase economics are no longer valid.
5. Instrument metadata supplies positive lot size and quantity precision. Quantities round down;
   checked fixed-decimal arithmetic proves neither leg exceeds its authorized notional or P3
   measured quantity. Planned imbalance must remain within the configured tolerance or the basket
   is rejected/shrunk conservatively.
6. Stable child identity is derived from intent ID, leg index, generation, and command ID. Initial
   child identities and both submit commands are journaled before one parallel action batch is
   emitted. A timeout moves the child to `Unknown` and the basket to `Unknown`/`Reconciling`; it is
   never treated as rejection and never causes a blind replacement.
7. Child states are `NotSent`, `Submitting`, `AcceptedOpen`, `PartiallyFilled`, `Filled`, `Canceled`,
   `Rejected`, and `Unknown`. Basket states are `Planned`, `Submitting`, `Pending`, `Partial`,
   `Imbalanced`, `Hedging`, `Balanced`, `Complete`, `Unknown`, `Reconciling`, `Aborting`, and
   `FailedSafe`.
8. Fill IDs are idempotent. Actual cumulative fill notional drives signed residual; requested size
   never does. Regressive cumulative fill facts fail closed. Late fills after cancel request or
   timeout still update residual and can trigger bounded recovery. `Complete` requires terminal
   relevant orders, no unknown state, residual within tolerance, and no outstanding action capable
   of increasing residual.
9. A single-process reservation book permits at most one active `IncreaseRisk` basket per pair.
   Reservation is acquired before dispatch, exposed as reserved matched notional, and released or
   converted only by authoritative lifecycle state.
10. An append-only journal retains intent/risk/preflight identity, child IDs, commands, events,
    transitions, residual snapshots, recovery decisions, and terminal reason. Journal persistence
    precedes command emission. P7 will later compare this evidence with venue truth.

## Implementation sequence

1. Extend typed IDs/numerics/config only where P6 invariants need them; retain the credential-free
   `signal_only` runtime contract.
2. Replace the P0 intent skeleton with validated normal and evidence-backed recovery constructors,
   shared serde validation, source P4/P5 bindings, instrument metadata, and conservative sizing.
3. Add pure execution command/event/journal/reservation contracts and explicit validated child and
   basket state.
4. Implement the coordinator preflight, same-batch two-leg preparation, idempotent event handling,
   fill accounting, cancel/timeout semantics, bounded recovery, and fail-safe escalation.
5. Drive all lifecycle behavior through a fake port/event harness and add the required deterministic
   fault-injection matrix, including the one-leg-fill plus opposite-unknown plus late-fill case.
6. Update architecture/current-state documentation, publish `P6_REPORT.md` and
   `GATE_6_REVIEW.md`, run all required local gates, push the final P6 branch, verify hosted CI, and
   stop at GPT Gate 6.

## Explicit exclusions

No real-money trading, private API authentication, real network order submission, tiny-live run,
P7 reconciliation/restart recovery, transfer/rebalancing, maker strategy, multi-leg generalization,
custom Hyperliquid/Lighter clients, Kafka/Redis, Nautilus core modification, or P8/P9 behavior.
