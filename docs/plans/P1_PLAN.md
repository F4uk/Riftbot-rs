# P1 Connectivity Plan

Status: in progress on `codex/p1-connectivity`

## Scope

P1 proves public market-data connectivity through the pinned official Nautilus adapters and adds
only the normalization and in-memory book state required to evaluate connectivity. It does not
calculate spreads, fair value, targets, signals, or orders.

## Pinned adapter paths

- Nautilus revision: `e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c`.
- Hyperliquid discovery: official `HyperliquidHttpClient::request_instruments`, which loads all
  perpetual DEX metadata including HIP-3.
- Hyperliquid live data: official `HyperliquidWebSocketClient` L2 order-book subscription and its
  transport reconnect behavior.
- Lighter discovery/snapshot: official `LighterHttpClient::request_instruments` and
  `request_order_book_snapshot` on Mainnet.
- Lighter live data: official `LighterWebSocketClient` L2 order-book subscription and its
  transport reconnect behavior.

No private endpoint, credential, execution client, custom venue REST/WS client, or Nautilus core
change is permitted. If a required public path cannot be completed, P1 stops and records an
Adapter Gap Report before considering any alternative.

## Small tasks and evidence

1. Add deterministic `MarketNormalizer` validation and canonical ordering for full L2 snapshots.
   Test invalid, crossed, empty, duplicated, and unsorted inputs.
2. Add `BookStore` version acceptance, exchange/receive timestamps, caller-supplied age
   calculation, freshness classification, and explicit feed lifecycle health. Test stale,
   out-of-order, disconnect, reconnect, and recovery behavior without using wall-clock sleeps.
3. Add a manual, public-only connectivity probe behind the `nautilus-adapters` feature. It must
   use the pinned official adapter clients to discover Entropy/io, trade.xyz/xyz, and Lighter,
   observe books, and exercise disconnect/reconnect recovery.
4. Run the probe against live public Mainnet endpoints. Store sanitized, deterministic evidence
   containing adapter revision, discovered instruments, timestamps, health transitions, and
   reconnect results. Compare venue inventories and select exactly one common V1 symbol only when
   that evidence exists.
5. Self-review against P1 restrictions; run formatting, locked clippy/tests/checks and repository
   policy scans. Push the branch and require hosted CI to remain green.
6. Create `docs/stages/P1_REPORT.md` and a Gate 1 review package. Stop for GPT Gate 1 without
   beginning P2.

## Acceptance

- Official adapters discover the named HIP-3 venues and Lighter Mainnet.
- Exactly one evidence-backed common V1 symbol is configured.
- The selected symbol yields stable normalized books with useful exchange and receive timestamps.
- Age/freshness and venue/feed health are observable; stale data is rejected by health checks.
- A forced public-feed disconnect/reconnect returns to healthy book updates.
- All local and hosted gates pass and no secret or build artifact is tracked.
