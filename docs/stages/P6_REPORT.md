# P6 V1 Execution Basket Coordinator Report

Status: implemented on `codex/p6-execution-basket`; stopped at GPT Gate 6 before P7

## Verified starting point

- Gate 5-approved P5 commit merged to `main`: `97d688879399fe8951030103da95f66a8eaa7380`
- Green merged-main CI:
  [run 33290212335](https://github.com/F4uk/Riftbot-rs/actions/runs/33290212335), conclusion
  `SUCCESS`
- Frozen P6 plan commit: `f9f6e2b`
- P6 implementation and Gate 6 package: the commit containing this report; the final SHA and
  hosted CI result are reported after push

## Safety boundary

P6 implements deterministic execution safety only. The runtime boundary is a pure
`ExecutionPort` trait accepting already-journaled command batches. Its typed `DispatchOutcome`
distinguishes `AcceptedForDispatch`, batch-wide `DefinitelyNotSent`, and `Ambiguous`. Only the
second permits a future bridge to assert that no command reached an external execution boundary;
all unclassified transport failures are ambiguous, enter `Unknown`, and cannot trigger a blind
resend. There is no real port, private venue connection, credential lookup, custom REST/WebSocket
client, Nautilus order submission, tiny-live mode, or unrestricted market-order fallback. The
complete P6 lifecycle and fault matrix uses only `FakeExecutionPort`,
`InMemoryExecutionJournal`, and caller-supplied logical time.

The existing `execution::nautilus_bridge` remains empty. P6 therefore cannot produce a network
side effect without a future, explicitly supplied runtime implementation. P7 startup/account
reconciliation is not present.

## Hardened execution intent

`ExecutionIntent` now has four typed purposes:

| Purpose | Required authority/evidence | Shape |
|---|---|---|
| `IncreaseRisk` | P4 `IncreaseRisk`, P5 `Approve`, `Ready`, `Normal` or `Degraded`, fresh P3 basis | exactly two initial legs |
| `ReduceRisk` | matching P4/P5 reduction under `Approve`, `ReduceOnly`, or `FlattenRequired` | exactly two initial legs |
| `ResidualHedge` | coordinator-only parent intent plus actual cumulative fill evidence | one bounded recovery leg |
| `EmergencyFlatten` | P5 `FlattenRequired` plus known signed-position evidence | one bounded safety leg |

Normal intents preserve the complete source `InventoryDecision` and embedded `RiskContext` /
`RiskAssessment`. Construction and serde bind decision, pair, symbol, route, P4 requested/proposed
size, P5 authorization, current exposure audit, regime, kill state, P3 measurement age, measured
matched-notional cap, and measurement-config fingerprint. P4 `FlattenForReversal` is represented as
`ReduceRisk` while its original action, target, and reason remain embedded.

An approved increase is impossible under a restrictive P5 action, regime, or kill state. An intent
cannot enlarge P5 authorization, P4 proposal, P3 requested base quantity, either measured leg
notional, or the P3 safe matched cap. Recovery and emergency intents have dedicated evidence-taking
constructors; arbitrary normal callers cannot construct a residual hedge. Serde re-runs all
cross-field invariants.

## Conservative sizing and bounded prices

P5 authorization is interpreted as matched notional **per leg**. Instrument metadata supplies lot
size, quantity precision, and reduce-only support. Quantity is divided by reference price, capped
by the measured P3 quantity for increases, truncated to whole lots, and checked again against the
P5 budget and each measured P3 leg. No rounding path can increase unmeasured or unauthorized
exposure. If no positive safe lot fits, or the planned signed imbalance exceeds
`max_residual_delta`, construction fails closed.

The only order policies are `MarketableLimit` and `ImmediateOrCancel`. Every buy carries a finite
`MaximumBuy`; every sell carries a finite `MinimumSell`. Side/guard mismatches fail during intent
construction and actual fill facts outside the frozen guard fail safe.

## Increase authorization freshness and preflight

Immediately before an initial increase batch, P6:

1. rejects expired, future, or regressive logical times;
2. adds elapsed logical time since P5 evaluation to P5's recorded measurement age and rejects an
   age above the frozen P5 maximum;
3. binds the P4 measurement fingerprint to the current P3 measurement configuration;
4. freezes exact normalized books, health, logical time, preflight ID, and P3 evaluation;
5. remeasures executable depth through `SpreadEngine` at a base quantity covering both planned
   legs;
6. re-evaluates current fair value, costs, regime, and positive tradable edge through the P3
   opportunity path; and
7. checks executable prices against both side-aware guards and maximum slippage.

Any failure occurs before either child identity is exposed for dispatch. Reductions and emergency
flatten do not require fresh or positive entry economics, but their intents still require finite
bounded price guards and unexpired caller logical time.

## Commands, journal, and parallel initial dispatch

Each child identity is stable across replay and contains root intent ID, leg index, and generation;
`client_order_id` and `command_id` are deterministic. The coordinator prepares every initial child
first, atomically journals the intent, frozen preflight, reservation, child identities, and all
commands, then returns one `ExecutionCommandBatch`. A normal strategy batch contains both submit
commands together. It never waits for leg A acknowledgement before deciding whether leg B exists.

The append-only `ExecutionJournal` boundary retains:

- decision, intent, purpose, P5 context, and preflight evidence;
- reservation acquisition, post-fill conversion, and evidence-backed release;
- typed dispatch outcomes, including ambiguous partial/non-atomic handoff;
- stable child and command identity;
- submissions, acknowledgements, rejections, fills, cancels, and timeouts;
- every state transition and residual update;
- evidence-backed recovery intent, attempt, and reason; and
- terminal state, reason, required restrictive authority, and logical timestamp.

Journal append is transactional from the coordinator's perspective. If the pre-side-effect append
fails, no coordinator state or reservation is committed and no command batch is returned.

## Child and basket state machines

Child states are `NotSent`, `Submitting`, `AcceptedOpen`, `PartiallyFilled`, `Filled`, `Canceled`,
`Rejected`, and `Unknown`. Cancel state is independent: `NotRequested`, `Requested`, `Confirmed`,
`Rejected`, or `Unknown`. A cancel request never implies cancellation.

Basket states are `Planned`, `Submitting`, `Pending`, `Partial`, `Imbalanced`, `Hedging`, `Balanced`,
`Complete`, `Unknown`, `Reconciling`, `Aborting`, and `FailedSafe`. `Balanced` is retained as an
explicit journaled transition immediately before `Complete` when the terminal proof becomes true.

`Complete` requires every relevant initial/recovery child to be terminal, no unknown child or
cancel state, actual-fill residual within tolerance, and no outstanding child capable of changing
residual. Timeout is ambiguity, not rejection: a genuinely unresolved, newer timeout produces
`Unknown`, can transition to `Reconciling`, never creates a replacement, and cannot reach
`Complete` without authoritative events. Timer/ambiguity observations are monotonic: a stale
acknowledgement timeout cannot overwrite a newer ack or fill, and stale cancel ambiguity cannot
overwrite a newer cancel confirmation/rejection. Authoritative out-of-order ack/fill events remain
accepted and fills still update cumulative exposure. Snapshot serde validates state-specific child
counts, initial/recovery generations, terminal facts, fill arithmetic, residual recomputation,
recovery-attempt count, and restrictive authority for `FailedSafe`.

## Actual fills, late events, and recovery

Residual is computed only from actual cumulative filled notional: buys are positive and sells are
negative. For a normal matched basket the starting residual is zero. For an emergency flatten the
starting residual is the signed known-position evidence, so actual fills prove movement toward
zero. Requested quantity never substitutes for a fill.

Fill IDs are globally idempotent within the basket. Duplicate identical fills do not double-count;
conflicting duplicate IDs, regressive cumulative quantity/notional, incoherent average-price
arithmetic, quantity above the child bound, or a fill outside its price guard enters `FailedSafe`.
Out-of-order acknowledgement after fill preserves the fill state. Fills after cancel request,
confirmed cancel, timeout, or `Unknown` still update residual. A late fill that reopens a previously
completed zero-fill canceled increase basket reacquires its reservation and restores effective
exposure visibility; it cannot silently free the pair.

When residual exceeds tolerance, recovery waits until both initial child states are authoritative
and every prior recovery generation is terminal. It then chooses an actually filled source side,
creates one opposite reduce-only recovery leg on that source venue, and proves:

```text
recovery_notional <= absolute unmatched actual filled exposure
absolute projected residual < absolute current residual
```

There is no entry-edge requirement for recovery. Price bounds remain finite. A rejected or partial
recovery can advance only to the next stable generation, up to configured
`max_recovery_attempts`. No bounded improving action or attempt exhaustion enters `FailedSafe`,
records the reason, and surfaces `FlattenRequired` authority. P6 reports that safety requirement;
it does not perform P7 reconciliation or an unbounded retry loop.

## Pair reservation visibility

Before an increase batch is returned, the coordinator acquires the sole single-process reservation
for that pair. A second increase is rejected while it exists. The reservation book can overlay its
matched per-leg amount on `EffectiveInventory`, so the next P4 tick sees it as reserved exposure
without changing P4 mathematics. Unknown, reconciling, imbalanced, hedging, aborting, and
failed-safe baskets retain the reservation.

Completion is not position truth. A zero-filled terminal basket releases ownership, but a completed
increase with remaining matched fills converts from `Active` to
`FilledAwaitingInventorySync`. The converted amount remains in the reservation book, blocks a
second increase, and is overlaid as pending exposure until actual account/cache inventory includes
it. The overlay adds only the missing amount relative to the baseline actual exposure frozen at
reservation acquisition, so an already-updated account view is not double-counted.

P6 exposes `AuthoritativeInventorySyncEvidence` and a validating release method as the ownership
boundary P7 may later consume. Release requires matching pair/intent/symbol/route identity, a
non-regressive logical observation time, and account `actual_notional_per_leg` at least equal to the
frozen baseline plus converted fills. The proof and release are journaled atomically. P6 does not
obtain venue/account truth or implement reconciliation.

## Deterministic fault-injection evidence

`tests/p6_execution.rs` contains 58 deterministic P6 tests. Key evidence is summarized below.

| Fault or invariant | Proven result |
|---|---|
| two accepted/full-filled legs | journaled `Balanced -> Complete`, residual from fills |
| initial dispatch | both normal submit commands in one batch after one atomic journal append |
| one reject/no fill | other child is canceled authoritatively; zero-fill basket completes safely |
| reject after opposite fill | basket becomes `Imbalanced`, never pretends atomic failure |
| one partial/one full; both partial | actual signed residual and `Imbalanced`/`Partial` are deterministic |
| delayed ack; fill before ack | no sequencing assumption; fill state is preserved |
| ack/cancel ambiguity ordering | stale timers cannot regress newer ack/fill/cancel truth; unresolved newer timeout enters `Unknown` |
| ambiguous dispatch failure | typed `Ambiguous`, both unresolved children become `Unknown`, no resend/generation |
| `Unknown -> Reconciling` | only authoritative ack/fill/reject resolves uncertainty |
| duplicate/conflicting/regressive fills | identical is idempotent; conflicting/regressive facts fail safe |
| cancel requested/rejected/unknown/confirmed | every cancel state remains distinct and audited |
| late fill after cancel or timeout | actual residual updates; completed canceled basket can reopen safely |
| one-leg fill + other `Unknown` + later fill | no recovery while unknown and no double hedge after late fill |
| bounded residual hedge | actual-fill bound and strict residual improvement are proven |
| rejected/partial recovery | stable next generation only; no unbounded replacement |
| recovery exhaustion | `FailedSafe` plus `FlattenRequired` |
| stale/expired/regressive authorization | rejected before any command or reservation |
| P5/P3 sizing and lot rounding | no intent or rounded leg exceeds its authority/measurement |
| restrictive P5 state | cannot deserialize or construct an increase |
| reduction / P4 reversal | works without entry edge and preserves source P4 action |
| emergency flatten | cannot cross zero; actual fill starts from known signed exposure |
| pair ownership after completion | filled exposure converts to pending visibility and blocks a second increase until typed account proof |
| inventory incorporation | converted overlay does not double-count updated actual state; authoritative proof releases ownership |
| zero-fill completion / late fill | zero fills release; a later fill reacquires ownership and effective exposure visibility |
| journal failure | no unjournaled command identity or reservation escapes |
| deterministic replay | same command/event sequence yields identical state, journal, and reservation |
| serde mutation | impossible intent and basket states are rejected |
| network safety | fake port records the batch and reports zero real network calls |

## Verification

The required local Gate 6 commands pass. `cargo test --locked --all-targets --all-features` runs
203 tests with 0 failures:

- 96 library tests present at the Gate 5 baseline, of which four P0 execution-skeleton tests were
  replaced by P6 integration coverage, leaving 92 library tests;
- 11 P2 recording/replay integration tests;
- 6 P3 measurement integration tests;
- 36 P5 hard-risk integration tests; and
- 58 P6 deterministic execution/fault-injection integration tests.

| Command | Result |
|---|---|
| `python scripts/ci_policy.py all` | pass |
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --all-targets --all-features` | 203 passed; 0 failed |
| `cargo check --locked --features nautilus-adapters` | pass |

## Scope audit

No production credentials, real-money/tiny-live path, real order submission, custom venue execution
client, unrestricted market order, startup/account reconciliation, transfer/rebalancing, maker
strategy, 1:N/N:M generalization, distributed lock, Kafka/Redis, Nautilus core modification, or
P7/P8/P9 behavior was added. P6 stops here for GPT Gate 6.
