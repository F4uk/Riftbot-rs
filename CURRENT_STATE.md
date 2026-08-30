# Current State and Gap Analysis

Status date: 2026-08-30

Governing contract: `PROJECT_TASKBOOK.md` V2.1 Mathematical & State Semantics Freeze

## Repository baseline before P0

The repository contained only `PROJECT_TASKBOOK.md`. It was not a Git repository and had no
Rust manifest, lockfile, source tree, tests, CI workflow, configuration schema, architecture
document, upstream-source register, or prior stage reports. A repository-wide scan found no
credential-like material outside the taskbook.

The available local toolchain is Rust 1.98.0 with Cargo, rustfmt, and Clippy 1.98.0.

## Current architecture

There was no implementation architecture at baseline. P0 establishes a single Rust library with:

- Nautilus-coupled code restricted to edge modules.
- Pure domain identifiers, numeric units, market, spread, inventory, risk, execution, and audit
  record types.
- Typed, validated, secret-free configuration.
- Empty responsibility modules for later stages without implementing P1-P9 behavior.

## Stage evidence at baseline

| Stage | Baseline status | Evidence |
|---|---|---|
| P0 Foundation | Not started | No Cargo project, source, CI, source pin, or P0 documents |
| P1-P9 | Not started | No implementation files |

No completed stage could be inferred from the taskbook alone.

## Gap analysis

### P0 gaps

- Create a Rust 2024 project pinned to Rust 1.98.0.
- Select and lock an exact Nautilus dependency reference.
- Record Nautilus, QuantGuy, and CJ provenance and license boundaries.
- Add typed configuration, domain IDs/types, and the frozen module boundaries.
- Add formatting, lint, test, adapter-compatibility, and secret-scan CI gates.
- Generate a lockfile and demonstrate reproducible dependency resolution.

### P1-P9 gaps

All connectivity, recording/replay, measurement, inventory strategy, risk enforcement, execution,
reconciliation, and live-readiness work remains absent. These gaps are deliberately out of scope
for P0.

## Technical risks

1. The required Hyperliquid HIP-3 and Lighter Rust adapters coexist only on the inspected current
   Nautilus Rust line, so an old stable Python-era tag is not an adequate compatibility choice.
2. Adapter presence and compilation do not prove live venue behavior; endpoint, instrument, and
   reconnect checks remain P1 acceptance work.
3. CJ publishes no repository license metadata. Any source copying or translation would create an
   unacceptable IP risk; only the taskbook's abstract grid concepts may be independently built.
4. There was no Git history at baseline; the Gate 0 publication creates the initial P0 commit on
   `main` without introducing P1 work.

## Upstream pin status

The P0-selected Nautilus reference is
`e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c`. The commit exists, identifies Nautilus workspace
version 0.63.0, requires Rust 1.98.0, and contains the official `nautilus-hyperliquid` and
`nautilus-lighter` crates. Cargo must resolve it by exact `rev`; no branch dependency is permitted.

QuantGuy and CJ references from the taskbook were also verified. Full provenance and accepted or
rejected concepts are recorded in `UPSTREAM_SOURCES.md`.

## Recommended gate

Start at P0 and stop at GPT Gate 0. Do not begin P1 until Gate 0 is reviewed.

## Post-P0 status

P0 implementation is complete and awaiting GPT Gate 0 review. The repository now has:

- A Rust 2024 library and lockfile pinned to Rust 1.98.0.
- Exact-revision Nautilus model and optional official Hyperliquid/Lighter adapter dependencies.
- Pure domain types for IDs, fixed-decimal units, normalized books, spread snapshots,
  opportunities, inventory, risk, V1 execution baskets, and decision audit records.
- A typed, validated configuration whose only runtime mode is `signal_only` and whose schema has no
  secret fields.
- Frozen responsibility modules, architecture documentation, and a GitHub Actions P0 workflow.
- Passing format, Clippy, 14-test, adapter-compile, exact-pin, panic-pattern, and credential-pattern
  checks.

| Stage | Current status | Evidence |
|---|---|---|
| P0 Foundation | Implemented; Gate 0 pending | `docs/stages/P0_REPORT.md`, `Cargo.lock`, local gates green |
| P1-P9 | Not started | Responsibility markers only; no later-stage behavior exists |

The next allowed action is GPT Gate 0 review. P1 connectivity must not begin unless Gate 0 permits
it.

## Post-P1 status

GPT Gate 0 passed, and P1 Connectivity is now implemented on `codex/p1-connectivity` and awaiting
GPT Gate 1. The repository now additionally has:

- Official pinned-adapter discovery for Entropy/io, trade.xyz/xyz, and Lighter Mainnet.
- A live public depth-10 probe with exchange/receive timestamps and forced reconnect recovery.
- Deterministic `MarketNormalizer` and versioned `BookStore` freshness/health behavior.
- Evidence-backed selection of exactly one V1 symbol, `SNDK`.
- P1 evidence, report, and Gate 1 review materials.

| Stage | Current status | Evidence |
|---|---|---|
| P0 Foundation | Gate 0 passed | `docs/stages/P0_REPORT.md` |
| P1 Connectivity | Implemented; Gate 1 pending | `docs/stages/P1_REPORT.md`, `docs/evidence/P1_CONNECTIVITY.json` |
| P2-P9 | Not started | No later-stage behavior added |

The next allowed action is GPT Gate 1 review. P2 must not begin unless Gate 1 permits it.

## Post-P2 status

GPT Gate 1 passed. P1 was merged to `main`, hosted run `33223396946` succeeded, and P2 Recorder &
Replay is implemented on `codex/p2-recorder-replay` for Gate 2 review. The repository now also has:

- A strict schema-v1 JSONL recording container with contiguous sequence numbers, per-event SHA-256,
  an event count, and a complete-content SHA-256 trailer.
- A bounded, non-blocking producer API with deterministic FIFO background persistence and
  shutdown drain/flush/sync semantics.
- Recorded normalized market books, explicit feed connection transitions, and caller-timestamped
  feed-health observations.
- Offline replay through the existing `MarketNormalizer` and `BookStore`, without an execution
  dependency or live-order hook.
- Fail-closed version, format, sequence, checksum, truncation, domain, and health validation.
- A real three-venue public recording whose two replay runs produced identical state/event output.

| Stage | Current status | Evidence |
|---|---|---|
| P0 Foundation | Gate 0 passed | `docs/stages/P0_REPORT.md` |
| P1 Connectivity | Gate 1 passed; merged to `main` | `docs/stages/P1_REPORT.md` |
| P2 Recorder & Replay | Implemented; Gate 2 pending | `docs/stages/P2_REPORT.md`, `docs/gates/GATE_2_REVIEW.md` |
| P3-P9 | Not started | No later-stage behavior added |

The next allowed action is GPT Gate 2 review. P3 must not begin unless Gate 2 permits it.

## Post-P3 status

GPT Gate 2 passed. P3 Measurement is implemented on `codex/p3-measurement` for Gate 3 review. The
repository now also has:

- Independent forward and reverse executable routes calculated from real L2 VWAP.
- Deterministic route-specific midpoint `ReferenceBasis` sampling, rolling robust midline,
  dispersion, warm-up, and logical-time eviction.
- Measurement-only regime classification and cost-adjusted opportunity evaluation using
  `Deviation`, expected round-trip fees, explicit execution uncertainty, signed funding, and named
  risk costs.
- Explicit fail-closed handling for unhealthy/stale/skewed books, insufficient depth, missing fees,
  and unavailable funding.
- Offline deterministic P3 replay bound to the P2 recording checksum, P3 model version, and
  measurement configuration fingerprint, with no execution capability.
- Fresh official-adapter SNDK recording/reconnect evidence and a machine-readable two-route
  measurement report.

| Stage | Current status | Evidence |
|---|---|---|
| P0 Foundation | Gate 0 passed | `docs/stages/P0_REPORT.md` |
| P1 Connectivity | Gate 1 passed; merged to `main` | `docs/stages/P1_REPORT.md` |
| P2 Recorder & Replay | Gate 2 passed | `docs/stages/P2_REPORT.md` |
| P3 Measurement | Implemented; Gate 3 pending | `docs/stages/P3_REPORT.md`, `docs/gates/GATE_3_REVIEW.md`, `docs/evidence/P3_MEASUREMENT.json` |
| P4-P9 | Not started | No later-stage behavior added |

The next allowed action is GPT Gate 3 review. P4 must not begin unless Gate 3 permits it.

## Post-P4 status

GPT Gate 3 passed. P3 was fast-forwarded to `main`, hosted run `33262465076` succeeded, and P4 CJ
Target Inventory is implemented on `codex/p4-grid-inventory` for Gate 4 review. The repository now
also has:

- A dedicated `TargetFraction` domain type that rejects values outside `[0, 1]` during construction
  and deserialization.
- A deterministic floor-step `GridInventoryModel` mapping each explicit route's `Deviation` to a
  non-negative matched-notional-per-leg candidate.
- Pair-level arbitration which emits at most one `TargetInventory` and explicitly blocks opposing
  simultaneous increase candidates.
- `EffectiveInventory` / `EffectiveActual` contracts that include actual, reserved, and pending
  exposure before calculating the target delta.
- Increase-only P3 economic gating and measured-size caps, reduction-independent edge behavior,
  and two-step direction reversal.

| Stage | Current status | Evidence |
|---|---|---|
| P0 Foundation | Gate 0 passed | `docs/stages/P0_REPORT.md` |
| P1 Connectivity | Gate 1 passed; merged to `main` | `docs/stages/P1_REPORT.md` |
| P2 Recorder & Replay | Gate 2 passed | `docs/stages/P2_REPORT.md` |
| P3 Measurement | Gate 3 passed; merged to `main` | `docs/stages/P3_REPORT.md` |
| P4 CJ Target Inventory | Implemented; Gate 4 pending | `docs/stages/P4_REPORT.md`, `docs/gates/GATE_4_REVIEW.md` |
| P5-P9 | Not started | No later-stage behavior added |

The next allowed action is GPT Gate 4 review. P5 must not begin unless Gate 4 permits it.

## Post-P5 status

GPT Gate 4 passed. P4 was fast-forwarded to `main`, hosted run `33281603915` succeeded, and P5 Risk
is implemented on `codex/p5-risk` for Gate 5 review. The repository now also has:

- Distinct `Regime`, per-decision `RiskDecision`, and persistent/global `KillState` authority
  contracts with most-restrictive-wins evaluation.
- A deterministic `RiskManager` consuming P4 proposals, explicit logical time, health, effective
  exposure, signed session PnL, and persistent kill state without any execution capability.
- Increase-only P3 recency checks, including future-timestamp and maximum-age fail-closed behavior.
- Projected pair-per-leg, per-venue absolute notional, global-delta, and session-loss hard limits
  that include actual, reserved, and pending facts.
- Validated, serde-hardened `RiskAssessment` output with typed reason codes, candidate/authorized
  exposure projections, source size cap, policy fingerprint, and human-readable explanation.
- A caller-timestamped `KillStateMachine` with a fail-closed transition graph and audit records.

| Stage | Current status | Evidence |
|---|---|---|
| P0 Foundation | Gate 0 passed | `docs/stages/P0_REPORT.md` |
| P1 Connectivity | Gate 1 passed; merged to `main` | `docs/stages/P1_REPORT.md` |
| P2 Recorder & Replay | Gate 2 passed | `docs/stages/P2_REPORT.md` |
| P3 Measurement | Gate 3 passed; merged to `main` | `docs/stages/P3_REPORT.md` |
| P4 CJ Target Inventory | Gate 4 passed; merged to `main` | `docs/stages/P4_REPORT.md` |
| P5 Risk | Implemented; Gate 5 pending | `docs/stages/P5_REPORT.md`, `docs/gates/GATE_5_REVIEW.md` |
| P6-P9 | Not started | No execution/lifecycle implementation added |

The next allowed action is GPT Gate 5 review. P6 must not begin unless Gate 5 permits it.
