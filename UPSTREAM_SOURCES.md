# Upstream Sources

Last reviewed: 2026-08-28

Upstream changes are never accepted automatically. A new reference requires an Upstream Change
Report and the review flow defined by `PROJECT_TASKBOOK.md`.

## NautilusTrader

| Field | Value |
|---|---|
| source | NautilusTrader |
| repo | https://github.com/nautechsystems/nautilus_trader |
| role | Production framework dependency; never forked or modified here |
| pinned_ref | `e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c` |
| license | LGPL-3.0-only at the pinned workspace |
| last_reviewed_at | 2026-08-28 |
| accepted_concepts | Event engine boundary, clock/cache/portfolio/order lifecycle, LiveNode, official venue adapters, reconciliation primitives |
| rejected_concepts | Floating `develop`, vendoring or modifying Nautilus core, direct strategy-to-venue bypasses |

### Pin and compatibility record

- The commit was verified through the official repository and resolves exactly to the taskbook
  snapshot.
- Workspace package version: 0.63.0; edition: Rust 2024; minimum Rust: 1.98.0.
- The project directly compiles `nautilus-model` with `high-precision` enabled at the exact `rev`.
- Optional P0 compatibility feature `nautilus-adapters` compiles official crates
  `nautilus-hyperliquid` and `nautilus-lighter` from the same exact `rev`.
- Hyperliquid source exposes all-perp-metadata and perp-DEX discovery requests, all-DEX asset
  context handling, and test data containing the `xyz` HIP-3 namespace.
- Lighter source exposes typed HTTP/WebSocket data and execution clients and Mainnet deployment
  configuration.
- P0 verifies source/API presence and compilation only. P1 must still prove Entropy/io,
  trade.xyz/xyz, and Lighter live discovery, timestamps, books, reconnect, and stale detection
  before selecting the single V1 symbol.

The Cargo lockfile is the reproducible dependency closure. No Cargo dependency uses a branch.

## yourQuantGuy / entropy-arb

| Field | Value |
|---|---|
| source | yourQuantGuy `entropy-arb` |
| repo | https://github.com/your-quantguy/entropy-arb |
| role | Measurement reference only; not a production dependency |
| pinned_ref | `aa0391471f6bf72f78c45801fb8117b7bf7e8c89` |
| license | MIT in GitHub repository metadata at review time |
| last_reviewed_at | 2026-08-28 |
| accepted_concepts | Executable bid/ask and depth, fee-aware edge, midline, staleness, recorder/statistical measurement ideas |
| rejected_concepts | Its final entry/exit policy, inventory ladder as position brain, Python bot architecture, venue execution state machine |

The commit was verified on the official repository. No source was copied into P0.

## CJ / crypto-trading-open

| Field | Value |
|---|---|
| source | CJ `crypto-trading-open` |
| repo | https://github.com/cryptocj520/crypto-trading-open |
| role | Conceptual strategy reference only; not a dependency |
| pinned_ref | `620737399bfe3c331f9989fc77d631536f2e89df` |
| license | None declared in GitHub repository metadata at review time |
| last_reviewed_at | 2026-08-28 |
| accepted_concepts | Spread-deviation grid, target inventory, `target - actual = delta`, convergence-based reduction, future multi-leg concept |
| rejected_concepts | All source copying, translation, derivative ports, file/function/class structure, and direct order logic |

The commit was verified on the official repository. Because no license is declared, the permanent
policy is concept-only clean implementation from the taskbook; no CJ source was copied or translated.
