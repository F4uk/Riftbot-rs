use std::{error::Error, fmt};

use riftbot::{
    config::{AppConfig, measurement_config_fingerprint, parse_toml},
    domain::{
        execution_intent::{
            EmergencyFlattenIntentRequest, EmergencyPositionEvidence, ExecutionIntent,
            ExecutionIntentError, ExecutionIntentPurpose, InstrumentExecutionMetadata,
            NormalExecutionIntentRequest, NormalLegRequest, OrderPolicy, OrderSide, PriceGuard,
        },
        ids::{
            ClientOrderId, DecisionId, EvidenceId, FillId, InstrumentId, IntentId, ModelVersion,
            PairId, PreflightId, Symbol, VenueId, VenueOrderId,
        },
        inventory::{
            EffectiveActual, EffectiveInventory, TargetDirection, TargetInventory,
            TargetInventoryParams,
        },
        market::{
            BookLevel, BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook,
        },
        numeric::{
            BaseQty, Bps, Delta, DurationMillis, Money, Notional, Price, TargetFraction, UnixNanos,
        },
        risk::{
            KillState, Regime, RiskAssessment, RiskAssessmentParams, RiskContext, RiskDecision,
            RiskExposureAudit, RiskInputAction, RiskLimitsSnapshot, RiskReasonCode,
        },
        spread::FundingState,
    },
    execution::{
        coordinator::{
            CoordinatorError, ExecutionBasketCoordinator, ExecutionCommand, ExecutionCommandBatch,
            ExecutionEvent, ExecutionJournalRecord, ExecutionPort, InMemoryExecutionJournal,
            IncreasePreflightInput, PairReservationBook,
        },
        state_machine::{AppliedFill, BasketState, CancelState, ChildOrderState},
    },
    models::{
        fair_value::{FairValueSnapshot, OrientedRouteKey},
        spread_engine::RouteCostInput,
    },
    strategy::inventory_manager::{IncreaseSizeBasis, InventoryAction, InventoryDecision},
};
use rust_decimal::Decimal;
use serde_json::json;

const EXAMPLE: &str = include_str!("../config/example.toml");
const EVALUATED_AT: UnixNanos = UnixNanos(10_000_000_000);
const CREATED_AT: UnixNanos = UnixNanos(10_050_000_000);
const PREFLIGHT_AT: UnixNanos = UnixNanos(10_100_000_000);
const EXPIRY: UnixNanos = UnixNanos(13_000_000_000);
const CONFIG_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

type TestResult = Result<(), Box<dyn Error>>;
type FixtureIds = (
    DecisionId,
    PairId,
    Symbol,
    VenueId,
    VenueId,
    InstrumentId,
    InstrumentId,
);

#[derive(Debug)]
struct FakePortError;

impl fmt::Display for FakePortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("fake port error")
    }
}

impl Error for FakePortError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FakeExecutionPort {
    batches: Vec<ExecutionCommandBatch>,
    real_network_calls: usize,
}

impl ExecutionPort for FakeExecutionPort {
    type Error = FakePortError;

    fn dispatch(&mut self, batch: &ExecutionCommandBatch) -> Result<(), Self::Error> {
        self.batches.push(batch.clone());
        Ok(())
    }
}

struct PreparedBasket {
    coordinator: ExecutionBasketCoordinator,
    journal: InMemoryExecutionJournal,
    reservations: PairReservationBook,
    batch: ExecutionCommandBatch,
}

fn config() -> Result<AppConfig, Box<dyn Error>> {
    Ok(parse_toml(EXAMPLE)?)
}

fn ids() -> Result<FixtureIds, Box<dyn Error>> {
    Ok((
        DecisionId::try_from("p6-decision")?,
        PairId::try_from("sndk_entropy_lighter")?,
        Symbol::try_from("SNDK")?,
        VenueId::try_from("entropy")?,
        VenueId::try_from("lighter")?,
        InstrumentId::try_from("io:SNDK-USD-PERP.HYPERLIQUID")?,
        InstrumentId::try_from("SNDK-PERP.LIGHTER")?,
    ))
}

fn limits() -> Result<RiskLimitsSnapshot, Box<dyn Error>> {
    Ok(RiskLimitsSnapshot {
        max_venue_notional: Notional::new(Decimal::from(1_000))?,
        max_pair_notional_per_leg: Notional::new(Decimal::from(1_500))?,
        max_global_delta: Delta::new(Decimal::from(25)),
        max_session_loss: Notional::new(Decimal::from(100))?,
        max_measurement_age_ms: DurationMillis(1_500),
        degraded_authorization_fraction: TargetFraction::new(Decimal::new(5, 1))?,
        session_loss_required_state: KillState::Flatten,
    })
}

fn exposure_audit(
    action: RiskInputAction,
    current: Decimal,
    proposed: Decimal,
    authorized: Decimal,
) -> Result<RiskExposureAudit, Box<dyn Error>> {
    let (_, _, _, long_venue, short_venue, _, _) = ids()?;
    let candidate = match action {
        RiskInputAction::IncreaseRisk => current + proposed,
        RiskInputAction::ReduceRisk | RiskInputAction::FlattenForReversal => current - proposed,
        _ => current,
    };
    let authorized_projected = match action {
        RiskInputAction::IncreaseRisk => current + authorized,
        RiskInputAction::ReduceRisk | RiskInputAction::FlattenForReversal => current - authorized,
        _ => current,
    };
    Ok(RiskExposureAudit {
        pair_current_notional_per_leg: Notional::new(current)?,
        pair_candidate_projected_notional_per_leg: Notional::new(candidate)?,
        pair_authorized_projected_notional_per_leg: Notional::new(authorized_projected)?,
        long_venue,
        long_current_notional: Notional::new(current)?,
        long_candidate_projected_notional: Notional::new(candidate)?,
        long_authorized_projected_notional: Notional::new(authorized_projected)?,
        short_venue,
        short_current_notional: Notional::new(current)?,
        short_candidate_projected_notional: Notional::new(candidate)?,
        short_authorized_projected_notional: Notional::new(authorized_projected)?,
        global_delta_current: Delta::new(Decimal::ZERO),
        global_delta_candidate_projected: Delta::new(Decimal::ZERO),
        global_delta_authorized_projected: Delta::new(Decimal::ZERO),
        session_pnl: Some(Money::new(Decimal::ZERO)),
        session_loss: Some(Notional::new(Decimal::ZERO)?),
    })
}

fn risk_context(
    action: RiskInputAction,
    decision: RiskDecision,
    regime: Regime,
    kill_state: KillState,
    current: Decimal,
    proposed: Decimal,
    authorized: Decimal,
) -> Result<RiskContext, Box<dyn Error>> {
    let (decision_id, _, _, _, _, _, _) = ids()?;
    let requested = if action.is_reduction() {
        -proposed
    } else if action == RiskInputAction::IncreaseRisk {
        Decimal::from(125)
    } else {
        proposed
    };
    let assessment = RiskAssessment::new(RiskAssessmentParams {
        decision_id,
        evaluated_at: EVALUATED_AT,
        input_action: action,
        requested_change_notional_per_leg: Money::new(requested),
        proposed_change_notional_per_leg: Notional::new(proposed)?,
        authorized_change_notional_per_leg: Notional::new(authorized)?,
        decision,
        regime,
        kill_state,
        reason_codes: vec![if decision == RiskDecision::Approve {
            RiskReasonCode::Approved
        } else {
            RiskReasonCode::KillReduceOnly
        }],
        explanation: "typed P5 fixture authority".to_owned(),
        exposure: Some(exposure_audit(action, current, proposed, authorized)?),
        measurement_age_ms: (action == RiskInputAction::IncreaseRisk)
            .then_some(DurationMillis(100)),
        measurement_safe_matched_notional_cap: (action == RiskInputAction::IncreaseRisk)
            .then_some(Notional::new(Decimal::from(50))?),
        limits: limits()?,
        config_fingerprint: CONFIG_SHA256.to_owned(),
    })?;
    Ok(RiskContext {
        regime,
        kill_state,
        assessment,
    })
}

fn inventory(
    purpose: ExecutionIntentPurpose,
    measurement_fingerprint: &str,
) -> Result<InventoryDecision, Box<dyn Error>> {
    let (decision_id, pair_id, symbol, long_venue, short_venue, _, _) = ids()?;
    let direction = TargetDirection::LongShort {
        long_venue,
        short_venue,
    };
    let (action, target, actual, required, proposed, basis) = match purpose {
        ExecutionIntentPurpose::IncreaseRisk => (
            InventoryAction::IncreaseRisk,
            Decimal::from(125),
            Decimal::ZERO,
            Decimal::from(125),
            Decimal::from(50),
            Some(IncreaseSizeBasis {
                requested_base_quantity: BaseQty::new(Decimal::new(5, 1))?,
                long_measured_notional: Notional::new(Decimal::from(50))?,
                short_measured_notional: Notional::new(Decimal::new(505, 1))?,
                measured_matched_notional_cap: Notional::new(Decimal::from(50))?,
                observed_at: UnixNanos(EVALUATED_AT.0 - 100_000_000),
                measurement_model_version: ModelVersion::try_from("p3-measurement-v1")?,
                measurement_config_fingerprint: measurement_fingerprint.to_owned(),
            }),
        ),
        ExecutionIntentPurpose::ReduceRisk => (
            InventoryAction::ReduceRisk,
            Decimal::from(100),
            Decimal::from(200),
            Decimal::from(-100),
            Decimal::from(100),
            None,
        ),
        _ => return Err("normal inventory purpose required".into()),
    };
    Ok(InventoryDecision {
        decision_id: decision_id.clone(),
        pair_id: pair_id.clone(),
        symbol: symbol.clone(),
        action,
        selected_target: Some(TargetInventory::new(TargetInventoryParams {
            symbol,
            pair_id,
            target_fraction: TargetFraction::new(Decimal::new(8, 1))?,
            target_notional: Notional::new(target)?,
            direction: direction.clone(),
            reason: "P6 normal intent fixture".to_owned(),
            model_version: ModelVersion::try_from("p4-grid-inventory-v1")?,
            decision_id,
        })?),
        effective_actual: Some(EffectiveActual {
            direction,
            actual_notional_per_leg: Notional::new(actual)?,
            reserved_notional_per_leg: Notional::new(Decimal::ZERO)?,
            pending_notional_per_leg: Notional::new(Decimal::ZERO)?,
            total_notional_per_leg: Notional::new(actual)?,
        }),
        required_change_notional_per_leg: Money::new(required),
        proposed_change_notional_per_leg: Notional::new(proposed)?,
        increase_size_basis: basis,
        block_reason: None,
    })
}

fn leg_requests(
    lot_size: Decimal,
    buy_guard: Decimal,
    sell_guard: Decimal,
) -> Result<[NormalLegRequest; 2], Box<dyn Error>> {
    let (_, _, _, long_venue, short_venue, long_instrument, short_instrument) = ids()?;
    let request = |venue: VenueId,
                   instrument: InstrumentId,
                   reference: Decimal,
                   guard: PriceGuard|
     -> Result<NormalLegRequest, Box<dyn Error>> {
        Ok(NormalLegRequest {
            venue,
            instrument: instrument.clone(),
            reference_price: Price::new(reference)?,
            metadata: InstrumentExecutionMetadata {
                instrument,
                lot_size: BaseQty::new(lot_size)?,
                quantity_precision: 4,
                supports_reduce_only: true,
            },
            order_policy: OrderPolicy::MarketableLimit,
            price_guard: guard,
        })
    };
    Ok([
        request(
            long_venue,
            long_instrument,
            Decimal::from(100),
            PriceGuard::MaximumBuy(Price::new(buy_guard)?),
        )?,
        request(
            short_venue,
            short_instrument,
            Decimal::from(101),
            PriceGuard::MinimumSell(Price::new(sell_guard)?),
        )?,
    ])
}

fn normal_intent(purpose: ExecutionIntentPurpose) -> Result<ExecutionIntent, Box<dyn Error>> {
    let requests = if purpose == ExecutionIntentPurpose::ReduceRisk {
        let mut requests = leg_requests(Decimal::new(1, 2), Decimal::from(102), Decimal::from(99))?;
        requests[0].price_guard = PriceGuard::MinimumSell(Price::new(Decimal::from(99))?);
        requests[1].price_guard = PriceGuard::MaximumBuy(Price::new(Decimal::from(102))?);
        requests
    } else {
        leg_requests(Decimal::new(1, 2), Decimal::from(101), Decimal::from(100))?
    };
    build_normal_intent(purpose, requests, CREATED_AT, EXPIRY)
}

fn build_normal_intent(
    purpose: ExecutionIntentPurpose,
    leg_requests: [NormalLegRequest; 2],
    created_at: UnixNanos,
    expiry: UnixNanos,
) -> Result<ExecutionIntent, Box<dyn Error>> {
    let config = config()?;
    let fingerprint = measurement_config_fingerprint(&config)?;
    let (action, decision, regime, kill_state, current, proposed, authorized) = match purpose {
        ExecutionIntentPurpose::IncreaseRisk => (
            RiskInputAction::IncreaseRisk,
            RiskDecision::Approve,
            Regime::Normal,
            KillState::Ready,
            Decimal::ZERO,
            Decimal::from(50),
            Decimal::from(50),
        ),
        ExecutionIntentPurpose::ReduceRisk => (
            RiskInputAction::ReduceRisk,
            RiskDecision::ReduceOnly,
            Regime::ReduceOnly,
            KillState::ReduceOnly,
            Decimal::from(200),
            Decimal::from(100),
            Decimal::from(100),
        ),
        _ => return Err("normal purpose required".into()),
    };
    Ok(ExecutionIntent::new_normal(NormalExecutionIntentRequest {
        intent_id: IntentId::try_from(match purpose {
            ExecutionIntentPurpose::IncreaseRisk => "p6-increase-intent",
            ExecutionIntentPurpose::ReduceRisk => "p6-reduce-intent",
            _ => unreachable!(),
        })?,
        purpose,
        source_inventory: inventory(purpose, fingerprint.as_str())?,
        risk_context: risk_context(
            action, decision, regime, kill_state, current, proposed, authorized,
        )?,
        leg_requests,
        created_at,
        expiry,
        max_residual_delta: config.execution.max_residual_delta,
        max_slippage_bps: config.execution.max_slippage_bps,
    })?)
}

fn book(
    venue_id: VenueId,
    instrument_id: InstrumentId,
    bid: Decimal,
    ask: Decimal,
) -> Result<VenueBook, Box<dyn Error>> {
    book_at(venue_id, instrument_id, bid, ask, PREFLIGHT_AT)
}

fn book_at(
    venue_id: VenueId,
    instrument_id: InstrumentId,
    bid: Decimal,
    ask: Decimal,
    at: UnixNanos,
) -> Result<VenueBook, Box<dyn Error>> {
    Ok(VenueBook {
        venue_id,
        instrument_id,
        bids: vec![BookLevel {
            price: Price::new(bid)?,
            quantity: BaseQty::new(Decimal::from(10))?,
        }],
        asks: vec![BookLevel {
            price: Price::new(ask)?,
            quantity: BaseQty::new(Decimal::from(10))?,
        }],
        exchange_ts: UnixNanos(at.0 - 20_000_000),
        receive_ts: UnixNanos(at.0 - 10_000_000),
        age_ms: DurationMillis(10),
        version: BookVersion(1),
    })
}

fn health(book: &VenueBook) -> FeedHealth {
    FeedHealth {
        venue_id: book.venue_id.clone(),
        instrument_id: book.instrument_id.clone(),
        connection: FeedConnectionState::Connected,
        freshness: FeedFreshness::Fresh,
        last_transition_ts: UnixNanos(PREFLIGHT_AT.0 - 30_000_000),
        last_exchange_ts: Some(book.exchange_ts),
        last_receive_ts: Some(book.receive_ts),
        age_ms: Some(DurationMillis(10)),
    }
}

fn prepare_increase() -> Result<PreparedBasket, Box<dyn Error>> {
    let config = config()?;
    let fingerprint = measurement_config_fingerprint(&config)?;
    let mut coordinator = ExecutionBasketCoordinator::new(
        normal_intent(ExecutionIntentPurpose::IncreaseRisk)?,
        config.execution.max_recovery_attempts,
    )?;
    let (_, pair_id, _, long_venue, short_venue, long_instrument, short_instrument) = ids()?;
    let long_book = book(
        long_venue.clone(),
        long_instrument,
        Decimal::from(99),
        Decimal::from(100),
    )?;
    let short_book = book(
        short_venue.clone(),
        short_instrument,
        Decimal::from(101),
        Decimal::from(102),
    )?;
    let long_health = health(&long_book);
    let short_health = health(&short_book);
    let fair_value = FairValueSnapshot {
        route: OrientedRouteKey::new(pair_id, long_venue, short_venue),
        tick_ts: PREFLIGHT_AT,
        reference_basis_bps: Some(Bps::new(Decimal::from(90))),
        midline_bps: Some(Bps::new(Decimal::from(90))),
        dispersion_bps: Some(Bps::new(Decimal::ONE)),
        sample_count: 300,
        minimum_samples: 300,
        warmed_up: true,
        rejection: None,
    };
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    let batch = coordinator.prepare_increase(
        IncreasePreflightInput {
            preflight_id: PreflightId::try_from("p6-preflight")?,
            preflight_at: PREFLIGHT_AT,
            long_book: &long_book,
            short_book: &short_book,
            long_health: &long_health,
            short_health: &short_health,
            fair_value,
            costs: RouteCostInput {
                long_taker_fee_bps: Some(Bps::new(Decimal::ONE)),
                short_taker_fee_bps: Some(Bps::new(Decimal::ONE)),
                execution_buffer_bps: Bps::new(Decimal::ONE),
                funding_state: FundingState::Disabled,
                funding_adjustment_bps: None,
            },
            other_explicit_risk_costs_bps: Vec::new(),
            max_book_age_ms: DurationMillis(1_500),
            max_receive_skew_ms: DurationMillis(500),
            regime_config: &config.regime,
            measurement_config_fingerprint: &fingerprint,
        },
        &mut journal,
        &mut reservations,
    )?;
    Ok(PreparedBasket {
        coordinator,
        journal,
        reservations,
        batch,
    })
}

fn prepare_increase_at(
    coordinator: &mut ExecutionBasketCoordinator,
    at: UnixNanos,
    long_ask: Decimal,
    short_bid: Decimal,
    journal: &mut InMemoryExecutionJournal,
    reservations: &mut PairReservationBook,
) -> Result<ExecutionCommandBatch, CoordinatorError> {
    let config = config().map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let fingerprint = measurement_config_fingerprint(&config)
        .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let (_, pair_id, _, long_venue, short_venue, long_instrument, short_instrument) =
        ids().map_err(|_| CoordinatorError::PreflightIdentityMismatch)?;
    let long_book = book_at(
        long_venue.clone(),
        long_instrument,
        long_ask - Decimal::ONE,
        long_ask,
        at,
    )
    .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let short_book = book_at(
        short_venue.clone(),
        short_instrument,
        short_bid,
        short_bid + Decimal::ONE,
        at,
    )
    .map_err(|_| CoordinatorError::PreflightEconomicsRejected)?;
    let long_health = health(&long_book);
    let short_health = health(&short_book);
    coordinator.prepare_increase(
        IncreasePreflightInput {
            preflight_id: PreflightId::try_from(format!("p6-preflight-{}", at.0))
                .map_err(|_| CoordinatorError::PreflightIdentityMismatch)?,
            preflight_at: at,
            long_book: &long_book,
            short_book: &short_book,
            long_health: &long_health,
            short_health: &short_health,
            fair_value: FairValueSnapshot {
                route: OrientedRouteKey::new(pair_id, long_venue, short_venue),
                tick_ts: at,
                reference_basis_bps: Some(Bps::new(Decimal::from(90))),
                midline_bps: Some(Bps::new(Decimal::from(90))),
                dispersion_bps: Some(Bps::new(Decimal::ONE)),
                sample_count: 300,
                minimum_samples: 300,
                warmed_up: true,
                rejection: None,
            },
            costs: RouteCostInput {
                long_taker_fee_bps: Some(Bps::new(Decimal::ONE)),
                short_taker_fee_bps: Some(Bps::new(Decimal::ONE)),
                execution_buffer_bps: Bps::new(Decimal::ONE),
                funding_state: FundingState::Disabled,
                funding_adjustment_bps: None,
            },
            other_explicit_risk_costs_bps: Vec::new(),
            max_book_age_ms: DurationMillis(1_500),
            max_receive_skew_ms: DurationMillis(500),
            regime_config: &config.regime,
            measurement_config_fingerprint: &fingerprint,
        },
        journal,
        reservations,
    )
}

fn prepare_reduction() -> Result<PreparedBasket, Box<dyn Error>> {
    let config = config()?;
    let mut coordinator = ExecutionBasketCoordinator::new(
        normal_intent(ExecutionIntentPurpose::ReduceRisk)?,
        config.execution.max_recovery_attempts,
    )?;
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    let batch = coordinator.prepare_reduction(PREFLIGHT_AT, &mut journal, &mut reservations)?;
    Ok(PreparedBasket {
        coordinator,
        journal,
        reservations,
        batch,
    })
}

fn child_id(prepared: &PreparedBasket, index: usize) -> ClientOrderId {
    prepared.coordinator.children()[index]
        .client_order_id
        .clone()
}

fn ack(client_order_id: ClientOrderId, sequence: u64) -> Result<ExecutionEvent, Box<dyn Error>> {
    Ok(ExecutionEvent::Acknowledged {
        client_order_id,
        venue_order_id: VenueOrderId::try_from(format!("venue-order-{sequence}"))?,
        exchange_ts: UnixNanos(PREFLIGHT_AT.0 + sequence * 1_000_000),
        receive_ts: UnixNanos(PREFLIGHT_AT.0 + sequence * 2_000_000),
    })
}

fn fill(
    prepared: &PreparedBasket,
    child_index: usize,
    fill_name: &str,
    fraction_tenths: i64,
    time_offset_ms: u64,
) -> Result<ExecutionEvent, Box<dyn Error>> {
    let child = &prepared.coordinator.children()[child_index];
    let fraction = Decimal::new(fraction_tenths, 1);
    let quantity = child.leg.target_qty.value() * fraction;
    let notional = child.leg.target_notional.value() * fraction;
    Ok(ExecutionEvent::Fill {
        client_order_id: child.client_order_id.clone(),
        fill: AppliedFill {
            fill_id: FillId::try_from(fill_name)?,
            cumulative_filled_qty: BaseQty::new(quantity)?,
            cumulative_filled_notional: Notional::new(notional)?,
            average_fill_price: child.leg.reference_price,
            cumulative_fees: Some(Notional::new(Decimal::new(1, 2))?),
            exchange_ts: UnixNanos(PREFLIGHT_AT.0 + time_offset_ms * 1_000_000),
            receive_ts: UnixNanos(PREFLIGHT_AT.0 + (time_offset_ms + 1) * 1_000_000),
        },
    })
}

fn reject(client_order_id: ClientOrderId, time_offset_ms: u64) -> ExecutionEvent {
    ExecutionEvent::Rejected {
        client_order_id,
        reason: "deterministic reject".to_owned(),
        exchange_ts: UnixNanos(PREFLIGHT_AT.0 + time_offset_ms * 1_000_000),
        receive_ts: UnixNanos(PREFLIGHT_AT.0 + (time_offset_ms + 1) * 1_000_000),
    }
}

fn cancel_confirmed(client_order_id: ClientOrderId, time_offset_ms: u64) -> ExecutionEvent {
    ExecutionEvent::CancelConfirmed {
        client_order_id,
        exchange_ts: UnixNanos(PREFLIGHT_AT.0 + time_offset_ms * 1_000_000),
        receive_ts: UnixNanos(PREFLIGHT_AT.0 + (time_offset_ms + 1) * 1_000_000),
    }
}

fn handle(event: ExecutionEvent, prepared: &mut PreparedBasket) -> Result<(), Box<dyn Error>> {
    prepared
        .coordinator
        .handle_event(event, &mut prepared.journal, &mut prepared.reservations)?;
    Ok(())
}

#[test]
fn normal_two_leg_accepted_full_fill_reaches_complete() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(ack(child_id(&prepared, 0), 1)?, &mut prepared)?;
    handle(ack(child_id(&prepared, 1), 2)?, &mut prepared)?;
    handle(fill(&prepared, 0, "fill-long", 10, 3)?, &mut prepared)?;
    handle(fill(&prepared, 1, "fill-short", 10, 4)?, &mut prepared)?;

    assert_eq!(prepared.coordinator.state(), BasketState::Complete);
    assert!(prepared.coordinator.snapshot().is_ok());
    assert!(prepared.journal.records().iter().any(|record| matches!(
        record,
        ExecutionJournalRecord::Terminal {
            state: BasketState::Complete,
            ..
        }
    )));
    Ok(())
}

#[test]
fn both_initial_submit_commands_are_emitted_together() -> TestResult {
    let prepared = prepare_increase()?;
    assert_eq!(prepared.batch.commands.len(), 2);
    assert!(prepared.batch.commands.iter().all(|command| matches!(
        command,
        ExecutionCommand::Submit(submit)
            if submit.generation == 0 && submit.recovery_intent_id.is_none()
    )));
    let command_records = prepared
        .journal
        .records()
        .iter()
        .filter(|record| matches!(record, ExecutionJournalRecord::CommandPrepared { .. }))
        .count();
    assert_eq!(command_records, 2);
    Ok(())
}

#[test]
fn one_leg_reject_other_no_fill_aborts_safely() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(reject(child_id(&prepared, 0), 1), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Aborting);
    let cancel_batch = prepared
        .coordinator
        .prepare_abort_cancels(UnixNanos(PREFLIGHT_AT.0 + 4_000_000), &mut prepared.journal)?;
    assert_eq!(cancel_batch.commands.len(), 1);
    handle(cancel_confirmed(child_id(&prepared, 1), 5), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Complete);
    assert_eq!(prepared.coordinator.residual().value(), Decimal::ZERO);
    Ok(())
}

#[test]
fn one_leg_reject_after_opposite_leg_fills_is_imbalanced() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(fill(&prepared, 0, "one-sided-fill", 10, 1)?, &mut prepared)?;
    handle(reject(child_id(&prepared, 1), 3), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    assert_eq!(prepared.coordinator.residual().value(), Decimal::from(50));
    Ok(())
}

#[test]
fn one_leg_partial_other_full_tracks_imbalance() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(fill(&prepared, 0, "partial-long", 5, 1)?, &mut prepared)?;
    handle(fill(&prepared, 1, "full-short", 10, 2)?, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    assert_eq!(
        prepared.coordinator.residual().value(),
        Decimal::new(-2449, 2)
    );
    Ok(())
}

#[test]
fn both_legs_partial_remain_partial_when_residual_is_bounded() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(fill(&prepared, 0, "partial-long", 5, 1)?, &mut prepared)?;
    handle(fill(&prepared, 1, "partial-short", 5, 2)?, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Partial);
    assert_eq!(
        prepared.coordinator.residual().value(),
        Decimal::new(255, 3)
    );
    Ok(())
}

#[test]
fn delayed_acknowledgement_preserves_submitting_until_received() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(ack(child_id(&prepared, 0), 1)?, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Submitting);
    handle(ack(child_id(&prepared, 1), 20)?, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Pending);
    Ok(())
}

#[test]
fn acknowledgement_timeout_enters_unknown_without_blind_resend() -> TestResult {
    let mut prepared = prepare_increase()?;
    let first_id = child_id(&prepared, 0);
    handle(
        ExecutionEvent::AcknowledgementTimeout {
            client_order_id: first_id.clone(),
            observed_at: UnixNanos(PREFLIGHT_AT.0 + 50_000_000),
        },
        &mut prepared,
    )?;
    assert_eq!(prepared.coordinator.state(), BasketState::Unknown);
    assert_eq!(prepared.coordinator.children().len(), 2);
    assert!(matches!(
        prepared.coordinator.prepare_residual_recovery(
            UnixNanos(PREFLIGHT_AT.0 + 60_000_000),
            &mut prepared.journal,
        ),
        Err(CoordinatorError::InvalidBasketState)
    ));
    assert_eq!(prepared.coordinator.children()[0].client_order_id, first_id);
    Ok(())
}

#[test]
fn reconciliation_requires_authoritative_event_and_preserves_child_identity() -> TestResult {
    let mut prepared = prepare_increase()?;
    let original_id = child_id(&prepared, 0);
    handle(
        ExecutionEvent::AcknowledgementTimeout {
            client_order_id: original_id.clone(),
            observed_at: UnixNanos(PREFLIGHT_AT.0 + 3_000_000),
        },
        &mut prepared,
    )?;
    prepared
        .coordinator
        .start_reconciling(UnixNanos(PREFLIGHT_AT.0 + 4_000_000), &mut prepared.journal)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Reconciling);
    handle(ack(original_id.clone(), 5)?, &mut prepared)?;
    assert_eq!(prepared.coordinator.children().len(), 2);
    assert_eq!(
        prepared.coordinator.children()[0].client_order_id,
        original_id
    );
    assert_ne!(prepared.coordinator.state(), BasketState::Complete);
    Ok(())
}

#[test]
fn duplicate_ack_is_idempotent() -> TestResult {
    let mut prepared = prepare_increase()?;
    let event = ack(child_id(&prepared, 0), 1)?;
    handle(event.clone(), &mut prepared)?;
    let after_first = prepared.coordinator.snapshot()?;
    handle(event, &mut prepared)?;
    assert_eq!(prepared.coordinator.snapshot()?, after_first);
    assert!(matches!(
        prepared.journal.records().last(),
        Some(ExecutionJournalRecord::EventApplied {
            duplicate: true,
            ..
        })
    ));
    Ok(())
}

#[test]
fn duplicate_fill_is_idempotent() -> TestResult {
    let mut prepared = prepare_increase()?;
    let event = fill(&prepared, 0, "duplicate-fill", 5, 1)?;
    handle(event.clone(), &mut prepared)?;
    let residual = prepared.coordinator.residual();
    handle(event, &mut prepared)?;
    assert_eq!(prepared.coordinator.residual(), residual);
    assert_eq!(prepared.coordinator.children()[0].fills.len(), 1);
    Ok(())
}

#[test]
fn conflicting_duplicate_fill_id_fails_closed() -> TestResult {
    let mut prepared = prepare_increase()?;
    let first = fill(&prepared, 0, "conflicting-fill-id", 5, 1)?;
    handle(first, &mut prepared)?;
    let conflicting = fill(&prepared, 0, "conflicting-fill-id", 10, 2)?;
    handle(conflicting, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::FailedSafe);
    assert_eq!(prepared.coordinator.children()[0].fills.len(), 1);
    Ok(())
}

#[test]
fn regressive_cumulative_fill_fails_closed() -> TestResult {
    let mut prepared = prepare_increase()?;
    let first = fill(&prepared, 0, "cumulative-five", 5, 1)?;
    handle(first, &mut prepared)?;
    let regressive = fill(&prepared, 0, "cumulative-two", 2, 2)?;
    handle(regressive, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::FailedSafe);
    assert_eq!(prepared.coordinator.children()[0].fills.len(), 1);
    Ok(())
}

#[test]
fn out_of_order_fill_before_ack_is_accounted_then_ack_is_idempotent_to_fill_state() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(fill(&prepared, 0, "early-fill", 5, 1)?, &mut prepared)?;
    assert_eq!(
        prepared.coordinator.children()[0].state,
        ChildOrderState::PartiallyFilled
    );
    handle(ack(child_id(&prepared, 0), 2)?, &mut prepared)?;
    assert_eq!(
        prepared.coordinator.children()[0].state,
        ChildOrderState::PartiallyFilled
    );
    Ok(())
}

#[test]
fn residual_is_calculated_from_actual_fills_not_requested_quantity() -> TestResult {
    let mut prepared = prepare_increase()?;
    handle(fill(&prepared, 0, "quarter-long", 2, 1)?, &mut prepared)?;
    assert_eq!(prepared.coordinator.residual().value(), Decimal::from(10));
    assert_ne!(
        prepared.coordinator.residual().value(),
        prepared.coordinator.intent().target_net_delta().value()
    );
    Ok(())
}

#[test]
fn no_real_order_or_network_path_is_invoked_by_tests() -> TestResult {
    let prepared = prepare_increase()?;
    let mut port = FakeExecutionPort::default();
    port.dispatch(&prepared.batch)?;
    assert_eq!(port.batches, vec![prepared.batch]);
    assert_eq!(port.real_network_calls, 0);
    Ok(())
}

#[test]
fn fill_after_cancel_request_updates_actual_residual() -> TestResult {
    let mut prepared = prepare_increase()?;
    prepared
        .coordinator
        .prepare_abort_cancels(UnixNanos(PREFLIGHT_AT.0 + 2_000_000), &mut prepared.journal)?;
    let event = fill(&prepared, 0, "fill-after-cancel-request", 10, 3)?;
    handle(event, &mut prepared)?;
    assert_eq!(prepared.coordinator.residual().value(), Decimal::from(50));
    assert_eq!(
        prepared.coordinator.children()[0].cancel_state,
        CancelState::Requested
    );
    assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    Ok(())
}

#[test]
fn late_fill_after_timeout_unknown_is_applied_without_double_hedge() -> TestResult {
    let mut prepared = prepare_increase()?;
    let first_fill = fill(&prepared, 0, "known-long-fill", 10, 1)?;
    handle(first_fill, &mut prepared)?;
    handle(
        ExecutionEvent::AcknowledgementTimeout {
            client_order_id: child_id(&prepared, 1),
            observed_at: UnixNanos(PREFLIGHT_AT.0 + 3_000_000),
        },
        &mut prepared,
    )?;
    assert_eq!(prepared.coordinator.state(), BasketState::Unknown);
    assert!(prepared.coordinator.recovery_intents().is_empty());

    let late_fill = fill(&prepared, 1, "late-short-fill", 10, 5)?;
    handle(late_fill, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Complete);
    assert_eq!(prepared.coordinator.recovery_intents().len(), 0);
    assert_eq!(prepared.coordinator.children().len(), 2);
    Ok(())
}

#[test]
fn cancel_rejected_is_distinct_from_canceled() -> TestResult {
    let mut prepared = prepare_increase()?;
    prepared
        .coordinator
        .prepare_abort_cancels(UnixNanos(PREFLIGHT_AT.0 + 2_000_000), &mut prepared.journal)?;
    handle(
        ExecutionEvent::CancelRejected {
            client_order_id: child_id(&prepared, 0),
            reason: "venue says still open".to_owned(),
            exchange_ts: UnixNanos(PREFLIGHT_AT.0 + 3_000_000),
            receive_ts: UnixNanos(PREFLIGHT_AT.0 + 4_000_000),
        },
        &mut prepared,
    )?;
    assert_eq!(
        prepared.coordinator.children()[0].cancel_state,
        CancelState::Rejected
    );
    assert_ne!(
        prepared.coordinator.children()[0].state,
        ChildOrderState::Canceled
    );
    Ok(())
}

#[test]
fn cancel_unknown_forces_unknown_basket() -> TestResult {
    let mut prepared = prepare_increase()?;
    prepared
        .coordinator
        .prepare_abort_cancels(UnixNanos(PREFLIGHT_AT.0 + 2_000_000), &mut prepared.journal)?;
    handle(
        ExecutionEvent::CancelUnknown {
            client_order_id: child_id(&prepared, 0),
            observed_at: UnixNanos(PREFLIGHT_AT.0 + 3_000_000),
        },
        &mut prepared,
    )?;
    assert_eq!(prepared.coordinator.state(), BasketState::Unknown);
    assert_eq!(
        prepared.coordinator.children()[0].cancel_state,
        CancelState::Unknown
    );
    Ok(())
}

#[test]
fn fill_after_confirmed_cancel_remains_terminal_and_updates_residual() -> TestResult {
    let mut prepared = prepare_increase()?;
    prepared
        .coordinator
        .prepare_abort_cancels(UnixNanos(PREFLIGHT_AT.0 + 2_000_000), &mut prepared.journal)?;
    handle(cancel_confirmed(child_id(&prepared, 0), 3), &mut prepared)?;
    let late_partial = fill(&prepared, 0, "late-pre-cancel-fill", 5, 5)?;
    handle(late_partial, &mut prepared)?;
    assert_eq!(
        prepared.coordinator.children()[0].state,
        ChildOrderState::Canceled
    );
    assert_eq!(prepared.coordinator.residual().value(), Decimal::from(25));
    Ok(())
}

#[test]
fn late_fill_reopens_completed_cancel_basket_and_reacquires_reservation() -> TestResult {
    let mut prepared = prepare_increase()?;
    prepared
        .coordinator
        .prepare_abort_cancels(UnixNanos(PREFLIGHT_AT.0 + 2_000_000), &mut prepared.journal)?;
    handle(cancel_confirmed(child_id(&prepared, 0), 3), &mut prepared)?;
    handle(cancel_confirmed(child_id(&prepared, 1), 5), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Complete);
    assert!(
        prepared
            .reservations
            .active(prepared.coordinator.intent().pair_id())
            .is_none()
    );

    let late_fill = fill(&prepared, 0, "fill-after-terminal-cancel", 10, 7)?;
    handle(late_fill, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    assert!(
        prepared
            .reservations
            .active(prepared.coordinator.intent().pair_id())
            .is_some_and(|reservation| {
                reservation.intent_id == *prepared.coordinator.intent().intent_id()
            })
    );
    Ok(())
}

#[test]
fn inconsistent_cumulative_fill_arithmetic_fails_closed() -> TestResult {
    let mut prepared = prepare_increase()?;
    let mut event = fill(&prepared, 0, "bad-fill-arithmetic", 5, 1)?;
    if let ExecutionEvent::Fill { fill, .. } = &mut event {
        fill.cumulative_filled_notional =
            Notional::new(fill.cumulative_filled_notional.value() + Decimal::ONE)?;
    }
    handle(event, &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::FailedSafe);
    assert_eq!(prepared.coordinator.residual().value(), Decimal::ZERO);
    assert_eq!(
        prepared.coordinator.snapshot()?.required_authority(),
        Some(RiskDecision::FlattenRequired)
    );
    Ok(())
}

fn imbalanced_with_terminal_initial_children() -> Result<PreparedBasket, Box<dyn Error>> {
    let mut prepared = prepare_increase()?;
    let event = fill(&prepared, 0, "initial-long-fill", 10, 1)?;
    handle(event, &mut prepared)?;
    handle(reject(child_id(&prepared, 1), 3), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    Ok(prepared)
}

#[test]
fn residual_hedge_strictly_reduces_residual() -> TestResult {
    let mut prepared = imbalanced_with_terminal_initial_children()?;
    let before = prepared.coordinator.residual().value().abs();
    let batch = prepared
        .coordinator
        .prepare_residual_recovery(UnixNanos(PREFLIGHT_AT.0 + 5_000_000), &mut prepared.journal)?
        .ok_or("recovery command missing")?;
    assert_eq!(batch.commands.len(), 1);
    assert_eq!(prepared.coordinator.state(), BasketState::Hedging);
    let recovery = prepared
        .coordinator
        .recovery_intents()
        .last()
        .ok_or("recovery evidence missing")?;
    assert_eq!(recovery.purpose(), ExecutionIntentPurpose::ResidualHedge);
    let projected = match recovery.safety_evidence() {
        Some(riftbot::domain::execution_intent::ExecutionSafetyEvidence::ResidualHedge(
            evidence,
        )) => evidence.projected_residual().value().abs(),
        _ => return Err("typed residual evidence missing".into()),
    };
    assert!(projected < before);
    Ok(())
}

#[test]
fn residual_hedge_never_exceeds_unmatched_filled_exposure() -> TestResult {
    let mut prepared = imbalanced_with_terminal_initial_children()?;
    let unmatched = prepared.coordinator.residual().value().abs();
    prepared
        .coordinator
        .prepare_residual_recovery(UnixNanos(PREFLIGHT_AT.0 + 5_000_000), &mut prepared.journal)?;
    let recovery = prepared
        .coordinator
        .recovery_intents()
        .last()
        .ok_or("recovery intent missing")?;
    assert!(recovery.authorized_matched_notional_per_leg().value() <= unmatched);
    assert!(recovery.legs()[0].target_notional.value() <= unmatched);
    Ok(())
}

#[test]
fn bounded_recovery_completes_after_actual_hedge_fill() -> TestResult {
    let mut prepared = imbalanced_with_terminal_initial_children()?;
    prepared
        .coordinator
        .prepare_residual_recovery(UnixNanos(PREFLIGHT_AT.0 + 5_000_000), &mut prepared.journal)?;
    let recovery_fill = fill(&prepared, 2, "recovery-full-fill", 10, 7)?;
    handle(recovery_fill, &mut prepared)?;
    assert_eq!(prepared.coordinator.residual().value(), Decimal::ZERO);
    assert_eq!(prepared.coordinator.state(), BasketState::Complete);
    Ok(())
}

#[test]
fn rejected_recovery_uses_only_the_next_bounded_generation() -> TestResult {
    let mut prepared = imbalanced_with_terminal_initial_children()?;
    prepared
        .coordinator
        .prepare_residual_recovery(UnixNanos(PREFLIGHT_AT.0 + 5_000_000), &mut prepared.journal)?;
    let first_recovery_id = child_id(&prepared, 2);
    handle(reject(first_recovery_id.clone(), 7), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    prepared
        .coordinator
        .prepare_residual_recovery(UnixNanos(PREFLIGHT_AT.0 + 9_000_000), &mut prepared.journal)?;
    assert_eq!(prepared.coordinator.children().len(), 4);
    assert_eq!(prepared.coordinator.children()[3].generation, 2);
    assert_ne!(
        prepared.coordinator.children()[3].client_order_id,
        first_recovery_id
    );
    Ok(())
}

#[test]
fn recovery_exhaustion_enters_failed_safe_with_restrictive_authority() -> TestResult {
    let mut prepared = imbalanced_with_terminal_initial_children()?;
    for attempt in 0..2 {
        prepared.coordinator.prepare_residual_recovery(
            UnixNanos(PREFLIGHT_AT.0 + (5 + attempt * 10) * 1_000_000),
            &mut prepared.journal,
        )?;
        let recovery_index = prepared.coordinator.children().len() - 1;
        let partial = fill(
            &prepared,
            recovery_index,
            &format!("partial-recovery-{attempt}"),
            5,
            7 + attempt * 10,
        )?;
        handle(partial, &mut prepared)?;
        prepared.coordinator.prepare_abort_cancels(
            UnixNanos(PREFLIGHT_AT.0 + (9 + attempt * 10) * 1_000_000),
            &mut prepared.journal,
        )?;
        handle(
            cancel_confirmed(child_id(&prepared, recovery_index), 10 + attempt * 10),
            &mut prepared,
        )?;
        assert_eq!(prepared.coordinator.state(), BasketState::Imbalanced);
    }
    assert!(
        prepared
            .coordinator
            .prepare_residual_recovery(
                UnixNanos(PREFLIGHT_AT.0 + 30_000_000),
                &mut prepared.journal,
            )?
            .is_none()
    );
    assert_eq!(prepared.coordinator.state(), BasketState::FailedSafe);
    assert_eq!(
        prepared.coordinator.snapshot()?.required_authority(),
        Some(RiskDecision::FlattenRequired)
    );
    Ok(())
}

#[test]
fn expired_intent_is_rejected_before_submit() -> TestResult {
    let mut coordinator =
        ExecutionBasketCoordinator::new(normal_intent(ExecutionIntentPurpose::ReduceRisk)?, 2)?;
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    assert_eq!(
        coordinator.prepare_reduction(EXPIRY, &mut journal, &mut reservations),
        Err(CoordinatorError::InvalidExecutionTime)
    );
    assert!(coordinator.children().is_empty());
    assert!(journal.records().is_empty());
    Ok(())
}

#[test]
fn stale_p5_authorization_is_rejected_for_increase() -> TestResult {
    let mut coordinator =
        ExecutionBasketCoordinator::new(normal_intent(ExecutionIntentPurpose::IncreaseRisk)?, 2)?;
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    let stale_at = UnixNanos(EVALUATED_AT.0 + 1_501_000_000);
    assert_eq!(
        prepare_increase_at(
            &mut coordinator,
            stale_at,
            Decimal::from(100),
            Decimal::from(101),
            &mut journal,
            &mut reservations,
        ),
        Err(CoordinatorError::StaleAuthorization)
    );
    assert!(coordinator.children().is_empty());
    Ok(())
}

#[test]
fn regressive_preflight_time_fails_closed() -> TestResult {
    let mut coordinator =
        ExecutionBasketCoordinator::new(normal_intent(ExecutionIntentPurpose::IncreaseRisk)?, 2)?;
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    let regressive = UnixNanos(CREATED_AT.0 - 1);
    assert_eq!(
        prepare_increase_at(
            &mut coordinator,
            regressive,
            Decimal::from(100),
            Decimal::from(101),
            &mut journal,
            &mut reservations,
        ),
        Err(CoordinatorError::InvalidExecutionTime)
    );
    Ok(())
}

#[test]
fn p5_authorized_size_cannot_be_enlarged_by_serde() -> TestResult {
    let intent = normal_intent(ExecutionIntentPurpose::IncreaseRisk)?;
    let mut enlarged = serde_json::to_value(&intent)?;
    enlarged["authorized_matched_notional_per_leg"] = json!("51");
    assert!(serde_json::from_value::<ExecutionIntent>(enlarged).is_err());

    let mut enlarged_leg = serde_json::to_value(&intent)?;
    enlarged_leg["legs"][0]["target_qty"] = json!("0.51");
    enlarged_leg["legs"][0]["target_notional"] = json!("51");
    assert!(serde_json::from_value::<ExecutionIntent>(enlarged_leg).is_err());
    Ok(())
}

#[test]
fn restrictive_risk_assessment_cannot_create_increase_intent() -> TestResult {
    let intent = normal_intent(ExecutionIntentPurpose::IncreaseRisk)?;
    let mut impossible = serde_json::to_value(&intent)?;
    impossible["risk_context"]["kill_state"] = json!("reduce_only");
    impossible["risk_context"]["assessment"]["kill_state"] = json!("reduce_only");
    assert!(serde_json::from_value::<ExecutionIntent>(impossible).is_err());
    Ok(())
}

#[test]
fn reduction_works_without_positive_entry_edge_under_restrictive_state() -> TestResult {
    let prepared = prepare_reduction()?;
    assert_eq!(prepared.batch.commands.len(), 2);
    assert!(
        prepared
            .coordinator
            .intent()
            .legs()
            .iter()
            .all(|leg| { leg.reduce_only && matches!(leg.side, OrderSide::Sell | OrderSide::Buy) })
    );
    assert_eq!(prepared.coordinator.state(), BasketState::Submitting);
    assert!(
        prepared
            .reservations
            .active(prepared.coordinator.intent().pair_id())
            .is_none()
    );
    Ok(())
}

#[test]
fn flatten_for_reversal_is_a_reduce_risk_intent_with_source_preserved() -> TestResult {
    let config = config()?;
    let fingerprint = measurement_config_fingerprint(&config)?;
    let mut source = inventory(ExecutionIntentPurpose::ReduceRisk, fingerprint.as_str())?;
    let (decision_id, pair_id, symbol, long_venue, short_venue, _, _) = ids()?;
    source.action = InventoryAction::FlattenForReversal;
    source.required_change_notional_per_leg = Money::new(Decimal::from(-200));
    source.proposed_change_notional_per_leg = Notional::new(Decimal::from(200))?;
    source.selected_target = Some(TargetInventory::new(TargetInventoryParams {
        symbol,
        pair_id,
        target_fraction: TargetFraction::new(Decimal::new(8, 1))?,
        target_notional: Notional::new(Decimal::from(400))?,
        direction: TargetDirection::LongShort {
            long_venue: short_venue,
            short_venue: long_venue,
        },
        reason: "opposite route selected after flatten".to_owned(),
        model_version: ModelVersion::try_from("p4-grid-inventory-v1")?,
        decision_id,
    })?);
    let mut requests = leg_requests(Decimal::new(1, 2), Decimal::from(102), Decimal::from(99))?;
    requests[0].price_guard = PriceGuard::MinimumSell(Price::new(Decimal::from(99))?);
    requests[1].price_guard = PriceGuard::MaximumBuy(Price::new(Decimal::from(102))?);
    let intent = ExecutionIntent::new_normal(NormalExecutionIntentRequest {
        intent_id: IntentId::try_from("p6-flatten-for-reversal")?,
        purpose: ExecutionIntentPurpose::ReduceRisk,
        source_inventory: source,
        risk_context: risk_context(
            RiskInputAction::FlattenForReversal,
            RiskDecision::FlattenRequired,
            Regime::Normal,
            KillState::Flatten,
            Decimal::from(200),
            Decimal::from(200),
            Decimal::from(200),
        )?,
        leg_requests: requests,
        created_at: CREATED_AT,
        expiry: EXPIRY,
        max_residual_delta: config.execution.max_residual_delta,
        max_slippage_bps: config.execution.max_slippage_bps,
    })?;
    assert_eq!(intent.purpose(), ExecutionIntentPurpose::ReduceRisk);
    assert_eq!(
        intent
            .source_inventory()
            .ok_or("source P4 decision missing")?
            .action,
        InventoryAction::FlattenForReversal
    );
    assert!(intent.legs().iter().all(|leg| leg.reduce_only));
    Ok(())
}

#[test]
fn quantity_rounding_never_exceeds_risk_or_measured_bounds() -> TestResult {
    let intent = build_normal_intent(
        ExecutionIntentPurpose::IncreaseRisk,
        leg_requests(Decimal::new(3, 2), Decimal::from(101), Decimal::from(100))?,
        CREATED_AT,
        EXPIRY,
    )?;
    for leg in intent.legs() {
        assert!(leg.target_qty.value() <= Decimal::new(5, 1));
        assert!(leg.target_notional.value() <= Decimal::from(50));
        assert_eq!(leg.target_qty.value() % leg.lot_size.value(), Decimal::ZERO);
    }
    assert_eq!(intent.legs()[0].target_qty.value(), Decimal::new(48, 2));
    assert_eq!(intent.legs()[1].target_qty.value(), Decimal::new(48, 2));
    Ok(())
}

#[test]
fn quantity_rounding_rejects_when_no_safe_lot_fits() -> TestResult {
    let result = build_normal_intent(
        ExecutionIntentPurpose::IncreaseRisk,
        leg_requests(Decimal::from(1), Decimal::from(101), Decimal::from(100))?,
        CREATED_AT,
        EXPIRY,
    );
    assert!(matches!(
        result,
        Err(error) if error.downcast_ref::<ExecutionIntentError>()
            == Some(&ExecutionIntentError::NoExecutableQuantity)
    ));
    Ok(())
}

#[test]
fn price_guard_side_mismatch_is_rejected() -> TestResult {
    let mut requests = leg_requests(Decimal::new(1, 2), Decimal::from(101), Decimal::from(100))?;
    requests[1].price_guard = PriceGuard::MaximumBuy(Price::new(Decimal::from(102))?);
    let result = build_normal_intent(
        ExecutionIntentPurpose::IncreaseRisk,
        requests,
        CREATED_AT,
        EXPIRY,
    );
    assert!(matches!(
        result,
        Err(error) if error.downcast_ref::<ExecutionIntentError>()
            == Some(&ExecutionIntentError::PriceGuardSideMismatch)
    ));
    Ok(())
}

#[test]
fn pre_submit_price_slippage_violation_aborts_both_legs() -> TestResult {
    let mut coordinator =
        ExecutionBasketCoordinator::new(normal_intent(ExecutionIntentPurpose::IncreaseRisk)?, 2)?;
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    assert_eq!(
        prepare_increase_at(
            &mut coordinator,
            PREFLIGHT_AT,
            Decimal::new(1001, 1),
            Decimal::new(1012, 1),
            &mut journal,
            &mut reservations,
        ),
        Err(CoordinatorError::PriceSafetyViolation)
    );
    assert!(coordinator.children().is_empty());
    assert!(
        reservations
            .active(coordinator.intent().pair_id())
            .is_none()
    );
    Ok(())
}

#[test]
fn pair_concurrent_increase_is_rejected_while_reserved() -> TestResult {
    let first = prepare_increase()?;
    assert!(
        first
            .reservations
            .active(first.coordinator.intent().pair_id())
            .is_some()
    );
    let mut second =
        ExecutionBasketCoordinator::new(normal_intent(ExecutionIntentPurpose::IncreaseRisk)?, 2)?;
    let mut second_journal = InMemoryExecutionJournal::default();
    let mut shared_reservations = first.reservations.clone();
    assert_eq!(
        prepare_increase_at(
            &mut second,
            PREFLIGHT_AT,
            Decimal::from(100),
            Decimal::from(101),
            &mut second_journal,
            &mut shared_reservations,
        ),
        Err(CoordinatorError::PairIncreaseAlreadyReserved)
    );
    assert!(second.children().is_empty());
    assert!(second_journal.records().is_empty());
    Ok(())
}

#[test]
fn active_reservation_is_visible_in_effective_inventory() -> TestResult {
    let prepared = prepare_increase()?;
    let (_, pair_id, symbol, long_venue, short_venue, _, _) = ids()?;
    let empty = EffectiveInventory::new(symbol, pair_id.clone(), Vec::new())?;
    let effective = prepared
        .reservations
        .effective_inventory_with_reservations(&empty)?;
    let projected = effective.project(&long_venue, &short_venue)?;
    assert_eq!(
        projected.reserved_notional_per_leg,
        prepared
            .reservations
            .reserved_notional_per_leg(&pair_id)
            .ok_or("reservation missing")?
    );
    assert_eq!(projected.total_notional_per_leg.value(), Decimal::from(50));
    Ok(())
}

#[test]
fn journal_failure_prevents_unjournaled_side_effect_identity() -> TestResult {
    let mut coordinator =
        ExecutionBasketCoordinator::new(normal_intent(ExecutionIntentPurpose::IncreaseRisk)?, 2)?;
    let mut journal = InMemoryExecutionJournal::default();
    journal.fail_next_append();
    let mut reservations = PairReservationBook::default();
    assert!(matches!(
        prepare_increase_at(
            &mut coordinator,
            PREFLIGHT_AT,
            Decimal::from(100),
            Decimal::from(101),
            &mut journal,
            &mut reservations,
        ),
        Err(CoordinatorError::Journal(_))
    ));
    assert!(coordinator.children().is_empty());
    assert!(
        reservations
            .active(coordinator.intent().pair_id())
            .is_none()
    );
    Ok(())
}

#[test]
fn deterministic_same_command_event_sequence_produces_identical_state() -> TestResult {
    let mut left = prepare_increase()?;
    let mut right = prepare_increase()?;
    assert_eq!(left.batch, right.batch);
    assert_eq!(left.journal, right.journal);

    let events = vec![
        ack(child_id(&left, 0), 1)?,
        ack(child_id(&left, 1), 2)?,
        fill(&left, 0, "deterministic-long", 10, 3)?,
        fill(&left, 1, "deterministic-short", 10, 4)?,
    ];
    for event in events {
        handle(event.clone(), &mut left)?;
        handle(event, &mut right)?;
    }
    assert_eq!(left.coordinator.snapshot()?, right.coordinator.snapshot()?);
    assert_eq!(left.journal, right.journal);
    assert_eq!(left.reservations, right.reservations);
    Ok(())
}

#[test]
fn serde_cannot_create_impossible_execution_intent_or_state() -> TestResult {
    let intent = normal_intent(ExecutionIntentPurpose::IncreaseRisk)?;
    let mut wrong_identity = serde_json::to_value(&intent)?;
    wrong_identity["decision_id"] = json!("different-decision");
    assert!(serde_json::from_value::<ExecutionIntent>(wrong_identity).is_err());

    let prepared = prepare_increase()?;
    let mut impossible_state = serde_json::to_value(prepared.coordinator.snapshot()?)?;
    impossible_state["state"] = json!("complete");
    impossible_state["terminal_reason"] = json!("forged completion");
    assert!(
        serde_json::from_value::<riftbot::execution::state_machine::ExecutionBasketSnapshot>(
            impossible_state
        )
        .is_err()
    );

    let mut impossible_child_identity = serde_json::to_value(prepared.coordinator.snapshot()?)?;
    impossible_child_identity["children"][0]["client_order_id"] = json!("forged-child");
    assert!(
        serde_json::from_value::<riftbot::execution::state_machine::ExecutionBasketSnapshot>(
            impossible_child_identity
        )
        .is_err()
    );

    let mut recovery_basket = imbalanced_with_terminal_initial_children()?;
    recovery_basket.coordinator.prepare_residual_recovery(
        UnixNanos(PREFLIGHT_AT.0 + 5_000_000),
        &mut recovery_basket.journal,
    )?;
    let recovery = recovery_basket
        .coordinator
        .recovery_intents()
        .last()
        .ok_or("recovery intent missing")?;
    let mut enlarged_recovery = serde_json::to_value(recovery)?;
    enlarged_recovery["authorized_matched_notional_per_leg"] = json!("60");
    enlarged_recovery["legs"][0]["target_qty"] = json!("0.60");
    enlarged_recovery["legs"][0]["target_notional"] = json!("60");
    enlarged_recovery["target_net_delta"] = json!("-60");
    assert!(serde_json::from_value::<ExecutionIntent>(enlarged_recovery).is_err());

    let emergency = emergency_intent(Decimal::from(50))?;
    let mut crossed_emergency = serde_json::to_value(emergency)?;
    crossed_emergency["authorized_matched_notional_per_leg"] = json!("60");
    crossed_emergency["legs"][0]["target_qty"] = json!("0.60");
    crossed_emergency["legs"][0]["target_notional"] = json!("60");
    crossed_emergency["target_net_delta"] = json!("-60");
    assert!(serde_json::from_value::<ExecutionIntent>(crossed_emergency).is_err());
    Ok(())
}

fn flatten_context() -> Result<RiskContext, Box<dyn Error>> {
    risk_context(
        RiskInputAction::ReduceRisk,
        RiskDecision::FlattenRequired,
        Regime::Normal,
        KillState::Flatten,
        Decimal::from(200),
        Decimal::from(100),
        Decimal::from(100),
    )
}

fn emergency_intent(maximum: Decimal) -> Result<ExecutionIntent, ExecutionIntentError> {
    let (decision_id, pair_id, symbol, long_venue, _, long_instrument, _) =
        ids().map_err(|_| ExecutionIntentError::InvalidEmergencyEvidence)?;
    let requests = leg_requests(Decimal::new(1, 2), Decimal::from(101), Decimal::from(100))
        .map_err(|_| ExecutionIntentError::InvalidEmergencyEvidence)?;
    let [mut flatten_leg, _] = requests;
    flatten_leg.venue = long_venue;
    flatten_leg.instrument = long_instrument.clone();
    flatten_leg.metadata.instrument = long_instrument;
    flatten_leg.price_guard = PriceGuard::MinimumSell(
        Price::new(Decimal::from(99))
            .map_err(|_| ExecutionIntentError::InvalidEmergencyEvidence)?,
    );
    ExecutionIntent::new_emergency_flatten(EmergencyFlattenIntentRequest {
        intent_id: IntentId::try_from("p6-emergency-flatten")
            .map_err(|_| ExecutionIntentError::InvalidEmergencyEvidence)?,
        decision_id,
        pair_id,
        symbol,
        risk_context: flatten_context()
            .map_err(|_| ExecutionIntentError::InvalidEmergencyEvidence)?,
        position_evidence: EmergencyPositionEvidence {
            evidence_id: EvidenceId::try_from("known-position")
                .map_err(|_| ExecutionIntentError::InvalidEmergencyEvidence)?,
            current_delta: Delta::new(Decimal::from(50)),
            observed_at: CREATED_AT,
        },
        flatten_leg,
        maximum_flatten_notional: Notional::new(maximum)
            .map_err(|_| ExecutionIntentError::EmergencyDoesNotReduce)?,
        created_at: CREATED_AT,
        expiry: EXPIRY,
        max_residual_delta: Delta::new(Decimal::from(10)),
        max_slippage_bps: Bps::new(Decimal::from(20)),
    })
}

#[test]
fn emergency_flatten_is_evidence_bounded_and_can_reach_zero() -> TestResult {
    let intent = emergency_intent(Decimal::from(50))?;
    assert_eq!(intent.purpose(), ExecutionIntentPurpose::EmergencyFlatten);
    assert!(intent.legs()[0].reduce_only);
    assert_eq!(intent.target_net_delta().value(), Decimal::from(-50));
    Ok(())
}

#[test]
fn emergency_flatten_cannot_cross_through_zero() -> TestResult {
    assert_eq!(
        emergency_intent(Decimal::from(60)),
        Err(ExecutionIntentError::EmergencyDoesNotReduce)
    );
    Ok(())
}

fn prepare_emergency() -> Result<PreparedBasket, Box<dyn Error>> {
    let mut coordinator =
        ExecutionBasketCoordinator::new_emergency(emergency_intent(Decimal::from(50))?, 2)?;
    let mut journal = InMemoryExecutionJournal::default();
    let mut reservations = PairReservationBook::default();
    let batch =
        coordinator.prepare_emergency_flatten(PREFLIGHT_AT, &mut journal, &mut reservations)?;
    Ok(PreparedBasket {
        coordinator,
        journal,
        reservations,
        batch,
    })
}

#[test]
fn emergency_flatten_tracks_known_starting_exposure_and_actual_fill() -> TestResult {
    let mut prepared = prepare_emergency()?;
    assert_eq!(prepared.batch.commands.len(), 1);
    assert_eq!(
        prepared.coordinator.snapshot()?.starting_residual().value(),
        Decimal::from(50)
    );
    let actual_fill = fill(&prepared, 0, "emergency-full-fill", 10, 1)?;
    handle(actual_fill, &mut prepared)?;
    assert_eq!(prepared.coordinator.residual().value(), Decimal::ZERO);
    assert_eq!(prepared.coordinator.state(), BasketState::Complete);
    assert!(
        prepared
            .reservations
            .active(prepared.coordinator.intent().pair_id())
            .is_none()
    );
    Ok(())
}

#[test]
fn rejected_emergency_flatten_fails_safe_instead_of_claiming_completion() -> TestResult {
    let mut prepared = prepare_emergency()?;
    handle(reject(child_id(&prepared, 0), 1), &mut prepared)?;
    assert_eq!(prepared.coordinator.state(), BasketState::FailedSafe);
    assert_eq!(prepared.coordinator.residual().value(), Decimal::from(50));
    assert_eq!(
        prepared.coordinator.snapshot()?.required_authority(),
        Some(RiskDecision::FlattenRequired)
    );
    Ok(())
}
