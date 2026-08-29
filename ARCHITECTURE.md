# Architecture

## Stage boundary map

```text
Nautilus event/types
        |
        v
market::nautilus_bridge  <---- the only compiled Nautilus type conversion in P0
        |
        v
domain types (no Nautilus dependency)
        |
        +--> P3 measurement --> P4 route candidates --> InventoryManager --> one target
        +--> risk gate (future P5)
        +--> execution basket --> Nautilus orders (future P6)
        +--> recording/replay (P2) and PnL (future P7+)
```

`domain` and `config` do not import Nautilus types. Future venue-specific event conversion and
order conversion stay in the two `nautilus_bridge` edge modules. The optional
`nautilus-adapters` feature is a compile-time compatibility probe; it does not construct a client
or access a network.

## Frozen responsibility rules

- Measurement produces facts and validity; it never chooses inventory or creates orders.
- `GridInventoryModel` is the only target-inventory strategy. It consumes oriented-route
  `Deviation` and produces non-negative internal route candidates.
- `InventoryManager` includes actual, reserved, and pending exposure in EffectiveActual, arbitrates
  the two candidates, and materializes at most one external `TargetInventory`. Opposing increases
  materialize no target.
- P4 outputs a target-versus-effective-actual proposal only. Execution alone will later convert an
  authorized delta into an intent and orders.
- Risk decisions outrank strategy and execution.
- An execution intent is basket-shaped from day one; the P0 constructor enforces exactly two,
  opposite-side, distinct-venue legs for V1.
- Unknown order state will be reconciled rather than inferred. P0 defines no order submission
  behavior.
- Replayable records use explicit timestamps and caller-supplied IDs; P0 generates no random or
  wall-clock-dependent decisions.

## P2 recording and replay boundary

```text
P1 normalized VenueBook / explicit feed transition / health observation
        |
        v
BufferedRecorder::try_record --bounded FIFO--> background file writer
        |                                      header + event SHA-256 + file trailer
        v
versioned recording
        |
        v
strict full-file validation --> MarketNormalizer --> BookStore --> ReplayReport
```

Replay has no execution client, trait, callback, or order-event variant. Future account, order, and
fill shapes are type contracts only and are not accepted by the P2 recording event enum. Replay
age and freshness use recorded receive/observation timestamps; the replay module imports no clock.

## P4 target-inventory boundary

```text
P3 oriented Deviation --------> GridInventoryModel ------> forward/reverse candidates
P3 measurement economics ----> InventoryManager <------- actual + reserved + pending
                                      |
                                      v
                       at most one TargetInventory
                       + bounded P4 change proposal
                       (no RiskDecision, intent, or order)
```

The grid uses a deterministic floor-step rule and matched notional per leg. Increases require valid,
positive P3 economics and are capped by the measured executable notional. Reductions do not require
a favorable entry edge. Reversals flatten the old orientation before any opposite increase.

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

Modules not yet reached by the active stage remain responsibility markers unless explicitly
documented as an earlier-stage domain contract.
