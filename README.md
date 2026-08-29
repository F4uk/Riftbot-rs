# Riftbot

Rust-first foundation for the Nautilus-based multi-venue perpetual arbitrage system governed by
`PROJECT_TASKBOOK.md`.

The repository is currently stopped at P2 Gate 2. It contains typed domain/configuration contracts,
official-adapter public connectivity, normalized feed health, and versioned deterministic market
recording/replay. It does not calculate spreads or fair value, emit orders, or enable live trading.

## Toolchain

- Rust 1.98.0, pinned by `rust-toolchain.toml`
- Rust 2024 edition
- NautilusTrader exact revision recorded in `UPSTREAM_SOURCES.md` and `Cargo.lock`

## Local checks

```text
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo check --locked --features nautilus-adapters
```

The example configuration is intentionally `signal_only` and contains no secret fields. P1 selected
the single evidence-backed V1 symbol `SNDK`; P2 recordings remain ignored local artifacts.

## Project documents

- `CURRENT_STATE.md`: baseline gap analysis and stage status
- `ARCHITECTURE.md`: responsibility and dependency boundaries
- `UPSTREAM_SOURCES.md`: exact upstream references and license policy
- `docs/plans/P0_PLAN.md`: P0-only plan
- `docs/stages/P0_REPORT.md`: P0 Gate 0 evidence
- `docs/stages/P1_REPORT.md`: P1 Gate 1 evidence
- `docs/plans/P2_PLAN.md`: P2-only plan
- `docs/stages/P2_REPORT.md`: P2 Gate 2 evidence
