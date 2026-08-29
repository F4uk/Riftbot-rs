# GPT Gate 4 Review Package

Decision requested: review P4 CJ Target Inventory only.

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
- Hosted CI:
  [run 33263554823](https://github.com/F4uk/Riftbot-rs/actions/runs/33263554823), conclusion
  `SUCCESS`
- Tests: 92 passed, 0 failed

## Gate checklist

- [x] `TargetFraction` accepts only `[0, 1]` at construction and deserialization boundaries.
- [x] Negative target fractions cannot be introduced through config or `TargetInventory` serde.
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
- [x] Outputs are proposals, not `RiskDecision`, `ExecutionIntent`, basket, or order commands.
- [x] Repository policy, formatting, locked Clippy, 92 tests, pinned adapter compilation, and
  hosted CI are green.
- [x] No P3 math, P5 behavior, venue execution, Nautilus core, or copied CJ source changed.

## Reviewer focus

1. Attempt negative construction and serde bypass of `TargetFraction` / `TargetInventory`.
2. Recompute all floor-step boundaries and the strategy-cap per-leg target notionals.
3. Confirm only the pair arbiter can emit one final target and ambiguity emits none.
4. Confirm EffectiveActual includes reserved and pending exposure before proposing an increase.
5. Confirm increase economics/size checks cannot block reductions or authorize a larger clip.
6. Confirm reversal is a reduction-only first step.
7. Confirm there is no RiskManager, execution, or order path in P4.

No P5 work is authorized by this package.
