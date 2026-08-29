# P3 Measurement Report

Status: implemented on `codex/p3-measurement`; stopped for GPT Gate 3

## Published implementation

- V2.1 mathematical and state semantics freeze:
  `2d4058af2dc91c5f0e55e038b9e1c0bd6f407990`
- Measurement contracts, configuration, model version, and fingerprint:
  `070c5939c4f8380fc9b04a854007728d751b0de6`
- Independent oriented-route executable L2 measurement:
  `46480739dc01732b9393953f9662ef537f028de1`
- Fair value, regime classification, and cost-adjusted opportunity measurement:
  `55f5d8d716e586aef5f531e82767882a121b41f8`
- Deterministic offline measurement replay:
  `cf02d3f2a6dd54f2346606bc280ae7551a7ea581`
- Hosted CI for the complete P3 implementation:
  [run 33260197588](https://github.com/F4uk/Riftbot-rs/actions/runs/33260197588), conclusion
  `SUCCESS`

## Measurement contracts and configuration

- `p3-measurement-v1` identifies the model semantics. A canonical SHA-256 fingerprint covers every
  measurement-affecting configuration field, including size, timing, fee, funding, fair-value, and
  regime assumptions. Funding route declaration order is canonicalized; unrelated recording and
  risk-limit fields do not change the fingerprint.
- Fixed-decimal domain values carry base quantity, price, notional, bps, duration, venue,
  instrument, pair, symbol, model version, and timestamps. Measurement output exposes executable
  leg prices, maximum executable quantity, raw premium, midline, deviation, expected round-trip
  fees, current depth impact, execution buffer, signed funding adjustment, separately named risk
  costs, tradable edge, ages/skew, confidence inputs, regime, validity, and rejection reason.
- `strategy.max_target_notional < risk.max_pair_notional` is validated as a configuration contract,
  but P3 never consumes it for sizing and never emits `TargetInventory`.

## Executable route measurement

For each explicit route `long A / short B`, the engine walks A asks to buy and B bids to sell the
configured base quantity. It rejects insufficient visible depth without extrapolation or a partial
fill assumption. The reverse route walks the opposite books independently.

```text
RawExecutablePremium = (sell_vwap_B / buy_vwap_A - 1) * 10_000
Deviation = RawExecutablePremium - FairValueMidline
TradableEdge = Deviation - expected_round_trip_fees - execution_buffer
               + signed_funding_adjustment - other_explicit_risk_costs
```

Executable prices are real L2 VWAP only. Midpoints never enter executable pricing. Current depth
impact is already embedded in VWAP and is disclosed, not deducted again. `fee_bps` is four expected
taker fills: both entry legs plus both exit legs. A missing fee is not zero.

Before measurement, both books must match the explicit route identity, remain canonical and
non-crossed, have healthy transport/recovery state, be no older than the caller-time limit, satisfy
receive-time skew, and contain sufficient depth. All timing is supplied by the caller.

## Fair value, regime, and opportunity

For `long A / short B`, epoch-aligned logical-time sampling uses only:

```text
mid_A = (best_bid_A + best_ask_A) / 2
mid_B = (best_bid_B + best_ask_B) / 2
ReferenceBasis = (mid_B / mid_A - 1) * 10_000
FairValueMidline = rolling median(valid synchronized ReferenceBasis samples)
```

Each oriented route has its own duration window and accepts at most one sample per canonical tick.
Invalid ticks are visible but never backfilled. Book-update frequency cannot add samples. The
window uses a rolling median and median absolute dispersion, with explicit warm-up and deterministic
logical-time eviction.

`RegimeFilter` only classifies measurement state as `normal`, `degraded`, `reduce_only`, or
`halted`. `OpportunityModel` packages measurement facts and evaluates economics. It does not size
positions, rank pairs, emit `TargetInventory`, or submit orders. Risk increase is economically
eligible only when validity is `valid`, the regime permits it, and `TradableEdge` is positive.
Funding `unavailable` produces no adjustment and no tradable edge; funding `disabled` is a visible
operator state with zero adjustment; funding `available` requires an explicit signed value.

The mandatory examples are executable tests:

- Example A: 20 bps raw premium minus 18 bps midline gives 2 bps deviation; 4 bps costs gives
  -2 bps edge and cannot increase risk.
- Example B: 43 bps raw premium minus 18 bps midline gives 25 bps deviation; 4 bps costs gives
  21 bps edge and the economic gate may permit a later GridInventory decision.

## Deterministic replay

`p3-measurement replay` accepts only a fully validated P2 `OfflineMarketDataOnly` replay report.
It rebuilds the P1 book/health path, applies events on recorded logical time, samples both configured
orientations, and emits an `OfflineMeasurementOnly` report. It has no adapter, account, executor,
order client, callback, or wall-clock input. Every analysis begins from empty state.

The report binds its output to the recording schema version and content SHA-256, replay end time,
model version, configuration fingerprint, interval, explicit routes, all tick outcomes, and every
non-healthy feed transition. Same recording plus same configuration and model version produces an
equal report; a measurement configuration change visibly changes the fingerprint and output.

## Real public evidence

Commands:

```text
cargo run --locked --features p1-connectivity --bin p1-connectivity -- record-validate <ignored-local-path>
cargo run --locked --bin p3-measurement -- replay <ignored-local-path> config/example.toml docs/evidence/P3_MEASUREMENT.json
```

The official pinned Hyperliquid and Lighter paths discovered 3 active Entropy/io, 103 active
trade.xyz/xyz, and 214 active Lighter perpetuals. The selected public SNDK segment recorded 3
initial plus 1 recovery Entropy books, 3 plus 1 trade.xyz books, and 9 plus 1 Lighter books. Both
official reconnect requests were accepted, both emitted `Reconnected`, every feed produced a
post-transition recovery book, and all three feeds ended `connected + fresh`.

The recording contained 36 events with content SHA-256
`03b1438fb8a186979b8ee9bc4ccb25009673b22fce41a97034f6a6347fc8fa6a`. P2 replayed it twice
identically with three final feeds. The local recording remains ignored and is not committed.

The committed `docs/evidence/P3_MEASUREMENT.json` is bound to that checksum and to configuration
fingerprint `b1ba6ffb4f7f08e71dacd5511a97148b011313717e850201eb59e94aff69834e`. It contains two
independent routes, 28 route ticks, and 15 explicit unhealthy/recovery-state observations. Two
route ticks had executable L2 measurements but remained `warming_up`; 16 ticks rejected unhealthy
feeds and 10 rejected stale books. The example configuration requires 300 fair-value samples,
declares funding unavailable, and does not assert venue fees, so this short live segment correctly
produced zero valid/increase-risk opportunities rather than inventing a midline or zero costs.

## Tests and verification

`cargo test --locked --all-targets --all-features` passed 75 tests and failed 0: 58 library tests,
11 P2 integration tests, and 6 P3 measurement replay integration tests. Binary targets contain no
unit tests.

The P3 coverage includes:

- exact top-level and multi-level VWAP, depth boundary/shortfall, precision, direction, midpoint
  separation, and no depth-cost double count;
- stale, future, unhealthy, skewed, empty, crossed, insufficient-depth, missing-fee, and
  funding-unavailable fail-closed paths;
- expected round-trip fee horizon and signed funding economics;
- canonical reference math, interval enforcement, one sample per tick, invalid-tick exclusion,
  rolling median/dispersion, duration eviction, outlier robustness, warm-up, and route isolation;
- all four regimes, extreme deviation halt, and the mandatory Gate 3 examples;
- identical offline measurement replay, independent reverse route output, preserved logical tick
  times, visible configuration fingerprint changes, disconnect/reconnect rejection evidence, and
  no execution capability.

| Command | Result |
|---|---|
| `python scripts/ci_policy.py all` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --all-targets --all-features` | 75 passed; 0 failed |
| `cargo check --locked --features nautilus-adapters` | pass |
| real three-venue `record-validate` | pass; 36 events; replay twice identical |
| offline P3 measurement replay | pass; 2 routes; 28 ticks; no valid risk increase |

## Scope audit and known limitations

No GridInventory behavior, `TargetInventory`, P4 inventory management, multi-pair ranking, order
submission, execution, account connectivity, custom venue transport, or Nautilus core change was
added. Existing future execution domain contracts are untouched and are not imported by the P3
measurement path.

The live segment is deliberately short relative to the configured 300-sample warm-up and its
example fee/funding inputs are deliberately unverified. It is connectivity, deterministic replay,
and fail-closed measurement evidence—not a claim of profitability or trade readiness. Production
fee and signed funding sources remain later integration/configuration work and must be explicit
before any positive `TradableEdge` can exist.

P3 stops here for GPT Gate 3.
