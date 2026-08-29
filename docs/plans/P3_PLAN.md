# P3 Measurement Plan

Scope: Measurement only. Base is Gate 2-approved `main` commit
`792718b9d348eea0c3e3eeb00c1c458784b9b10e`. Work stops at GPT Gate 3.

This plan implements the `PROJECT_TASKBOOK.md` V2.1 Mathematical & State Semantics Freeze.

## Frozen terminology and oriented routes

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
- positive `Deviation` means convergence economics favor this explicit route;
- `TradableEdge = Deviation - Fees - ExecutionUncertaintyBuffer +
  SignedFundingAdjustment - OtherExplicitRiskCosts` and is exposed as `tradable_edge_bps`.

The reverse route `long venue B / short venue A` is calculated independently from its own buy/sell
VWAP, reference samples, midline, deviation, and costs. `PairId` identifies the pair/symbol;
`long_venue` and `short_venue` carry route orientation. Neither a direction-adjustment heuristic nor
the sign of `target_fraction` may encode direction.

Depth impact is disclosed as the sum of buy-VWAP impact versus best ask and sell-VWAP impact
versus best bid. It is already embedded in raw executable VWAP premium and is not subtracted again.
Absolute cross-venue spread is not expected arbitrage profit, and the persistent natural spread
must not be counted as edge. Funding is a signed economic adjustment to the spread-convergence
edge, not an independent V1 Funding Arbitrage strategy. Funding unavailable is not verified zero:
tradable edge remains unavailable. Explicitly disabled funding uses a visible disabled state and
contributes zero by operator policy.

For the increase-risk gate, `fee_bps` is expected round-trip trading fees: entry two-leg fees plus
expected exit two-leg fees. Entry-only fees are not total trading cost. The execution uncertainty
buffer may cover latency beyond observed L2, adverse execution movement, and conservative expected
exit microstructure/slippage, but it cannot duplicate current observed depth impact.

## Frozen fair-value sampling

For `long venue A / short venue B`:

```text
mid_A = (best_bid_A + best_ask_A) / 2
mid_B = (best_bid_B + best_ask_B) / 2
ReferenceBasis = (mid_B / mid_A - 1) * 10_000
FairValueMidline = rolling robust median of valid synchronized ReferenceBasis samples
```

Midpoints are baseline-only and can never be executable prices. Sampling uses
`fair_value.sample_interval_ms`, `fair_value.window_duration_ms`,
`fair_value.minimum_samples`, and `fair_value.max_sample_age_ms`. Unix-epoch-aligned fixed sampling
ticks (`tick_ms % sample_interval_ms == 0`) come from injectable logical time (Nautilus clock live,
recorded/replay clock in replay), accept at most one valid synchronized sample per oriented route per
tick, never backfill invalid ticks, and evict by logical window duration. Raw book-update frequency
cannot weight the median.

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

Downstream contract only (not P3 implementation): `target_fraction` is non-negative; direction is
`TargetDirection::Flat` or `TargetDirection::LongShort { long_venue, short_venue }`. Grid 100% maps
to matched per-leg `strategy.max_target_notional`, never `risk.max_pair_notional`, and configuration
requires `strategy.max_target_notional < risk.max_pair_notional`.

## Task 1 — Measurement contracts and configuration

- Expand measurement output with typed quantity, VWAP, `raw_executable_premium_bps`, `midline_bps`,
  `deviation_bps`, `fee_bps`, `depth_impact_bps`, `execution_buffer_bps`,
  `funding_adjustment_bps`, separately auditable other explicit risk costs,
  `tradable_edge_bps`, feed ages/skew, fair-value confidence, regime, validity, and rejection reason.
- Add typed maximum book age/skew, requested size, route fees, funding state, fair-value window,
  and explainable regime thresholds.
- Use config schema v2: replace update-count weighting with the four frozen fair-value timing
  fields, rename the old estimated-slippage assumption to `execution_buffer_bps`, and add
  `strategy.max_target_notional` with strict Risk headroom validation.
- Add a stable model version and SHA-256 fingerprint over measurement-affecting configuration.

Verification: configuration invariant and fingerprint-change tests.

## Task 2 — SpreadEngine and pair-quality gate

- Validate health, caller-time book ages, receive-time skew, canonical non-crossed books, and visible
  quantity on both legs.
- Walk asks for buys and bids for sells with fixed-decimal multi-level VWAP.
- Reject at insufficient depth without extrapolation, midpoint, last price, or partial pretending.
- Calculate both oriented routes independently and expose maximum executable size and depth impact.
- Calculate expected round-trip `fee_bps`; never label entry-only fees as total trading cost.

Verification: both directions, top level, multi-level, exact boundary, insufficient depth, stale,
skew, sign, precision, round-trip fee horizon, midpoint-not-executable, and no-double-counting tests.

## Task 3 — FairValue, Regime, and Opportunity

- Generate route-isolated canonical `ReferenceBasis` samples on logical-time ticks; maintain rolling
  medians, median absolute dispersion, warm-up/minimum samples, deterministic duration eviction,
  and invalid-observation exclusion without book-update-frequency weighting.
- Classify only measurement regime (`normal`, `degraded`, `reduce_only`, `halted`) from data quality,
  confidence, deviation, dispersion, and midline instability.
- Combine market facts and explicit cost assumptions into a signal-only opportunity object;
  increasing risk requires a positive, available cost-adjusted `TradableEdge`.
- Keep `OpportunityModel` measurement-only: it cannot emit `TargetInventory`, size positions, rank
  V2 multi-pair candidates, or submit orders.

Verification: canonical reference math, deterministic interval sampling, missing/stale tick
rejection, no update-frequency weighting, independent route windows, warm-up, rolling median,
eviction, outlier robustness, isolation, regime shift, all four regimes, degraded data, volatility,
extreme deviation, and confidence tests.

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

  Both examples apply to one explicit oriented route, assume zero signed funding adjustment, and
  total costs cover expected round-trip fees, execution buffer, and every other explicit risk cost.
- Run repository policy, format, locked Clippy, locked all-feature tests, and pinned adapter check.
- Publish `docs/stages/P3_REPORT.md`, `docs/gates/GATE_3_REVIEW.md`, and machine-readable measurement
  evidence; push and wait for hosted CI.
- Stop before P4.

## Explicit exclusions

No CJ Grid behavior, TargetInventory decisions, order submission, execution, live RiskManager
expansion, position opening/closing, P4 work, custom venue transport, or Nautilus core changes.

Later-stage ownership remains frozen but unimplemented in P3: P4 owns InventoryManager and
Target-vs-EffectiveActual; Regime is classification input to RiskDecision; KillState is persistent
global highest authority and the most restrictive permission wins; V1 permits at most one active
risk-increasing intent per pair unless reserved/pending exposure is in EffectiveInventory; P6 intent
purposes are `IncreaseRisk`, `ReduceRisk`, `ResidualHedge`, and `EmergencyFlatten`, with only
`IncreaseRisk` requiring positive TradableEdge; P7 owns PnL foundations, P8 shadow validation, and
P9 live acceptance. Every decision-affecting timer uses injectable logical time.
