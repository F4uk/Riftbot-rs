# P4 CJ Target Inventory Plan

Scope: CJ Target Inventory only. Base is Gate 3-approved green `main` commit
`5e8883d067092389c306c15037ca96bc877e9fa8`. Work stops at GPT Gate 4.

This plan implements `PROJECT_TASKBOOK.md` V2.1 without changing P3 measurement mathematics or
entering P5 Risk authorization and P6 execution.

## Frozen target and grid rules

- `TargetFraction` is a dedicated fixed-decimal domain type in the inclusive range `[0, 1]`.
  Construction and deserialization both validate the range. A negative value can never encode
  direction.
- Direction exists only as `TargetDirection::Flat` or
  `TargetDirection::LongShort { long_venue, short_venue }`.
- Grid input is the `Deviation` for one explicit oriented route. Grid output is a desired
  non-negative target fraction and matched notional per leg.
- V1 uses a conservative floor/step rule between configured boundaries: choose the target at the
  greatest boundary not exceeding positive deviation. Below the first boundary, at zero, and for
  negative deviation, target is zero. There is no interpolation.
- `target_notional = target_fraction * strategy.max_target_notional`.
  Grid 100% maps to `strategy.max_target_notional`, never `risk.max_pair_notional`; configuration
  continues to require strict risk headroom.
- Forward and reverse routes are evaluated independently. A pair-level decision selects at most
  one risk-increasing direction. Simultaneous opposing increases are explicit ambiguity and no
  additional risk is proposed.

## Effective inventory and delta rules

- `EffectiveInventory` represents actual, reserved, and pending matched notional per leg for
  explicit oriented routes. Reserved and pending exposure count before proposing another increase.
- For the selected direction, `required_change = desired target - effective actual`.
- A positive delta is an increase candidate. It requires P3 measurement validity, positive
  `TradableEdge`, and economic permission; P4 does not perform P5 authorization.
- An accepted increase is capped at the notional measured by P3. The proposal preserves requested
  base quantity, measured executable notional, measurement timestamp, model version, and
  configuration fingerprint so P5/P6 can enforce or recheck the size basis later.
- A negative delta is a reduction and cannot be blocked merely by bad or missing entry economics.
- A direction reversal first emits only reduction/flattening of the old direction. The opposite
  increase may be reconsidered only after actual, reserved, and pending old-direction exposure is
  zero.
- Target and effective actual notionals are matched notional **per leg**. Gross two-leg exposure is
  twice that amount and is never substituted for the per-leg delta.

## Implementation tasks

1. Harden the target-fraction domain and serde boundary independently of grid configuration.
2. Implement deterministic `GridInventoryModel` step sizing and explicit route outputs.
3. Implement pair-level route arbitration and `InventoryManager` comparison against actual,
   reserved, and pending exposure.
4. Enforce measurement asymmetry, reversal sequencing, and measured-size binding without creating
   an execution intent or RiskDecision.
5. Run policy/format/locked Clippy/all-feature tests/pinned adapter checks, publish
   `docs/stages/P4_REPORT.md` and `docs/gates/GATE_4_REVIEW.md`, push, wait for hosted CI, and stop.

## Required verification

Tests cover zero and every grid boundary, floor behavior between boundaries, monotonic expansion
and convergence, forward/reverse direction, target-fraction construction and serde rejection,
100% strategy-cap mapping and risk headroom, per-leg notional semantics, correct increase/reduction
deltas, economic blocking only for increases, opposing-route ambiguity, two-step reversal, and
actual/reserved/pending exposure preventing duplicate increases.

## Explicit exclusions

No orders, `ExecutionIntent`, `ExecutionBasketCoordinator`, order lifecycle, hard `RiskManager`,
P5 behavior, custom strategy brain, P3 formula change, or copied CJ source.
