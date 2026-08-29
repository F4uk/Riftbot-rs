# P4 CJ Target Inventory Report

Status: implemented on `codex/p4-grid-inventory`; stopped for GPT Gate 4

## Green base and published implementation

- Gate 3-approved P3 merged to `main`: `5e8883d067092389c306c15037ca96bc877e9fa8`
- Green merged-main CI:
  [run 33262465076](https://github.com/F4uk/Riftbot-rs/actions/runs/33262465076), conclusion
  `SUCCESS`
- Dedicated target-fraction boundary and P4 plan:
  `a10b3a1634e5ac26d709cb695ac8831e906a33c2`
- Deterministic oriented-route grid candidates:
  `2d3dccf7e23e38670c51884d116d03a767e61563`
- Effective inventory comparison and measurement asymmetry:
  `365f1fe743402da749c37ec0b2ec6033373b36bd`
- Candidate-before-target arbitration boundary:
  `5dcac9b9bac18dc67b5dee910a93289efdc0e676`
- Final decision-identity preservation:
  `ba5d968194ef956efe99b7bcd636356cde75329a`
- Explicit negative configuration serde test:
  `6be5b62b0c6be10dd55dd056db1e10c63f7fcdc3`
- Hosted CI for the complete P4 implementation:
  [run 33263554823](https://github.com/F4uk/Riftbot-rs/actions/runs/33263554823), conclusion
  `SUCCESS`

## Target-fraction domain hardening

The old signed `Fraction` was replaced by the dedicated fixed-decimal `TargetFraction`. Its only
constructor accepts the inclusive range `[0, 1]`, and serde deserialization is routed through the
same validation. A negative fraction cannot be constructed directly, parsed from configuration,
or embedded in a deserialized `TargetInventory`.

Target direction remains exclusively:

```text
TargetDirection::Flat
TargetDirection::LongShort { long_venue, short_venue }
```

The sign of a fraction is never used as route orientation. `PairId` remains orientation-free.
This invariant exists in the domain type and does not depend on `GridConfig::validate`.

## GridInventoryModel

`GridInventoryModel` is the sole target-sizing strategy. It accepts `Deviation` for one explicit
`long venue A / short venue B` route and evaluates forward/reverse routes independently. It does
not consume absolute spread or `TradableEdge`, and it cannot create an order.

V1 freezes a conservative floor-step rule: use the target at the greatest configured positive
deviation boundary not exceeding the current deviation. Zero, negative, or below-first-boundary
deviation produces zero. There is no interpolation or update-frequency state.

For the example grid:

| Deviation | Target fraction | Target notional per leg |
|---:|---:|---:|
| below 5 bps | 0.00 | $0 |
| 5 bps | 0.20 | $100 |
| 10 bps | 0.40 | $200 |
| 15 bps | 0.60 | $300 |
| 20 bps | 0.80 | $400 |
| 25 bps or above | 1.00 | $500 |

The formula is `target_fraction * strategy.max_target_notional`. With the committed configuration,
100% is $500 matched notional **per leg**, so the two-leg gross is $1000. It is not the $1500
`risk.max_pair_notional`, and existing validation continues to require the strategy cap to remain
strictly below that hard limit.

The grid exposes internal oriented candidates. It cannot directly materialize an external
`TargetInventory`; that conversion is crate-private and occurs only after pair-level arbitration.
This preserves independent route measurement without allowing two simultaneous opposing final
targets.

## EffectiveInventory and InventoryManager

`OrientedExposure` represents actual, reserved, and pending matched notional per leg. The domain
validates route shape and duplicate orientations; `InventoryManager` validates pair and configured
route identity. `EffectiveActual` projects the three components and their checked sum into the same
route/unit as a target.

For a selected route:

```text
required_change_per_leg = desired_target_per_leg - effective_actual_per_leg
effective_actual = actual + reserved + pending
```

- Positive delta is an increase candidate.
- Negative delta is a reduction.
- Zero delta produces no change.
- Reserved and pending exposure therefore prevent a duplicate risk increase before fills or an
  order lifecycle exist.

Forward and reverse candidates are compared at the pair boundary. If both require additional risk,
the result is `AmbiguousOpposingIncrease`, proposed change is zero, and no `TargetInventory` is
materialized. Opposing non-zero effective exposure is separately visible as
`AmbiguousEffectiveInventory` and also fails closed.

For a direction reversal, the old route's entire effective exposure is proposed for flattening
first. The opposite target is reconsidered only after actual, reserved, and pending old-route
exposure reaches zero. P4 never jumps through zero in one decision.

## Measurement asymmetry and size awareness

An increase requires all P3 facts to match the target route and show:

- `MeasurementValidity::Valid`;
- positive `tradable_edge_bps`;
- `increase_risk_economically_allowed = true`;
- a positive measured executable notional.

This is P3 economic permission, not P5 hard-risk authorization. A missing, invalid, mismatched, or
non-positive measurement returns `IncreaseBlocked` with an explicit reason and zero proposed size.

A valid increase is capped at the smaller of the required target delta and P3's measured executable
notional. Its proposal retains requested base quantity, measured notional, measurement timestamp,
P3 model version, and configuration fingerprint. Thus an edge measured for a small clip cannot
authorize an arbitrarily larger increase.

Reductions never require a favorable entry edge. Bad, missing, or non-positive entry economics do
not block convergence-driven reduction. A reversal uses valid opposite economics to select the
future direction but the current decision is still reduction-only and carries no increase-size
basis.

## Tests and verification

`cargo test --locked --all-targets --all-features` passed 92 tests and failed 0: 75 library tests,
11 P2 integration tests, and 6 P3 replay integration tests. P4 adds 18 focused domain/config/grid/manager
tests, with one replacing the earlier signed-fraction test.

P4 coverage includes:

- target fraction zero/one boundaries, negative and above-one construction rejection, and serde
  rejection through both the numeric type and `TargetInventory`;
- zero/negative/below-first deviation, every configured grid boundary, between-grid floor behavior,
  monotonic expansion, and convergence reduction;
- independent forward and reverse route orientation;
- 100% mapping to the strategy cap rather than the risk hard limit, with per-leg notional meaning;
- target above/below EffectiveActual and signed required-change correctness;
- actual, reserved, and pending aggregation preventing duplicate increase;
- non-positive edge blocking only increase, while bad edge cannot block reduction;
- opposing increase ambiguity and opposing effective-exposure fail-closed behavior;
- two-step reversal; and
- measured-size capping plus preservation of its exact evidence fields.

| Command | Result |
|---|---|
| `python scripts/ci_policy.py all` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --all-targets --all-features` | 92 passed; 0 failed |
| `cargo check --locked --features nautilus-adapters` | pass |

## Scope audit and limitations

P4 does not implement `RiskManager`, `RiskDecision`, hard-limit authorization, kill state,
`ExecutionIntent`, `ExecutionBasketCoordinator`, order lifecycle, order submission, account
connectivity, or P5 behavior. It does not change any P3 measurement formula and contains no copied
CJ source.

Effective actual/reserved/pending inputs are domain contracts supplied by future reconciliation and
execution lifecycle integration. P4 compares them deterministically but does not invent their
venue/account source. An `InventoryDecision` is a strategy proposal only; it cannot reach a venue.

P4 stops here for GPT Gate 4.
