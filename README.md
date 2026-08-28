# Riftbot

Rust-first foundation for the Nautilus-based multi-venue perpetual arbitrage system governed by
`PROJECT_TASKBOOK.md`.

The repository is currently stopped at P0 Foundation. It contains typed domain and configuration
contracts, the frozen module boundaries, exact upstream pins, and CI gates. It does not connect to
venues, calculate signals, emit orders, or enable live trading.

## Toolchain

- Rust 1.98.0, pinned by `rust-toolchain.toml`
- Rust 2024 edition
- NautilusTrader exact revision recorded in `UPSTREAM_SOURCES.md` and `Cargo.lock`

## Local checks

```text
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --features nautilus-adapters
```

The example configuration is intentionally `signal_only` and contains no secret fields. P1 will
perform venue discovery before selecting the single V1 symbol.

## Project documents

- `CURRENT_STATE.md`: baseline gap analysis and stage status
- `ARCHITECTURE.md`: responsibility and dependency boundaries
- `UPSTREAM_SOURCES.md`: exact upstream references and license policy
- `docs/plans/P0_PLAN.md`: P0-only plan
- `docs/stages/P0_REPORT.md`: P0 Gate 0 evidence
