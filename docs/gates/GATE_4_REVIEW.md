# GPT Gate 4 Safety-Fix Review Package

Decision requested: verify the Gate 4 safety fixes only; P5 remains out of scope.

## Review inputs

- Governing semantics: `PROJECT_TASKBOOK.md` V2.1 Mathematical & State Semantics Freeze
- P4 plan: `docs/plans/P4_PLAN.md`
- Stage report: `docs/stages/P4_REPORT.md`
- Target and effective-inventory domain: `src/domain/inventory.rs`
- Non-negative numeric boundary: `src/domain/numeric.rs`
- Sole grid strategy: `src/models/grid_inventory.rs`
- Pair arbitration and target delta: `src/strategy/inventory_manager.rs`
- Complete P4 implementation commit: `ba5d968194ef956efe99b7bcd636356cde75329a`
- Final code/test commit: `6be5b62b0c6be10dd55dd056db1e10c63f7fcdc3`
- Gate 4 fix commit: the commit containing this review package; final SHA is reported after push
- Hosted CI:
  [run 33263554823](https://github.com/F4uk/Riftbot-rs/actions/runs/33263554823), conclusion
  `SUCCESS`
- Hosted CI for the fix commit: required before final handoff and reported with the final SHA
- Tests: 102 passed, 0 failed

## Gate checklist

- [x] `TargetFraction` accepts only `[0, 1]` at construction and deserialization boundaries.
- [x] Negative target fractions cannot be introduced through config or `TargetInventory` serde.
- [x] `TargetInventory` construction and serde enforce zero/flat, positive/directional, positive
  notional, and distinct-venue cross-field invariants.
- [x] Direction is `Flat` or explicit `LongShort`; no signed fraction or `PairId` encodes it.
- [x] Grid consumes oriented-route `Deviation`, not absolute spread or `TradableEdge`.
- [x] Zero and every boundary are exact; between boundaries use the documented floor-step rule.
- [x] Expansion is monotonic and convergence lowers the target.
- [x] Forward and reverse candidates are evaluated independently.
- [x] Grid 100% maps to `strategy.max_target_notional`, below the hard risk limit.
- [x] Target notional and all actual/reserved/pending deltas are matched notional per leg.
- [x] Only pair arbitration can materialize a final `TargetInventory`, at most one per decision.
- [x] Opposing simultaneous increases and opposing effective exposure fail closed explicitly.
- [x] EffectiveActual includes actual, reserved, and pending exposure before calculating delta.
- [x] Valid positive P3 economics are required only for increasing risk.
- [x] Bad entry economics cannot block necessary reduction.
- [x] Reversal flattens the old route before any opposite increase.
- [x] Increase size is capped by and retains the exact P3 measured size/notional evidence.
- [x] Grid `Deviation` and increase economics/size come from one immutable P3 measurement view.
- [x] Route, observed timestamp, P3 model version, config fingerprint, and source deviation must
  match before any increase; mismatches fail closed explicitly.
- [x] Matched notional authorization is the smaller of checked long-price and short-price notionals
  at the exact P3 requested base quantity.
- [x] The proposal preserves requested quantity, both measured leg notionals, safe matched cap,
  timestamp, model version, and fingerprint.
- [x] Outputs are proposals, not `RiskDecision`, `ExecutionIntent`, basket, or order commands.
- [x] Repository policy, formatting, locked Clippy, 102 tests, pinned adapter compilation, and
  hosted CI are green.
- [x] No P3 math, P5 behavior, venue execution, Nautilus core, or copied CJ source changed.

## Reviewer focus

1. Attempt impossible cross-field construction and serde bypass of `TargetInventory`.
2. Try to authorize a target with stale or mismatched P3 snapshot identity.
3. Recompute both leg notionals at requested `q` and confirm the proposal uses their minimum.
4. Confirm increase economics/size checks cannot block reductions or authorize a larger clip.
5. Confirm no `RiskManager`, `RiskDecision`, `ExecutionIntent`, lifecycle, execution, or P5 path
   was added.

No P5 work is authorized by this package.
