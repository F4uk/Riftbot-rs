# P5 Deterministic Hard-Risk Report

Status: implemented on `codex/p5-risk`; stopped for GPT Gate 5

## Green base and implementation commits

- Gate 4-approved P4 merged to `main`: `40c65dc29441346b058af87b74d13a53348c5618`
- Green merged-main CI:
  [run 33281603915](https://github.com/F4uk/Riftbot-rs/actions/runs/33281603915), conclusion
  `SUCCESS`
- Frozen deterministic P5 policy/config: `7265c2a`
- Hard-risk authorization and fault matrix: `619f47b`
- Risk-audit cross-field serde hardening: `f2f52ba`
- Final P5 evidence commit: the commit containing this report; SHA is reported after push
- Hosted CI for the final P5 branch: required before Gate 5 handoff and reported with the final SHA

## Authority and stage boundary

P5 freezes `Risk > Strategy > Profit`. The existing concepts remain separate:

- `Regime` is a market/system classification input.
- `RiskDecision` is authorization for exactly one decision.
- `KillState` is persistent/global operational state with highest authority.

The effective result is the most restrictive applicable disposition. Large deviation, positive
edge, or a large P4 target cannot override a restrictive regime, kill state, health failure, session
loss state, or hard limit.

`RiskManager` consumes `InventoryDecision`; it does not generate strategy targets. An increase
requires P4 economic permission but P5 independently authorizes it. P5 never increases P4's
proposed matched notional and never exceeds P3's safe two-leg matched-notional cap. The P5 output is
a validated `RiskAssessment`, not an `ExecutionIntent`, basket, child order, or venue request.

## Frozen P5 configuration

The risk configuration now includes:

```text
max_venue_notional
max_pair_notional
max_global_delta
max_session_loss
max_measurement_age_ms
degraded_authorization_fraction
session_loss_action = flatten | halt
```

All limits and policies are validated without implicit defaults. The degraded fraction must be
strictly between zero and one. A canonical SHA-256 risk-config fingerprint covers only the schema
version and risk-policy fields and is frozen into every assessment.

## Measurement recency and size authority

For `IncreaseRisk`, P5 uses `IncreaseSizeBasis.observed_at` and caller-supplied `evaluated_at`.
Future timestamps fail closed. Age strictly above `max_measurement_age_ms` fails closed; exactly the
configured maximum is accepted. No `std::time`, `SystemTime`, or implicit wall clock is used.

The P4 proposal is revalidated at the risk boundary. Its requested/proposed signs, route/effective
actual identity, measurement basis, and safe cap shape must be coherent. P5 records the safe cap and
validates:

```text
authorized_change_per_leg <= P4 proposed_change_per_leg
authorized_change_per_leg <= P3 measured_matched_notional_cap
```

`Regime::Degraded` is the sole V1 risk clip policy: an otherwise valid increase is multiplied by
`degraded_authorization_fraction` before hard-limit projection. Normal policy never enlarges a
proposal. Reductions ignore stale, missing, or unfavorable entry economics.

## Hard-limit units and projection

P4 change/target notional is matched notional **per leg**. P5 therefore evaluates:

- `max_pair_notional` against effective matched pair notional per leg;
- `max_venue_notional` independently against absolute notional at each selected venue;
- `max_global_delta` against the absolute effective signed USD delta; and
- `max_session_loss` against loss derived from explicit signed session PnL.

Actual, reserved, and pending components use checked fixed-decimal addition. The candidate change is
projected onto the pair and both venues before authorization. A matched two-leg proposal contributes
zero modeled global delta, but existing actual/reserved/pending global delta is still checked. Thus
delta neutrality never waives a venue limit. Equality at venue/pair/global limits is allowed; session
loss equality triggers the configured restrictive action as required.

Each assessment preserves current, candidate-projected, and authorized-projected exposure. A denied
increase shows the breached candidate projection while its authorized projection remains current.
Arithmetic overflow produces an explicit fail-closed reason and zero authorization.

## Health and operational facts

There is no healthy default. `IncreaseRisk` requires explicit health for both route venues and the
system:

- market/feed health;
- venue connectivity;
- account/private-stream health;
- reconciliation health;
- state freshness;
- latency/operational health;
- unknown-operation count; and
- proof that outstanding-operation exposure is included in effective inventory.

Missing, duplicated, unknown, stale, degraded, or unhealthy required facts deny an increase.
Reductions do not require positive/fresh entry economics or healthy market-data evidence, while
exposure identity remains required to prove the action moves toward lower risk.

## Regime, kill state, and session loss

| Input | IncreaseRisk | Reduction |
|---|---|---|
| `Regime::Normal` | normal hard-risk evaluation | allowed if proposal/exposure is valid |
| `Regime::Degraded` | configured conservative clip, then all gates | no entry-edge dependency |
| `Regime::ReduceOnly` | `ReduceOnly`, zero authorization | authorized as reduce-only |
| `Regime::Halted` | `FlattenRequired`, zero authorization | flattening reduction permitted |
| `KillState::Ready` | normal evaluation | normal reduction |
| `KillState::PauseNew` | denied | reduction allowed |
| `KillState::ReduceOnly` | zero authorization | reduce-only authorization |
| `KillState::Flatten` | `FlattenRequired`, zero increase | reduction toward zero authorized |
| `KillState::Halt` | `HaltRequired`, zero authorization | routine P4 action blocked |

Reaching or exceeding the signed-session-PnL loss limit requires the configured persistent-state
policy: `FlattenRequired` or `HaltRequired`. P5 consumes PnL facts; it does not calculate venue PnL.

`KillStateMachine` validates an explicit graph, rejects timestamp regression and unsafe direct
recovery such as `Halt -> Ready`, and records `from`, `to`, reason, logical timestamp, and trigger.
It never creates an order or venue action.

## Audit and validation

`RiskAssessment` preserves:

- decision ID, caller evaluation timestamp, P4 input action;
- requested, proposed, and authorized matched notional per leg;
- `RiskDecision`, `Regime`, and `KillState`;
- ordered typed reason codes and deterministic human-readable explanation;
- measurement age and P3 safe matched-notional cap for increases;
- relevant current/candidate/authorized pair, venue, global-delta, and session-PnL facts; and
- configured hard limits, session escalation state, and risk-config fingerprint.

Construction and deserialization share the same validation. Serde cannot enlarge authorization,
exceed the recorded P3 cap, omit required approved-increase facts, attach non-zero size to a denied
decision, or make authorized exposure projections disagree with the authorization amount.

## Tests and verification

`cargo test --locked --all-targets --all-features` passes 133 tests and fails 0:

- 89 library tests;
- 11 P2 recording/replay integration tests;
- 6 P3 measurement replay integration tests; and
- 27 P5 hard-risk integration tests.

P5 coverage includes all requested happy-path, restrictive-state, freshness, health, hard-limit,
projection, session-loss, overflow, determinism, serde, transition, logical-time, and no-execution
invariants. It also tests configured degraded clipping, configured session-loss halt escalation,
P3 safe-cap revalidation, candidate-versus-authorized audit projections, and unknown/outstanding
operation handling.

| Command | Result |
|---|---|
| `python scripts/ci_policy.py all` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --all-targets --all-features` | 133 passed; 0 failed |
| `cargo check --locked --features nautilus-adapters` | pass |

## Scope audit and limitations

P5 defines deterministic health, exposure, PnL, and kill-state facts but does not connect account
streams, calculate PnL, reconcile venue truth, optimize capital, or manage an order lifecycle. Those
facts are supplied by later integration and fail closed for increases when required information is
absent.

No `ExecutionIntent` generation, `ExecutionBasketCoordinator`, residual handler, Nautilus order
submission, child-order lifecycle, P6 behavior, or P3/P4 mathematics change is present. P5 stops
here for GPT Gate 5.
