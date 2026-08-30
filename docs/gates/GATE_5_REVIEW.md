# GPT Gate 5 Review Package

Decision requested: review P5 deterministic hard-risk authorization only.

## Review inputs

- Governing semantics: `PROJECT_TASKBOOK.md` V2.1 plus the user-authorized P5 risk contract
- P5 plan: `docs/plans/P5_PLAN.md`
- Stage report: `docs/stages/P5_REPORT.md`
- Risk domain and validated assessment: `src/domain/risk.rs`
- Checked hard-limit arithmetic: `src/risk/limits.rs`
- Risk authorization and kill transitions: `src/risk/manager.rs`
- Fault/invariant matrix: `tests/p5_risk.rs`
- P4 green merge base: `40c65dc29441346b058af87b74d13a53348c5618`
- Green merged-main CI:
  [run 33281603915](https://github.com/F4uk/Riftbot-rs/actions/runs/33281603915), conclusion
  `SUCCESS`
- Final P5 commit: the commit containing this package; SHA is reported after push
- Hosted CI for final P5: required before handoff and reported with the final SHA
- Tests: 133 passed, 0 failed

## Gate checklist

- [x] `Regime`, `RiskDecision`, and persistent/global `KillState` remain distinct.
- [x] Most-restrictive-wins precedence is deterministic; risk outranks strategy/profit.
- [x] P4 `InventoryDecision` is input only and cannot authorize itself.
- [x] P5 never enlarges P4 proposed size or exceeds P3 safe matched-notional size.
- [x] Increase freshness uses `IncreaseSizeBasis.observed_at` and caller logical time only.
- [x] Future and too-old measurements fail closed; the exact maximum-age boundary is tested.
- [x] Reductions do not depend on positive, available, or fresh entry economics.
- [x] Pair limits use matched notional per leg; venue limits use each venue's absolute notional.
- [x] Actual, reserved, and pending exposure is included before candidate projection.
- [x] Pair, each venue, global delta, and signed-session-loss limits are checked independently.
- [x] Equality behavior is explicit: exposure/delta limits allow equality; session loss triggers at
  equality.
- [x] Delta neutrality cannot bypass a venue limit.
- [x] Missing or non-healthy feed, connectivity, account stream, reconciliation, state, latency,
  and operation facts fail closed for increases.
- [x] `Normal`, conservatively clipped `Degraded`, `ReduceOnly`, and `Halted` policies are tested.
- [x] `Ready`, `PauseNew`, `ReduceOnly`, `Flatten`, and `Halt` policies are tested.
- [x] Session loss can require configured flatten or halt state.
- [x] Kill transitions are caller-timestamped, audited, graph-validated, and reject regressions.
- [x] `RiskAssessment` preserves identity, logical time, action, all sizes, authority states, typed
  reasons, exposures, measurement age/cap, limits, and config fingerprint.
- [x] Construction and serde reject enlarged or cross-field-inconsistent authorizations.
- [x] Checked fixed-decimal overflow fails closed.
- [x] Same input produces identical output; deterministic risk logic has no wall clock.
- [x] Repository policy, formatting, locked Clippy, 133 tests, pinned adapter compilation, and the
  merged-main hosted baseline are green.
- [x] No intent generation, execution basket, order submission/lifecycle, P6 behavior, or Nautilus
  dependency was added to P5.

## Reviewer focus

1. Try to bypass assessment serde with an authorization above P4/P3 size or inconsistent exposure.
2. Recompute projected actual + reserved + pending pair/venue exposure and exact limit boundaries.
3. Confirm restrictive regime/kill/session states cannot be overridden by positive economics.
4. Remove or degrade each health fact and confirm increases fail closed while reductions do not
   depend on entry economics.
5. Supply future/stale timestamps and verify only caller logical time determines recency.
6. Confirm routine P5 outputs cannot reach `ExecutionIntent`, an order API, or Nautilus.

No P6 work is authorized by this package.
