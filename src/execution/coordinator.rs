//! Deterministic two-leg coordinator, preflight, command, journal, and reservation boundaries.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::{MeasurementConfigFingerprint, RegimeConfig},
    domain::{
        execution_intent::{
            ExecutionIntent, ExecutionIntentError, ExecutionIntentPurpose, ExecutionLeg,
            ExecutionSafetyEvidence, OrderSide, PriceGuard, ResidualFillEvidence,
            ResidualHedgeIntentRequest,
        },
        ids::{
            ClientOrderId, CommandId, IntentId, PairId, PreflightId, Symbol, VenueId, VenueOrderId,
        },
        inventory::{EffectiveInventory, InventoryDomainError, OrientedExposure},
        market::{FeedHealth, VenueBook},
        numeric::{BaseQty, Bps, Delta, DurationMillis, Notional, Price, UnixNanos},
        risk::RiskDecision,
        spread::ExplicitRiskCost,
    },
    models::{
        fair_value::FairValueSnapshot,
        opportunity::{OpportunityEvaluation, OpportunityInput, OpportunityModel},
        spread_engine::{RouteCostInput, RouteMeasurementInput, SpreadEngine},
    },
};

use super::{
    residual::{ResidualPlanError, plan_residual_recovery},
    state_machine::{
        AppliedFill, BasketState, CancelState, ChildOrderSnapshot, ChildOrderState,
        ExecutionBasketSnapshot, ExecutionBasketSnapshotParts, ExecutionStateError, absolute,
        residual_from_children,
    },
};

const NANOS_PER_MILLISECOND: u64 = 1_000_000;
const BPS_SCALE: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

/// All current market/economic facts required immediately before an increase dispatch.
#[derive(Clone, Debug)]
pub struct IncreasePreflightInput<'a> {
    pub preflight_id: PreflightId,
    pub preflight_at: UnixNanos,
    pub long_book: &'a VenueBook,
    pub short_book: &'a VenueBook,
    pub long_health: &'a FeedHealth,
    pub short_health: &'a FeedHealth,
    pub fair_value: FairValueSnapshot,
    pub costs: RouteCostInput,
    pub other_explicit_risk_costs_bps: Vec<ExplicitRiskCost>,
    pub max_book_age_ms: DurationMillis,
    pub max_receive_skew_ms: DurationMillis,
    pub regime_config: &'a RegimeConfig,
    pub measurement_config_fingerprint: &'a MeasurementConfigFingerprint,
}

/// Exact frozen pre-submit evidence retained in the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IncreasePreflightSnapshot {
    pub preflight_id: PreflightId,
    pub preflight_at: UnixNanos,
    pub long_book: VenueBook,
    pub short_book: VenueBook,
    pub long_health: FeedHealth,
    pub short_health: FeedHealth,
    pub evaluation: OpportunityEvaluation,
}

/// Stable bounded order request emitted by the pure coordinator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitOrderCommand {
    pub command_id: CommandId,
    pub client_order_id: ClientOrderId,
    pub basket_intent_id: IntentId,
    pub recovery_intent_id: Option<IntentId>,
    pub purpose: ExecutionIntentPurpose,
    pub leg_index: usize,
    pub generation: u32,
    pub leg: ExecutionLeg,
    pub limit_price: Price,
    pub created_at: UnixNanos,
}

/// Stable cancel command. It is a request, never proof that the order is canceled.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancelOrderCommand {
    pub command_id: CommandId,
    pub client_order_id: ClientOrderId,
    pub basket_intent_id: IntentId,
    pub requested_at: UnixNanos,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionCommand {
    Submit(SubmitOrderCommand),
    Cancel(CancelOrderCommand),
}

/// Commands in one coordinator action are dispatched together by the port boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCommandBatch {
    pub basket_intent_id: IntentId,
    pub emitted_at: UnixNanos,
    pub commands: Vec<ExecutionCommand>,
}

/// Runtime bridge contract. P6 ships no real implementation.
pub trait ExecutionPort {
    type Error;

    fn dispatch(&mut self, batch: &ExecutionCommandBatch) -> Result<(), Self::Error>;
}

/// Authoritative or explicitly ambiguous event delivered by the runtime bridge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionEvent {
    Acknowledged {
        client_order_id: ClientOrderId,
        venue_order_id: VenueOrderId,
        exchange_ts: UnixNanos,
        receive_ts: UnixNanos,
    },
    Rejected {
        client_order_id: ClientOrderId,
        reason: String,
        exchange_ts: UnixNanos,
        receive_ts: UnixNanos,
    },
    Fill {
        client_order_id: ClientOrderId,
        fill: AppliedFill,
    },
    CancelConfirmed {
        client_order_id: ClientOrderId,
        exchange_ts: UnixNanos,
        receive_ts: UnixNanos,
    },
    CancelRejected {
        client_order_id: ClientOrderId,
        reason: String,
        exchange_ts: UnixNanos,
        receive_ts: UnixNanos,
    },
    CancelUnknown {
        client_order_id: ClientOrderId,
        observed_at: UnixNanos,
    },
    AcknowledgementTimeout {
        client_order_id: ClientOrderId,
        observed_at: UnixNanos,
    },
}

impl ExecutionEvent {
    #[must_use]
    pub const fn logical_time(&self) -> UnixNanos {
        match self {
            Self::Acknowledged { receive_ts, .. }
            | Self::Rejected { receive_ts, .. }
            | Self::CancelConfirmed { receive_ts, .. }
            | Self::CancelRejected { receive_ts, .. } => *receive_ts,
            Self::Fill { fill, .. } => fill.receive_ts,
            Self::CancelUnknown { observed_at, .. }
            | Self::AcknowledgementTimeout { observed_at, .. } => *observed_at,
        }
    }

    #[must_use]
    pub fn client_order_id(&self) -> &ClientOrderId {
        match self {
            Self::Acknowledged {
                client_order_id, ..
            }
            | Self::Rejected {
                client_order_id, ..
            }
            | Self::Fill {
                client_order_id, ..
            }
            | Self::CancelConfirmed {
                client_order_id, ..
            }
            | Self::CancelRejected {
                client_order_id, ..
            }
            | Self::CancelUnknown {
                client_order_id, ..
            }
            | Self::AcknowledgementTimeout {
                client_order_id, ..
            } => client_order_id,
        }
    }
}

/// Append-only evidence written before commands and after every accepted event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionJournalRecord {
    IntentPrepared {
        intent: ExecutionIntent,
        timestamp: UnixNanos,
    },
    PreflightFrozen {
        snapshot: IncreasePreflightSnapshot,
    },
    ReservationAcquired {
        pair_id: PairId,
        intent_id: IntentId,
        notional_per_leg: Notional,
        timestamp: UnixNanos,
    },
    ReservationReleased {
        pair_id: PairId,
        intent_id: IntentId,
        converted_to_filled: bool,
        timestamp: UnixNanos,
    },
    ChildPrepared {
        child: ChildOrderSnapshot,
        timestamp: UnixNanos,
    },
    CommandPrepared {
        command: ExecutionCommand,
        timestamp: UnixNanos,
    },
    EventApplied {
        event: ExecutionEvent,
        duplicate: bool,
    },
    StateTransition {
        from: BasketState,
        to: BasketState,
        reason: String,
        timestamp: UnixNanos,
    },
    ResidualUpdated {
        residual: Delta,
        timestamp: UnixNanos,
    },
    RecoveryIntentPrepared {
        intent: ExecutionIntent,
        attempt: u32,
        reason: String,
        timestamp: UnixNanos,
    },
    Terminal {
        state: BasketState,
        reason: String,
        required_authority: Option<RiskDecision>,
        timestamp: UnixNanos,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExecutionJournalError {
    #[error("execution journal append failed")]
    AppendFailed,
}

/// Durable implementations must append each supplied batch atomically and in order.
pub trait ExecutionJournal {
    fn append_batch(
        &mut self,
        records: &[ExecutionJournalRecord],
    ) -> Result<(), ExecutionJournalError>;
}

/// Deterministic in-memory journal used by P6 tests and replay harnesses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InMemoryExecutionJournal {
    records: Vec<ExecutionJournalRecord>,
    fail_next_append: bool,
}

impl InMemoryExecutionJournal {
    #[must_use]
    pub fn records(&self) -> &[ExecutionJournalRecord] {
        &self.records
    }

    pub fn fail_next_append(&mut self) {
        self.fail_next_append = true;
    }
}

impl ExecutionJournal for InMemoryExecutionJournal {
    fn append_batch(
        &mut self,
        records: &[ExecutionJournalRecord],
    ) -> Result<(), ExecutionJournalError> {
        if self.fail_next_append {
            self.fail_next_append = false;
            return Err(ExecutionJournalError::AppendFailed);
        }
        self.records.extend_from_slice(records);
        Ok(())
    }
}

/// One active pair reservation visible to subsequent effective-inventory composition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairReservation {
    pub intent_id: IntentId,
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub notional_per_leg: Notional,
    pub acquired_at: UnixNanos,
}

/// Single-process V1 ownership; there is deliberately no distributed lock.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PairReservationBook {
    active: BTreeMap<PairId, PairReservation>,
}

impl PairReservationBook {
    #[must_use]
    pub fn active(&self, pair_id: &PairId) -> Option<&PairReservation> {
        self.active.get(pair_id)
    }

    #[must_use]
    pub fn reserved_notional_per_leg(&self, pair_id: &PairId) -> Option<Notional> {
        self.active.get(pair_id).map(|value| value.notional_per_leg)
    }

    /// Overlays active P6 reservations onto the P4 inventory input without mutating P4 math.
    pub fn effective_inventory_with_reservations(
        &self,
        inventory: &EffectiveInventory,
    ) -> Result<EffectiveInventory, CoordinatorError> {
        let Some(reservation) = self.active.get(&inventory.pair_id) else {
            return Ok(inventory.clone());
        };
        if reservation.symbol != inventory.symbol {
            return Err(CoordinatorError::ReservationMismatch);
        }
        let mut exposures = inventory.exposures.clone();
        if let Some(exposure) = exposures.iter_mut().find(|exposure| {
            exposure.long_venue == reservation.long_venue
                && exposure.short_venue == reservation.short_venue
        }) {
            exposure.reserved_notional_per_leg = Notional::new(
                exposure
                    .reserved_notional_per_leg
                    .value()
                    .checked_add(reservation.notional_per_leg.value())
                    .ok_or(CoordinatorError::ReservationArithmetic)?,
            )
            .map_err(|_| CoordinatorError::ReservationArithmetic)?;
        } else {
            exposures.push(OrientedExposure::new(
                reservation.long_venue.clone(),
                reservation.short_venue.clone(),
                Notional::new(Decimal::ZERO)
                    .map_err(|_| CoordinatorError::ReservationArithmetic)?,
                reservation.notional_per_leg,
                Notional::new(Decimal::ZERO)
                    .map_err(|_| CoordinatorError::ReservationArithmetic)?,
            )?);
        }
        Ok(EffectiveInventory::new(
            inventory.symbol.clone(),
            inventory.pair_id.clone(),
            exposures,
        )?)
    }

    fn reserve(
        &mut self,
        pair_id: PairId,
        reservation: PairReservation,
    ) -> Result<(), CoordinatorError> {
        if self.active.contains_key(&pair_id) {
            return Err(CoordinatorError::PairIncreaseAlreadyReserved);
        }
        self.active.insert(pair_id, reservation);
        Ok(())
    }

    fn finalize(&mut self, pair_id: &PairId, intent_id: &IntentId) -> Result<(), CoordinatorError> {
        if self
            .active
            .get(pair_id)
            .is_none_or(|reservation| &reservation.intent_id != intent_id)
        {
            return Err(CoordinatorError::ReservationMismatch);
        }
        self.active.remove(pair_id);
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CoordinatorError {
    #[error("execution intent has an unsupported root-basket shape")]
    InvalidRootIntent,
    #[error("intent expired or logical execution time regressed")]
    InvalidExecutionTime,
    #[error("P5 increase authorization is stale at preflight")]
    StaleAuthorization,
    #[error("preflight market identities differ from the planned intent")]
    PreflightIdentityMismatch,
    #[error("current P3 executable depth/data/economics no longer permit an increase")]
    PreflightEconomicsRejected,
    #[error("current executable price violates a finite guard or slippage limit")]
    PriceSafetyViolation,
    #[error("basket is not in a state which permits this action")]
    InvalidBasketState,
    #[error("one active risk-increasing basket already owns the pair reservation")]
    PairIncreaseAlreadyReserved,
    #[error("reservation state does not match this basket")]
    ReservationMismatch,
    #[error("reservation arithmetic failed closed")]
    ReservationArithmetic,
    #[error("execution event references an unknown child identity")]
    UnknownChild,
    #[error("execution identity cannot be represented deterministically")]
    InvalidStableIdentity,
    #[error("recovery is blocked until all initial child states are authoritative")]
    RecoveryAwaitingAuthority,
    #[error(transparent)]
    Intent(#[from] ExecutionIntentError),
    #[error(transparent)]
    State(#[from] ExecutionStateError),
    #[error(transparent)]
    Residual(#[from] ResidualPlanError),
    #[error(transparent)]
    Journal(#[from] ExecutionJournalError),
    #[error(transparent)]
    Inventory(#[from] InventoryDomainError),
}

/// Pure deterministic owner of one V1 root basket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionBasketCoordinator {
    intent: ExecutionIntent,
    state: BasketState,
    children: Vec<ChildOrderSnapshot>,
    starting_residual: Delta,
    residual: Delta,
    recovery_attempts: u32,
    max_recovery_attempts: u32,
    recovery_intents: Vec<ExecutionIntent>,
    required_authority: Option<RiskDecision>,
    terminal_reason: Option<String>,
    reservation_finalized: bool,
}

impl ExecutionBasketCoordinator {
    pub fn new(
        intent: ExecutionIntent,
        max_recovery_attempts: u32,
    ) -> Result<Self, CoordinatorError> {
        if !matches!(
            intent.purpose(),
            ExecutionIntentPurpose::IncreaseRisk | ExecutionIntentPurpose::ReduceRisk
        ) || intent.legs().len() != 2
            || max_recovery_attempts == 0
        {
            return Err(CoordinatorError::InvalidRootIntent);
        }
        Ok(Self {
            intent,
            state: BasketState::Planned,
            children: Vec::new(),
            starting_residual: Delta::new(Decimal::ZERO),
            residual: Delta::new(Decimal::ZERO),
            recovery_attempts: 0,
            max_recovery_attempts,
            recovery_intents: Vec::new(),
            required_authority: None,
            terminal_reason: None,
            reservation_finalized: false,
        })
    }

    /// Creates an evidence-backed one-leg emergency safety basket.
    pub fn new_emergency(
        intent: ExecutionIntent,
        max_recovery_attempts: u32,
    ) -> Result<Self, CoordinatorError> {
        let starting_residual = match intent.safety_evidence() {
            Some(ExecutionSafetyEvidence::EmergencyFlatten(evidence))
                if intent.purpose() == ExecutionIntentPurpose::EmergencyFlatten
                    && intent.legs().len() == 1 =>
            {
                evidence.position().current_delta
            }
            _ => return Err(CoordinatorError::InvalidRootIntent),
        };
        if max_recovery_attempts == 0 {
            return Err(CoordinatorError::InvalidRootIntent);
        }
        Ok(Self {
            intent,
            state: BasketState::Planned,
            children: Vec::new(),
            starting_residual,
            residual: starting_residual,
            recovery_attempts: 0,
            max_recovery_attempts,
            recovery_intents: Vec::new(),
            required_authority: None,
            terminal_reason: None,
            reservation_finalized: false,
        })
    }

    #[must_use]
    pub const fn intent(&self) -> &ExecutionIntent {
        &self.intent
    }

    #[must_use]
    pub const fn state(&self) -> BasketState {
        self.state
    }

    #[must_use]
    pub const fn residual(&self) -> Delta {
        self.residual
    }

    #[must_use]
    pub fn children(&self) -> &[ChildOrderSnapshot] {
        &self.children
    }

    #[must_use]
    pub fn recovery_intents(&self) -> &[ExecutionIntent] {
        &self.recovery_intents
    }

    pub fn snapshot(&self) -> Result<ExecutionBasketSnapshot, CoordinatorError> {
        Ok(ExecutionBasketSnapshot::from_parts(
            ExecutionBasketSnapshotParts {
                intent_id: self.intent.intent_id().clone(),
                initial_leg_count: self.intent.legs().len(),
                state: self.state,
                children: self.children.clone(),
                starting_residual: self.starting_residual,
                residual: self.residual,
                max_residual_delta: self.intent.max_residual_delta(),
                recovery_attempts: self.recovery_attempts,
                max_recovery_attempts: self.max_recovery_attempts,
                required_authority: self.required_authority,
                terminal_reason: self.terminal_reason.clone(),
            },
        )?)
    }

    /// Performs the full P3-based preflight and journals both initial commands before dispatch.
    pub fn prepare_increase<J: ExecutionJournal>(
        &mut self,
        input: IncreasePreflightInput<'_>,
        journal: &mut J,
        reservations: &mut PairReservationBook,
    ) -> Result<ExecutionCommandBatch, CoordinatorError> {
        if self.intent.purpose() != ExecutionIntentPurpose::IncreaseRisk {
            return Err(CoordinatorError::InvalidRootIntent);
        }
        validate_execution_time(&self.intent, input.preflight_at)?;
        let snapshot = preflight_increase(&self.intent, input)?;
        self.prepare_initial_commands(snapshot.preflight_at, Some(snapshot), journal, reservations)
    }

    /// Reductions do not require positive/fresh entry economics, but retain bounded limit guards.
    pub fn prepare_reduction<J: ExecutionJournal>(
        &mut self,
        preflight_at: UnixNanos,
        journal: &mut J,
        reservations: &mut PairReservationBook,
    ) -> Result<ExecutionCommandBatch, CoordinatorError> {
        if self.intent.purpose() != ExecutionIntentPurpose::ReduceRisk {
            return Err(CoordinatorError::InvalidRootIntent);
        }
        validate_execution_time(&self.intent, preflight_at)?;
        self.prepare_initial_commands(preflight_at, None, journal, reservations)
    }

    /// Emergency flatten skips entry economics but remains expiry- and price-bound.
    pub fn prepare_emergency_flatten<J: ExecutionJournal>(
        &mut self,
        preflight_at: UnixNanos,
        journal: &mut J,
        reservations: &mut PairReservationBook,
    ) -> Result<ExecutionCommandBatch, CoordinatorError> {
        if self.intent.purpose() != ExecutionIntentPurpose::EmergencyFlatten {
            return Err(CoordinatorError::InvalidRootIntent);
        }
        validate_execution_time(&self.intent, preflight_at)?;
        self.prepare_initial_commands(preflight_at, None, journal, reservations)
    }

    fn prepare_initial_commands<J: ExecutionJournal>(
        &mut self,
        timestamp: UnixNanos,
        preflight: Option<IncreasePreflightSnapshot>,
        journal: &mut J,
        reservations: &mut PairReservationBook,
    ) -> Result<ExecutionCommandBatch, CoordinatorError> {
        if self.state != BasketState::Planned || !self.children.is_empty() {
            return Err(CoordinatorError::InvalidBasketState);
        }
        let mut next = self.clone();
        let mut next_reservations = reservations.clone();
        let mut records = vec![ExecutionJournalRecord::IntentPrepared {
            intent: self.intent.clone(),
            timestamp,
        }];
        if let Some(snapshot) = preflight {
            records.push(ExecutionJournalRecord::PreflightFrozen { snapshot });
        }
        if self.intent.purpose() == ExecutionIntentPurpose::IncreaseRisk {
            next_reservations.reserve(
                self.intent.pair_id().clone(),
                PairReservation {
                    intent_id: self.intent.intent_id().clone(),
                    symbol: self.intent.symbol().clone(),
                    long_venue: self.intent.legs()[0].venue.clone(),
                    short_venue: self.intent.legs()[1].venue.clone(),
                    notional_per_leg: self.intent.authorized_matched_notional_per_leg(),
                    acquired_at: timestamp,
                },
            )?;
            records.push(ExecutionJournalRecord::ReservationAcquired {
                pair_id: self.intent.pair_id().clone(),
                intent_id: self.intent.intent_id().clone(),
                notional_per_leg: self.intent.authorized_matched_notional_per_leg(),
                timestamp,
            });
        }
        let mut commands = Vec::with_capacity(self.intent.legs().len());
        for (leg_index, leg) in self.intent.legs().iter().enumerate() {
            let (command_id, client_order_id) =
                stable_submit_ids(self.intent.intent_id(), leg_index, 0, self.intent.purpose())?;
            let command = ExecutionCommand::Submit(SubmitOrderCommand {
                command_id: command_id.clone(),
                client_order_id: client_order_id.clone(),
                basket_intent_id: self.intent.intent_id().clone(),
                recovery_intent_id: None,
                purpose: self.intent.purpose(),
                leg_index,
                generation: 0,
                leg: leg.clone(),
                limit_price: leg.price_guard.price(),
                created_at: timestamp,
            });
            let child = new_child(
                self.intent.intent_id().clone(),
                leg_index,
                command_id,
                client_order_id,
                leg.clone(),
                false,
                0,
            );
            records.push(ExecutionJournalRecord::ChildPrepared {
                child: child.clone(),
                timestamp,
            });
            records.push(ExecutionJournalRecord::CommandPrepared {
                command: command.clone(),
                timestamp,
            });
            next.children.push(child);
            commands.push(command);
        }
        records.push(ExecutionJournalRecord::StateTransition {
            from: BasketState::Planned,
            to: BasketState::Submitting,
            reason: "all initial child commands journaled for one atomic dispatch batch".to_owned(),
            timestamp,
        });
        next.state = BasketState::Submitting;
        journal.append_batch(&records)?;
        *self = next;
        *reservations = next_reservations;
        Ok(ExecutionCommandBatch {
            basket_intent_id: self.intent.intent_id().clone(),
            emitted_at: timestamp,
            commands,
        })
    }

    /// Applies an event transactionally and idempotently, then recomputes actual-fill residual.
    pub fn handle_event<J: ExecutionJournal>(
        &mut self,
        event: ExecutionEvent,
        journal: &mut J,
        reservations: &mut PairReservationBook,
    ) -> Result<(), CoordinatorError> {
        if self.children.is_empty() {
            return Err(CoordinatorError::InvalidBasketState);
        }
        let mut next = self.clone();
        let mut next_reservations = reservations.clone();
        let previous_state = next.state;
        let previous_residual = next.residual;
        let duplicate = next.apply_event(&event)?;
        next.residual = residual_from_children(next.starting_residual, &next.children)?;
        next.refresh_state(event.logical_time());
        let mut records = vec![ExecutionJournalRecord::EventApplied {
            event: event.clone(),
            duplicate,
        }];
        if next.residual != previous_residual {
            records.push(ExecutionJournalRecord::ResidualUpdated {
                residual: next.residual,
                timestamp: event.logical_time(),
            });
        }
        if previous_state == BasketState::Complete
            && next.state != BasketState::Complete
            && next.intent.purpose() == ExecutionIntentPurpose::IncreaseRisk
            && next.reservation_finalized
        {
            match next_reservations.active(next.intent.pair_id()) {
                None => {
                    let reservation = PairReservation {
                        intent_id: next.intent.intent_id().clone(),
                        symbol: next.intent.symbol().clone(),
                        long_venue: next.intent.legs()[0].venue.clone(),
                        short_venue: next.intent.legs()[1].venue.clone(),
                        notional_per_leg: next.intent.authorized_matched_notional_per_leg(),
                        acquired_at: event.logical_time(),
                    };
                    next_reservations.reserve(next.intent.pair_id().clone(), reservation)?;
                    records.push(ExecutionJournalRecord::ReservationAcquired {
                        pair_id: next.intent.pair_id().clone(),
                        intent_id: next.intent.intent_id().clone(),
                        notional_per_leg: next.intent.authorized_matched_notional_per_leg(),
                        timestamp: event.logical_time(),
                    });
                    next.reservation_finalized = false;
                }
                Some(reservation) if reservation.intent_id == *next.intent.intent_id() => {
                    next.reservation_finalized = false;
                }
                Some(_) => next.mark_failed_safe(
                    "late fill reopened a basket after another increase acquired the pair",
                ),
            }
        }
        append_state_records(
            &mut records,
            previous_state,
            next.state,
            "authoritative child event applied",
            event.logical_time(),
        );
        if next.state == BasketState::Complete && !next.reservation_finalized {
            let has_fills = next
                .children
                .iter()
                .take(2)
                .any(|child| child.cumulative_filled_notional > Decimal::ZERO);
            if next.intent.purpose() == ExecutionIntentPurpose::IncreaseRisk {
                next_reservations.finalize(next.intent.pair_id(), next.intent.intent_id())?;
                records.push(ExecutionJournalRecord::ReservationReleased {
                    pair_id: next.intent.pair_id().clone(),
                    intent_id: next.intent.intent_id().clone(),
                    converted_to_filled: has_fills,
                    timestamp: event.logical_time(),
                });
            }
            next.reservation_finalized = true;
            records.push(ExecutionJournalRecord::Terminal {
                state: BasketState::Complete,
                reason: next
                    .terminal_reason
                    .clone()
                    .unwrap_or_else(|| "basket complete".to_owned()),
                required_authority: None,
                timestamp: event.logical_time(),
            });
        } else if next.state == BasketState::FailedSafe && previous_state != BasketState::FailedSafe
        {
            records.push(ExecutionJournalRecord::Terminal {
                state: BasketState::FailedSafe,
                reason: next
                    .terminal_reason
                    .clone()
                    .unwrap_or_else(|| "basket failed safe".to_owned()),
                required_authority: next.required_authority,
                timestamp: event.logical_time(),
            });
        }
        journal.append_batch(&records)?;
        *self = next;
        *reservations = next_reservations;
        Ok(())
    }

    /// Moves an unknown basket into explicit reconciliation without guessing venue truth.
    pub fn start_reconciling<J: ExecutionJournal>(
        &mut self,
        timestamp: UnixNanos,
        journal: &mut J,
    ) -> Result<(), CoordinatorError> {
        if self.state != BasketState::Unknown {
            return Err(CoordinatorError::InvalidBasketState);
        }
        let record = ExecutionJournalRecord::StateTransition {
            from: BasketState::Unknown,
            to: BasketState::Reconciling,
            reason: "unknown child requires authoritative venue reconciliation".to_owned(),
            timestamp,
        };
        journal.append_batch(&[record])?;
        self.state = BasketState::Reconciling;
        Ok(())
    }

    /// Journals bounded cancel requests for live children after an abort decision.
    pub fn prepare_abort_cancels<J: ExecutionJournal>(
        &mut self,
        timestamp: UnixNanos,
        journal: &mut J,
    ) -> Result<ExecutionCommandBatch, CoordinatorError> {
        if !matches!(
            self.state,
            BasketState::Submitting
                | BasketState::Pending
                | BasketState::Partial
                | BasketState::Imbalanced
                | BasketState::Hedging
                | BasketState::Aborting
        ) {
            return Err(CoordinatorError::InvalidBasketState);
        }
        let mut next = self.clone();
        let mut records = Vec::new();
        let mut commands = Vec::new();
        for child in next.children.iter_mut().filter(|child| {
            !child.is_terminal()
                && child.state != ChildOrderState::Unknown
                && child.cancel_state == CancelState::NotRequested
        }) {
            let command_id = stable_cancel_id(&child.client_order_id)?;
            child.cancel_state = CancelState::Requested;
            let command = ExecutionCommand::Cancel(CancelOrderCommand {
                command_id,
                client_order_id: child.client_order_id.clone(),
                basket_intent_id: self.intent.intent_id().clone(),
                requested_at: timestamp,
            });
            records.push(ExecutionJournalRecord::CommandPrepared {
                command: command.clone(),
                timestamp,
            });
            commands.push(command);
        }
        if commands.is_empty() {
            return Err(CoordinatorError::InvalidBasketState);
        }
        if next.state != BasketState::Aborting {
            records.push(ExecutionJournalRecord::StateTransition {
                from: next.state,
                to: BasketState::Aborting,
                reason: "bounded cancel commands journaled; cancellation is not yet confirmed"
                    .to_owned(),
                timestamp,
            });
            next.state = BasketState::Aborting;
        }
        journal.append_batch(&records)?;
        *self = next;
        Ok(ExecutionCommandBatch {
            basket_intent_id: self.intent.intent_id().clone(),
            emitted_at: timestamp,
            commands,
        })
    }

    /// Produces at most one bounded recovery generation after initial states are authoritative.
    pub fn prepare_residual_recovery<J: ExecutionJournal>(
        &mut self,
        timestamp: UnixNanos,
        journal: &mut J,
    ) -> Result<Option<ExecutionCommandBatch>, CoordinatorError> {
        if self.state != BasketState::Imbalanced
            || self.intent.purpose() == ExecutionIntentPurpose::EmergencyFlatten
            || self.children.iter().any(|child| {
                child.state == ChildOrderState::Unknown
                    || child.cancel_state == CancelState::Unknown
            })
        {
            return Err(CoordinatorError::InvalidBasketState);
        }
        if !self
            .children
            .iter()
            .take(self.intent.legs().len())
            .all(ChildOrderSnapshot::is_terminal)
            || self
                .children
                .iter()
                .skip(self.intent.legs().len())
                .any(|child| !child.is_terminal())
        {
            return Err(CoordinatorError::RecoveryAwaitingAuthority);
        }
        if self.recovery_attempts >= self.max_recovery_attempts {
            self.fail_safe_transaction(
                timestamp,
                "bounded residual recovery attempts exhausted",
                journal,
            )?;
            return Ok(None);
        }
        let plan = match plan_residual_recovery(
            &self.children,
            self.residual,
            self.intent.max_slippage_bps(),
        ) {
            Ok(plan) => plan,
            Err(_) => {
                self.fail_safe_transaction(
                    timestamp,
                    "no bounded residual recovery can strictly improve risk",
                    journal,
                )?;
                return Ok(None);
            }
        };
        let attempt = self.recovery_attempts + 1;
        let recovery_intent_id = stable_recovery_intent_id(self.intent.intent_id(), attempt)?;
        let duration = self
            .intent
            .expiry()
            .0
            .checked_sub(self.intent.created_at().0)
            .ok_or(CoordinatorError::InvalidExecutionTime)?;
        let expiry = UnixNanos(
            timestamp
                .0
                .checked_add(duration)
                .ok_or(CoordinatorError::InvalidExecutionTime)?,
        );
        let fill_evidence = self.current_fill_evidence()?;
        let recovery_intent = ExecutionIntent::new_residual_hedge(ResidualHedgeIntentRequest {
            intent_id: recovery_intent_id.clone(),
            parent: self.intent.clone(),
            fill_evidence,
            recovery_leg: plan.leg_request,
            maximum_hedge_notional: plan.maximum_hedge_notional,
            created_at: timestamp,
            expiry,
        })?;
        let leg_index = self.children.len();
        let recovery_leg = recovery_intent
            .legs()
            .first()
            .cloned()
            .ok_or(CoordinatorError::InvalidRootIntent)?;
        let (command_id, client_order_id) = stable_submit_ids(
            self.intent.intent_id(),
            leg_index,
            attempt,
            ExecutionIntentPurpose::ResidualHedge,
        )?;
        let command = ExecutionCommand::Submit(SubmitOrderCommand {
            command_id: command_id.clone(),
            client_order_id: client_order_id.clone(),
            basket_intent_id: self.intent.intent_id().clone(),
            recovery_intent_id: Some(recovery_intent_id),
            purpose: ExecutionIntentPurpose::ResidualHedge,
            leg_index,
            generation: attempt,
            leg: recovery_leg.clone(),
            limit_price: recovery_leg.price_guard.price(),
            created_at: timestamp,
        });
        let child = new_child(
            self.intent.intent_id().clone(),
            leg_index,
            command_id,
            client_order_id,
            recovery_leg,
            true,
            attempt,
        );
        let records = vec![
            ExecutionJournalRecord::RecoveryIntentPrepared {
                intent: recovery_intent.clone(),
                attempt,
                reason: format!(
                    "close actual fill imbalance using source child {} with strictly smaller projected residual",
                    plan.source_child_index
                ),
                timestamp,
            },
            ExecutionJournalRecord::ChildPrepared {
                child: child.clone(),
                timestamp,
            },
            ExecutionJournalRecord::CommandPrepared {
                command: command.clone(),
                timestamp,
            },
            ExecutionJournalRecord::StateTransition {
                from: BasketState::Imbalanced,
                to: BasketState::Hedging,
                reason: "bounded residual hedge journaled".to_owned(),
                timestamp,
            },
        ];
        journal.append_batch(&records)?;
        self.children.push(child);
        self.recovery_intents.push(recovery_intent);
        self.recovery_attempts = attempt;
        self.state = BasketState::Hedging;
        Ok(Some(ExecutionCommandBatch {
            basket_intent_id: self.intent.intent_id().clone(),
            emitted_at: timestamp,
            commands: vec![command],
        }))
    }

    fn apply_event(&mut self, event: &ExecutionEvent) -> Result<bool, CoordinatorError> {
        let child_index = self
            .children
            .iter()
            .position(|child| &child.client_order_id == event.client_order_id())
            .ok_or(CoordinatorError::UnknownChild)?;
        if let ExecutionEvent::Fill { fill, .. } = event
            && let Some(existing) = self
                .children
                .iter()
                .flat_map(|child| &child.fills)
                .find(|existing| existing.fill_id == fill.fill_id)
        {
            if existing == fill {
                return Ok(true);
            }
            self.mark_failed_safe("duplicate fill ID carried conflicting facts");
            return Ok(false);
        }
        let child = &mut self.children[child_index];
        match event {
            ExecutionEvent::Acknowledged {
                venue_order_id,
                exchange_ts,
                receive_ts,
                ..
            } => {
                if let Some(existing) = &child.venue_order_id {
                    if existing == venue_order_id {
                        return Ok(true);
                    }
                    self.mark_failed_safe("acknowledgement changed the venue order identity");
                    return Ok(false);
                }
                child.venue_order_id = Some(venue_order_id.clone());
                child.last_exchange_ts = max_time(child.last_exchange_ts, *exchange_ts);
                child.last_receive_ts = max_time(child.last_receive_ts, *receive_ts);
                if !matches!(
                    child.state,
                    ChildOrderState::PartiallyFilled
                        | ChildOrderState::Filled
                        | ChildOrderState::Canceled
                        | ChildOrderState::Rejected
                ) {
                    child.state = ChildOrderState::AcceptedOpen;
                }
            }
            ExecutionEvent::Rejected {
                reason,
                exchange_ts,
                receive_ts,
                ..
            } => {
                if child.state == ChildOrderState::Rejected {
                    return Ok(true);
                }
                if reason.trim().is_empty() {
                    self.mark_failed_safe("order rejection omitted its reason");
                    return Ok(false);
                }
                if child.cumulative_filled_qty > Decimal::ZERO {
                    self.mark_failed_safe("order rejection conflicts with accepted fill evidence");
                    return Ok(false);
                }
                child.last_exchange_ts = max_time(child.last_exchange_ts, *exchange_ts);
                child.last_receive_ts = max_time(child.last_receive_ts, *receive_ts);
                child.state = ChildOrderState::Rejected;
            }
            ExecutionEvent::Fill { fill, .. } => {
                let coherent_notional = fill
                    .cumulative_filled_qty
                    .value()
                    .checked_mul(fill.average_fill_price.value());
                let within_guard = match child.leg.price_guard {
                    PriceGuard::MaximumBuy(maximum) => {
                        child.leg.side == OrderSide::Buy
                            && fill.average_fill_price.value() <= maximum.value()
                    }
                    PriceGuard::MinimumSell(minimum) => {
                        child.leg.side == OrderSide::Sell
                            && fill.average_fill_price.value() >= minimum.value()
                    }
                };
                if !within_guard
                    || coherent_notional != Some(fill.cumulative_filled_notional.value())
                    || fill.cumulative_filled_qty.value() < child.cumulative_filled_qty
                    || fill.cumulative_filled_notional.value() < child.cumulative_filled_notional
                    || fill.cumulative_filled_qty.value() > child.leg.target_qty.value()
                    || fill.cumulative_filled_notional.value() <= Decimal::ZERO
                {
                    self.mark_failed_safe("fill cumulative facts regressed or exceeded order size");
                    return Ok(false);
                }
                child.cumulative_filled_qty = fill.cumulative_filled_qty.value();
                child.cumulative_filled_notional = fill.cumulative_filled_notional.value();
                child.average_fill_price = Some(fill.average_fill_price);
                child.cumulative_fees = fill.cumulative_fees;
                child.last_exchange_ts = max_time(child.last_exchange_ts, fill.exchange_ts);
                child.last_receive_ts = max_time(child.last_receive_ts, fill.receive_ts);
                child.fills.push(fill.clone());
                child.state = if child.cumulative_filled_qty == child.leg.target_qty.value() {
                    ChildOrderState::Filled
                } else if child.cancel_state == CancelState::Confirmed {
                    ChildOrderState::Canceled
                } else {
                    ChildOrderState::PartiallyFilled
                };
            }
            ExecutionEvent::CancelConfirmed {
                exchange_ts,
                receive_ts,
                ..
            } => {
                if child.cancel_state == CancelState::Confirmed {
                    return Ok(true);
                }
                if child.cancel_state != CancelState::Requested {
                    self.mark_failed_safe("cancel confirmation had no journaled cancel request");
                    return Ok(false);
                }
                child.cancel_state = CancelState::Confirmed;
                child.last_exchange_ts = max_time(child.last_exchange_ts, *exchange_ts);
                child.last_receive_ts = max_time(child.last_receive_ts, *receive_ts);
                if child.state != ChildOrderState::Filled {
                    child.state = ChildOrderState::Canceled;
                }
            }
            ExecutionEvent::CancelRejected {
                reason,
                exchange_ts,
                receive_ts,
                ..
            } => {
                if child.cancel_state == CancelState::Rejected {
                    return Ok(true);
                }
                if reason.trim().is_empty() {
                    self.mark_failed_safe("cancel rejection omitted its reason");
                    return Ok(false);
                }
                if child.cancel_state != CancelState::Requested {
                    self.mark_failed_safe("cancel rejection had no journaled cancel request");
                    return Ok(false);
                }
                child.cancel_state = CancelState::Rejected;
                child.last_exchange_ts = max_time(child.last_exchange_ts, *exchange_ts);
                child.last_receive_ts = max_time(child.last_receive_ts, *receive_ts);
            }
            ExecutionEvent::CancelUnknown { .. } => {
                if child.cancel_state == CancelState::Unknown {
                    return Ok(true);
                }
                if child.cancel_state != CancelState::Requested {
                    self.mark_failed_safe("ambiguous cancel had no journaled cancel request");
                    return Ok(false);
                }
                child.cancel_state = CancelState::Unknown;
                child.state = ChildOrderState::Unknown;
            }
            ExecutionEvent::AcknowledgementTimeout { .. } => {
                if child.state == ChildOrderState::Unknown {
                    return Ok(true);
                }
                if !child.is_terminal() {
                    child.state = ChildOrderState::Unknown;
                }
            }
        }
        Ok(false)
    }

    fn refresh_state(&mut self, _timestamp: UnixNanos) {
        if self.state == BasketState::FailedSafe {
            return;
        }
        let has_unknown = self.children.iter().any(|child| {
            child.state == ChildOrderState::Unknown || child.cancel_state == CancelState::Unknown
        });
        if has_unknown {
            self.state = BasketState::Unknown;
            return;
        }
        let residual_abs = absolute(self.residual.value()).unwrap_or(Decimal::MAX);
        let residual_within = residual_abs <= self.intent.max_residual_delta().value();
        let all_terminal = self.children.iter().all(ChildOrderSnapshot::is_terminal);
        if self.intent.purpose() == ExecutionIntentPurpose::EmergencyFlatten {
            if all_terminal && residual_within {
                self.state = BasketState::Complete;
                self.terminal_reason = Some(
                    "emergency order is terminal and known-position residual is within tolerance"
                        .to_owned(),
                );
            } else if all_terminal {
                self.mark_failed_safe(
                    "emergency order is terminal before known exposure reached tolerance",
                );
            } else if self
                .children
                .iter()
                .any(|child| child.cancel_state == CancelState::Requested)
            {
                self.state = BasketState::Aborting;
            } else if self
                .children
                .iter()
                .any(|child| child.cumulative_filled_notional > Decimal::ZERO)
            {
                self.state = BasketState::Partial;
            } else if self
                .children
                .iter()
                .any(|child| child.state == ChildOrderState::Submitting)
            {
                self.state = BasketState::Submitting;
            } else {
                self.state = BasketState::Pending;
            }
            return;
        }
        let active_recovery = self
            .children
            .iter()
            .skip(self.intent.legs().len())
            .any(|child| !child.is_terminal());
        if active_recovery {
            self.state = BasketState::Hedging;
        } else if !residual_within {
            self.state = BasketState::Imbalanced;
        } else if all_terminal {
            self.state = BasketState::Complete;
            self.terminal_reason = Some(
                if self
                    .children
                    .iter()
                    .any(|child| child.cumulative_filled_notional > Decimal::ZERO)
                {
                    "all orders terminal and actual-fill residual is within tolerance".to_owned()
                } else {
                    "basket aborted before any fill".to_owned()
                },
            );
        } else if self
            .children
            .iter()
            .take(self.intent.legs().len())
            .any(|child| {
                matches!(
                    child.state,
                    ChildOrderState::Rejected | ChildOrderState::Canceled
                )
            })
            || self
                .children
                .iter()
                .any(|child| child.cancel_state == CancelState::Requested)
        {
            self.state = BasketState::Aborting;
        } else if self
            .children
            .iter()
            .any(|child| child.cumulative_filled_notional > Decimal::ZERO)
        {
            self.state = BasketState::Partial;
        } else if self
            .children
            .iter()
            .any(|child| child.state == ChildOrderState::Submitting)
        {
            self.state = BasketState::Submitting;
        } else {
            self.state = BasketState::Pending;
        }
    }

    fn current_fill_evidence(&self) -> Result<Vec<ResidualFillEvidence>, CoordinatorError> {
        self.children
            .iter()
            .enumerate()
            .filter(|(_, child)| child.cumulative_filled_notional > Decimal::ZERO)
            .map(|(leg_index, child)| {
                let latest = child
                    .fills
                    .last()
                    .ok_or(CoordinatorError::InvalidBasketState)?;
                Ok(ResidualFillEvidence {
                    fill_id: latest.fill_id.clone(),
                    parent_intent_id: self.intent.intent_id().clone(),
                    leg_index,
                    venue: child.leg.venue.clone(),
                    instrument: child.leg.instrument.clone(),
                    side: child.leg.side,
                    filled_qty: latest.cumulative_filled_qty,
                    filled_notional: latest.cumulative_filled_notional,
                    exchange_ts: latest.exchange_ts,
                    receive_ts: latest.receive_ts,
                })
            })
            .collect()
    }

    fn fail_safe_transaction<J: ExecutionJournal>(
        &mut self,
        timestamp: UnixNanos,
        reason: &str,
        journal: &mut J,
    ) -> Result<(), CoordinatorError> {
        let records = vec![
            ExecutionJournalRecord::StateTransition {
                from: self.state,
                to: BasketState::FailedSafe,
                reason: reason.to_owned(),
                timestamp,
            },
            ExecutionJournalRecord::Terminal {
                state: BasketState::FailedSafe,
                reason: reason.to_owned(),
                required_authority: Some(RiskDecision::FlattenRequired),
                timestamp,
            },
        ];
        journal.append_batch(&records)?;
        self.mark_failed_safe(reason);
        Ok(())
    }

    fn mark_failed_safe(&mut self, reason: &str) {
        self.state = BasketState::FailedSafe;
        self.required_authority = Some(RiskDecision::FlattenRequired);
        self.terminal_reason = Some(reason.to_owned());
    }
}

fn validate_execution_time(
    intent: &ExecutionIntent,
    preflight_at: UnixNanos,
) -> Result<(), CoordinatorError> {
    if preflight_at < intent.created_at() || preflight_at >= intent.expiry() {
        return Err(CoordinatorError::InvalidExecutionTime);
    }
    Ok(())
}

fn preflight_increase(
    intent: &ExecutionIntent,
    input: IncreasePreflightInput<'_>,
) -> Result<IncreasePreflightSnapshot, CoordinatorError> {
    let assessment = &intent.risk_context().assessment;
    if input.preflight_at < assessment.evaluated_at()
        || intent.created_at() < assessment.evaluated_at()
    {
        return Err(CoordinatorError::InvalidExecutionTime);
    }
    let base_age = assessment
        .measurement_age_ms()
        .ok_or(CoordinatorError::StaleAuthorization)?;
    let elapsed_nanos = input.preflight_at.0 - assessment.evaluated_at().0;
    let elapsed_ms = elapsed_nanos
        .checked_add(NANOS_PER_MILLISECOND - 1)
        .ok_or(CoordinatorError::InvalidExecutionTime)?
        / NANOS_PER_MILLISECOND;
    let total_age = base_age
        .0
        .checked_add(elapsed_ms)
        .ok_or(CoordinatorError::StaleAuthorization)?;
    if total_age > assessment.limits().max_measurement_age_ms.0 {
        return Err(CoordinatorError::StaleAuthorization);
    }
    let source = intent
        .source_inventory()
        .ok_or(CoordinatorError::InvalidRootIntent)?;
    let basis = source
        .increase_size_basis
        .as_ref()
        .ok_or(CoordinatorError::InvalidRootIntent)?;
    if basis.measurement_config_fingerprint != input.measurement_config_fingerprint.as_str() {
        return Err(CoordinatorError::PreflightIdentityMismatch);
    }
    let [long_leg, short_leg] = intent.legs() else {
        return Err(CoordinatorError::InvalidRootIntent);
    };
    if input.long_book.venue_id != long_leg.venue
        || input.short_book.venue_id != short_leg.venue
        || input.long_book.instrument_id != long_leg.instrument
        || input.short_book.instrument_id != short_leg.instrument
        || input.fair_value.tick_ts != input.preflight_at
    {
        return Err(CoordinatorError::PreflightIdentityMismatch);
    }
    let requested_quantity = BaseQty::new(
        long_leg
            .target_qty
            .value()
            .max(short_leg.target_qty.value()),
    )
    .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let executable = SpreadEngine::measure(RouteMeasurementInput {
        pair_id: intent.pair_id(),
        symbol: intent.symbol(),
        long_book: input.long_book,
        short_book: input.short_book,
        long_health: input.long_health,
        short_health: input.short_health,
        requested_base_quantity: requested_quantity,
        max_book_age_ms: input.max_book_age_ms,
        max_receive_skew_ms: input.max_receive_skew_ms,
        observed_at: input.preflight_at,
        costs: input.costs,
    })
    .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let model = OpportunityModel::new(input.regime_config, input.measurement_config_fingerprint)
        .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let evaluation = model
        .evaluate(OpportunityInput {
            executable,
            fair_value: input.fair_value,
            other_explicit_risk_costs_bps: input.other_explicit_risk_costs_bps,
        })
        .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    if !evaluation.opportunity.increase_risk_economically_allowed
        || evaluation
            .opportunity
            .tradable_edge_bps
            .is_none_or(|edge| edge.value() <= Decimal::ZERO)
    {
        return Err(CoordinatorError::PreflightEconomicsRejected);
    }
    validate_price_safety(
        long_leg,
        evaluation.spread.executable_long_price,
        intent.max_slippage_bps(),
    )?;
    validate_price_safety(
        short_leg,
        evaluation.spread.executable_short_price,
        intent.max_slippage_bps(),
    )?;
    Ok(IncreasePreflightSnapshot {
        preflight_id: input.preflight_id,
        preflight_at: input.preflight_at,
        long_book: input.long_book.clone(),
        short_book: input.short_book.clone(),
        long_health: input.long_health.clone(),
        short_health: input.short_health.clone(),
        evaluation,
    })
}

fn validate_price_safety(
    leg: &ExecutionLeg,
    executable_price: Price,
    max_slippage: Bps,
) -> Result<(), CoordinatorError> {
    let guard_ok = match leg.price_guard {
        PriceGuard::MaximumBuy(maximum) => executable_price.value() <= maximum.value(),
        PriceGuard::MinimumSell(minimum) => executable_price.value() >= minimum.value(),
    };
    if !guard_ok {
        return Err(CoordinatorError::PriceSafetyViolation);
    }
    let adverse = match leg.side {
        OrderSide::Buy => executable_price
            .value()
            .checked_sub(leg.reference_price.value()),
        OrderSide::Sell => leg
            .reference_price
            .value()
            .checked_sub(executable_price.value()),
    }
    .ok_or(CoordinatorError::PriceSafetyViolation)?;
    let adverse = adverse.max(Decimal::ZERO);
    let slippage = adverse
        .checked_div(leg.reference_price.value())
        .and_then(|value| value.checked_mul(BPS_SCALE))
        .ok_or(CoordinatorError::PriceSafetyViolation)?;
    if slippage > max_slippage.value() {
        return Err(CoordinatorError::PriceSafetyViolation);
    }
    Ok(())
}

fn new_child(
    intent_id: IntentId,
    leg_index: usize,
    command_id: CommandId,
    client_order_id: ClientOrderId,
    leg: ExecutionLeg,
    recovery: bool,
    generation: u32,
) -> ChildOrderSnapshot {
    ChildOrderSnapshot {
        intent_id,
        leg_index,
        generation,
        command_id,
        client_order_id,
        venue_order_id: None,
        leg,
        state: ChildOrderState::Submitting,
        cancel_state: CancelState::NotRequested,
        cumulative_filled_qty: Decimal::ZERO,
        cumulative_filled_notional: Decimal::ZERO,
        average_fill_price: None,
        cumulative_fees: None,
        last_exchange_ts: None,
        last_receive_ts: None,
        fills: Vec::new(),
        recovery,
    }
}

fn stable_submit_ids(
    intent_id: &IntentId,
    leg_index: usize,
    generation: u32,
    purpose: ExecutionIntentPurpose,
) -> Result<(CommandId, ClientOrderId), CoordinatorError> {
    let tag = match purpose {
        ExecutionIntentPurpose::ResidualHedge => "hedge",
        ExecutionIntentPurpose::EmergencyFlatten => "emergency",
        ExecutionIntentPurpose::IncreaseRisk | ExecutionIntentPurpose::ReduceRisk => "initial",
    };
    let base = format!("{}-{tag}-l{leg_index}-g{generation}", intent_id.as_str());
    let client_order_id = ClientOrderId::try_from(base.clone())
        .map_err(|_| CoordinatorError::InvalidStableIdentity)?;
    let command_id = CommandId::try_from(format!("{base}-submit"))
        .map_err(|_| CoordinatorError::InvalidStableIdentity)?;
    Ok((command_id, client_order_id))
}

fn stable_cancel_id(client_order_id: &ClientOrderId) -> Result<CommandId, CoordinatorError> {
    CommandId::try_from(format!("{}-cancel", client_order_id.as_str()))
        .map_err(|_| CoordinatorError::InvalidStableIdentity)
}

fn stable_recovery_intent_id(
    intent_id: &IntentId,
    attempt: u32,
) -> Result<IntentId, CoordinatorError> {
    IntentId::try_from(format!("{}-hedge-{attempt}", intent_id.as_str()))
        .map_err(|_| CoordinatorError::InvalidStableIdentity)
}

fn max_time(current: Option<UnixNanos>, candidate: UnixNanos) -> Option<UnixNanos> {
    Some(current.map_or(candidate, |current| current.max(candidate)))
}

fn append_state_records(
    records: &mut Vec<ExecutionJournalRecord>,
    from: BasketState,
    to: BasketState,
    reason: &str,
    timestamp: UnixNanos,
) {
    if from == to {
        return;
    }
    if to == BasketState::Complete && from != BasketState::Balanced {
        records.push(ExecutionJournalRecord::StateTransition {
            from,
            to: BasketState::Balanced,
            reason: "actual-fill residual is within tolerance".to_owned(),
            timestamp,
        });
        records.push(ExecutionJournalRecord::StateTransition {
            from: BasketState::Balanced,
            to,
            reason: reason.to_owned(),
            timestamp,
        });
    } else {
        records.push(ExecutionJournalRecord::StateTransition {
            from,
            to,
            reason: reason.to_owned(),
            timestamp,
        });
    }
}
