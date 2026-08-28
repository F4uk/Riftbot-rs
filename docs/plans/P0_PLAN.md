# P0 Execution Plan

Scope: Foundation only. No venue connections, strategy calculations, risk decisions, order paths,
or replay behavior are implemented in this plan.

## Work items

1. Capture the empty-repository baseline and stage gaps in `CURRENT_STATE.md`.
2. Verify all three frozen upstream commits and repository license metadata.
3. Pin Nautilus by exact revision, lock dependencies, and compile a minimal model bridge plus both
   official target adapters.
4. Define typed IDs, fixed-decimal units, required P0 domain records, and V1 two-leg intent
   construction invariants.
5. Define a typed, validated, secret-free signal-only configuration and checked example.
6. Create the frozen module skeleton and architecture documentation.
7. Add CI for format, lint, tests, adapter compile compatibility, locked dependency resolution, and
   a conservative credential-pattern scan.
8. Run every P0 gate, self-review the diff, fix findings, and publish the P0 Gate 0 package.

## Non-goals

- P1 connectivity or symbol selection.
- Market normalization or staleness evaluation.
- Recorder/replay implementation.
- Spread, fair value, regime, or grid calculations.
- Runtime risk enforcement, execution, reconciliation, or live trading.

## Acceptance commands

```text
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --features nautilus-adapters
```

CI additionally verifies that Cargo manifests contain no floating Git branch dependency and scans
tracked project content for common secret assignments.
