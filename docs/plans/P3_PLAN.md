# P3 Measurement Plan

Scope: Measurement only. Base is Gate 2-approved `main` commit
`792718b9d348eea0c3e3eeb00c1c458784b9b10e`. Work stops at GPT Gate 3.

## Frozen terminology and route sign

For an explicit route `long venue A / short venue B` and requested base quantity `q`:

- buy VWAP walks A asks for `q`;
- sell VWAP walks B bids for `q`;
- `raw_executable_premium_bps = (sell_vwap / buy_vwap - 1) * 10_000`;
- positive raw premium means B can currently be sold above the executable cost of buying A;
- `midline_bps` is the route-specific rolling median of valid synchronized raw premiums;
- `deviation_bps = raw_executable_premium_bps - midline_bps`;
- `net_actionable_edge_bps = deviation_bps - fee_bps - execution_buffer_bps +
  funding_adjustment_bps`.

Depth impact is disclosed as the sum of buy-VWAP impact versus best ask and sell-VWAP impact
versus best bid. It is already embedded in raw executable VWAP premium and is not subtracted again.
Funding unavailable is not zero: net edge remains unavailable. Explicitly disabled funding uses a
visible disabled state and contributes zero by operator policy.

## Task 1 — Measurement contracts and configuration

- Expand measurement output with typed quantity, VWAP, raw premium, midline, deviation, fees,
  depth impact, execution buffer, funding state, net edge, feed ages/skew, fair-value confidence,
  regime, validity, and rejection reason.
- Add typed maximum book age/skew, requested size, route fees, funding state, fair-value window,
  and explainable regime thresholds.
- Add a stable model version and SHA-256 fingerprint over measurement-affecting configuration.

Verification: configuration invariant and fingerprint-change tests.

## Task 2 — SpreadEngine and pair-quality gate

- Validate health, caller-time book ages, receive-time skew, canonical non-crossed books, and visible
  quantity on both legs.
- Walk asks for buys and bids for sells with fixed-decimal multi-level VWAP.
- Reject at insufficient depth without extrapolation, midpoint, last price, or partial pretending.
- Calculate both directions and expose maximum executable size and depth impact.

Verification: both directions, top level, multi-level, exact boundary, insufficient depth, stale,
skew, sign, precision, fee removal, and no double-counting tests.

## Task 3 — FairValue, Regime, and Opportunity

- Maintain route-isolated rolling medians, median absolute dispersion, warm-up/minimum samples,
  deterministic eviction, and invalid-observation exclusion.
- Classify only measurement regime (`normal`, `degraded`, `reduce_only`, `halted`) from data quality,
  confidence, deviation, dispersion, and midline instability.
- Combine market facts and explicit cost assumptions into a signal-only opportunity object.

Verification: warm-up, rolling median, eviction, outlier robustness, isolation, regime shift, all
four regimes, degraded data, volatility, extreme deviation, and confidence tests.

## Task 4 — Deterministic measurement replay and live evidence

- Rebuild P1 state from a validated P2 replay report and emit both configured route directions.
- Use recorded event/observation timestamps only; never consult wall-clock time.
- Demonstrate same recording + same config + same model version gives identical output and changed
  measurement config produces a visible fingerprint change.
- Record a fresh public SNDK segment through official P1/P2 paths, generate signal-only
  Entropy/Lighter measurement evidence, and include invalid/reconnect rejection reasons.

## Task 5 — Gate 3

- Run repository policy, format, locked Clippy, locked all-feature tests, and pinned adapter check.
- Publish `docs/stages/P3_REPORT.md`, `docs/gates/GATE_3_REVIEW.md`, and machine-readable measurement
  evidence; push and wait for hosted CI.
- Stop before P4.

## Explicit exclusions

No CJ Grid behavior, TargetInventory decisions, order submission, execution, live RiskManager
expansion, position opening/closing, P4 work, custom venue transport, or Nautilus core changes.
