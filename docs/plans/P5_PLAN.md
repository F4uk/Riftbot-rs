# P5 Risk-Only Plan

Status: frozen for implementation on `codex/p5-risk`

## Scope and authority

P5 consumes P4 `InventoryDecision` proposals and emits deterministic, auditable hard-risk
authorization. The authority order is `Risk > Strategy > Profit`. `Regime`, per-decision
`RiskDecision`, and persistent/global `KillState` remain separate types, and the effective result
is always the most restrictive applicable permission.

P5 does not create `ExecutionIntent`, execution baskets, child orders, venue requests, or order
lifecycle state. It has no Nautilus dependency and accepts caller-supplied logical time only.

## Frozen V1 risk policies

- P4 matched notional is **per leg**. Pair limits use matched per-leg notional; venue limits use
  absolute notional at each venue. No implicit two-leg gross conversion is used.
- An increase is never larger than P4's proposed change or P3's safe matched-notional cap.
- Measurement recency uses `IncreaseSizeBasis.observed_at` and configured
  `risk.max_measurement_age_ms`. Future timestamps and ages strictly above the limit fail closed;
  exactly the configured maximum age is accepted.
- `Regime::Degraded` clips otherwise valid increases by configured
  `risk.degraded_authorization_fraction`; all health and hard-limit checks still apply.
- `Regime::ReduceOnly` denies increases and permits risk reduction. `Regime::Halted` requires
  flattening policy and never permits an increase.
- `KillState::PauseNew` denies increases; `ReduceOnly` allows only reduction; `Flatten` requires
  movement toward zero; and `Halt` blocks P4 routine actions. Future explicitly typed emergency
  safety behavior belongs to P6 and is not invented here.
- Reaching or exceeding `risk.max_session_loss` requires the configured
  `risk.session_loss_action` (`flatten` or `halt`). P5 consumes signed session PnL and does not
  calculate venue PnL.
- Required market/feed, venue, account-stream, reconciliation, state-freshness, operations, and
  latency health must be explicit and healthy for an increase. Missing, unknown, stale, degraded,
  or unhealthy required facts fail closed. Reductions do not require favorable or fresh entry
  economics.
- Current, reserved, and pending exposure is checked before projecting the authorized change.
  Pair, both venue, global-delta, and session-loss limits are independent; delta neutrality never
  waives a venue limit.

## Tasks

1. Extend typed risk configuration with logical-time freshness, degraded clipping, and session-loss
   escalation policy; add a stable risk-config fingerprint.
2. Harden risk domain contracts: distinct enums, explicit health/exposure/PnL inputs, reason codes,
   validated `RiskAssessment` construction/deserialization, and auditable kill transitions.
3. Implement deterministic kill-state transition validation using caller timestamps.
4. Implement `RiskManager` validation, precedence, recency, health, projected hard-limit, clipping,
   and fail-closed arithmetic behavior.
5. Add happy-path, boundary, fault, determinism, serde, scope, and no-wall-clock tests.
6. Run repository policy, formatting, locked Clippy/tests, and pinned Nautilus adapter compilation;
   publish `P5_REPORT.md` and `GATE_5_REVIEW.md`; stop before P6.

## Explicit exclusions

No execution intent generation, `ExecutionBasketCoordinator`, residual handler, Nautilus order
submission, child-order lifecycle, P6 behavior, PnL calculation, capital optimization, or venue
account integration is authorized by this plan.
