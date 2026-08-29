# P3 Measurement Plan

Scope: Measurement only. Base is Gate 2-approved `main` commit
`792718b9d348eea0c3e3eeb00c1c458784b9b10e`. Work stops at GPT Gate 3.

## Frozen terminology and route sign

For an explicit route `long venue A / short venue B` and requested base quantity `q`:

- buy VWAP walks A asks for `q`;
- sell VWAP walks B bids for `q`;
- `raw_executable_premium_bps = (sell_vwap / buy_vwap - 1) * 10_000`;
- positive raw premium means B can currently be sold above the executable cost of buying A;
- `RawExecutablePremium` is the executable cross-venue premium from real L2 VWAP and is exposed as
  `raw_executable_premium_bps`;
- `FairValueMidline` is the route-specific persistent/natural spread baseline and is exposed as
  `midline_bps`;
- `Deviation = RawExecutablePremium - FairValueMidline` and is exposed as `deviation_bps`;
- `TradableEdge = DirectionAdjusted(Deviation) - Fees - ExecutionUncertaintyBuffer +
  SignedFundingAdjustment - OtherExplicitRiskCosts` and is exposed as `tradable_edge_bps`.

Depth impact is disclosed as the sum of buy-VWAP impact versus best ask and sell-VWAP impact
versus best bid. It is already embedded in raw executable VWAP premium and is not subtracted again.
Absolute cross-venue spread is not expected arbitrage profit, and the persistent natural spread
must not be counted as edge. Funding is a signed economic adjustment to the spread-convergence
edge, not an independent V1 Funding Arbitrage strategy. Funding unavailable is not verified zero:
tradable edge remains unavailable. Explicitly disabled funding uses a visible disabled state and
contributes zero by operator policy.

`GridInventoryModel` consumes `Deviation` for segmented target sizing. `Opportunity` and `Risk`
consume cost-adjusted `TradableEdge` when deciding whether increasing risk is economically allowed.
The CJ historical-data logic is the `FairValueMidline` / natural-spread baseline estimator, not a
second strategy. CJ segmented-grid logic is the sole inventory/position-sizing strategy:

```text
historical observations
-> FairValueMidline
-> Deviation
-> GridInventoryModel
-> TargetInventory
```

## Task 1 — Measurement contracts and configuration

- Expand measurement output with typed quantity, VWAP, `raw_executable_premium_bps`, `midline_bps`,
  `deviation_bps`, `fee_bps`, `depth_impact_bps`, `execution_buffer_bps`,
  `funding_adjustment_bps`, separately auditable other explicit risk costs,
  `tradable_edge_bps`, feed ages/skew, fair-value confidence, regime, validity, and rejection reason.
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
- Combine market facts and explicit cost assumptions into a signal-only opportunity object;
  increasing risk requires a positive, available cost-adjusted `TradableEdge`.

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

- Add mandatory contract examples/tests:

  - A: `midline = 18 bps`, `current executable premium = 20 bps`, `total costs = 4 bps` gives
    `deviation = 2 bps`, `tradable edge = -2 bps`, and MUST NOT increase risk.
  - B: `midline = 18 bps`, `current executable premium = 43 bps`, `total costs = 4 bps` gives
    `deviation = 25 bps`, `tradable edge = 21 bps`, and the economic gate may permit
    `GridInventoryModel` to increase target.

  Both examples assume positive direction-adjusted deviation, zero signed funding adjustment, and
  total costs covering every applicable fee, execution buffer, and other explicit risk cost.
- Run repository policy, format, locked Clippy, locked all-feature tests, and pinned adapter check.
- Publish `docs/stages/P3_REPORT.md`, `docs/gates/GATE_3_REVIEW.md`, and machine-readable measurement
  evidence; push and wait for hosted CI.
- Stop before P4.

## Explicit exclusions

No CJ Grid behavior, TargetInventory decisions, order submission, execution, live RiskManager
expansion, position opening/closing, P4 work, custom venue transport, or Nautilus core changes.
