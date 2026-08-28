# Architecture

## P0 boundary map

```text
Nautilus event/types
        |
        v
market::nautilus_bridge  <---- the only compiled Nautilus type conversion in P0
        |
        v
domain types (no Nautilus dependency)
        |
        +--> models --> strategy target (future P3/P4)
        +--> risk gate (future P5)
        +--> execution basket --> Nautilus orders (future P6)
        +--> recording/replay and PnL (future P2/P7+)
```

`domain` and `config` do not import Nautilus types. Future venue-specific event conversion and
order conversion stay in the two `nautilus_bridge` edge modules. The optional
`nautilus-adapters` feature is a compile-time compatibility probe; it does not construct a client
or access a network.

## Frozen responsibility rules

- Measurement produces facts and validity; it never chooses inventory or creates orders.
- `GridInventoryModel` will be the only target-inventory strategy.
- Strategy will emit `TargetInventory`; execution alone will convert current-versus-target delta
  into an intent and orders.
- Risk decisions outrank strategy and execution.
- An execution intent is basket-shaped from day one; the P0 constructor enforces exactly two,
  opposite-side, distinct-venue legs for V1.
- Unknown order state will be reconciled rather than inferred. P0 defines no order submission
  behavior.
- Replayable records use explicit timestamps and caller-supplied IDs; P0 generates no random or
  wall-clock-dependent decisions.

## Stage ownership

| Module | Responsibility | First implementing stage |
|---|---|---|
| `market` | Nautilus conversion, normalized books, freshness | P1 |
| `recording` | versioned event recording and deterministic replay | P2 |
| `models` | spread, fair value, regime, opportunity, grid target | P3/P4 |
| `risk` | hard limits and kill-state enforcement | P5 |
| `execution` | two-leg state machine and residual control | P6 |
| `reconciliation` | startup and continuous venue-truth recovery | P7 |
| `app` | LiveNode composition, never domain policy | P1 onward |

P0 files in these modules are responsibility markers only unless explicitly documented as a
domain contract or Nautilus compile bridge.
