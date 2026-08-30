use std::error::Error;

use riftbot::{
    config::{AppConfig, parse_toml, risk_config_fingerprint},
    domain::{
        ids::{DecisionId, ModelVersion, PairId, Symbol, VenueId},
        inventory::{EffectiveActual, TargetDirection, TargetInventory, TargetInventoryParams},
        numeric::{BaseQty, Delta, Money, Notional, TargetFraction, UnixNanos},
        risk::{
            ExposureComponents, GlobalDeltaComponents, HealthStatus, KillState, Regime,
            RiskDecision, RiskExposureSnapshot, RiskHealthSnapshot, RiskReasonCode, VenueExposure,
            VenueRiskHealth,
        },
    },
    risk::manager::{KillStateMachine, RiskEvaluationInput, RiskManager},
    strategy::inventory_manager::{IncreaseSizeBasis, InventoryAction, InventoryDecision},
};
use rust_decimal::Decimal;

const EXAMPLE: &str = include_str!("../config/example.toml");
const NOW: UnixNanos = UnixNanos(10_000_000_000);

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture {
    config: AppConfig,
    inventory: InventoryDecision,
    exposure: Option<RiskExposureSnapshot>,
    health: Option<RiskHealthSnapshot>,
    session_pnl: Option<Money>,
    regime: Regime,
    kill_state: KillState,
    evaluated_at: UnixNanos,
}

impl Fixture {
    fn increase() -> Result<Self, Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let inventory = increase_inventory(UnixNanos(NOW.0 - 100_000_000))?;
        Self::from_inventory(config, inventory)
    }

    fn reduction() -> Result<Self, Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let inventory = reduction_inventory()?;
        Self::from_inventory(config, inventory)
    }

    fn no_change() -> Result<Self, Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let mut inventory = reduction_inventory()?;
        inventory.action = InventoryAction::NoChange;
        inventory.required_change_notional_per_leg = Money::new(Decimal::ZERO);
        inventory.proposed_change_notional_per_leg = Notional::new(Decimal::ZERO)?;
        Self::from_inventory(config, inventory)
    }

    fn from_inventory(
        config: AppConfig,
        inventory: InventoryDecision,
    ) -> Result<Self, Box<dyn Error>> {
        let actual = inventory
            .effective_actual
            .as_ref()
            .ok_or("fixture inventory must carry effective actual")?;
        let (long_venue, short_venue) = direction_venues(&actual.direction)?;
        let components = ExposureComponents {
            actual: actual.actual_notional_per_leg,
            reserved: actual.reserved_notional_per_leg,
            pending: actual.pending_notional_per_leg,
        };
        let exposure = RiskExposureSnapshot {
            pair_id: inventory.pair_id.clone(),
            symbol: inventory.symbol.clone(),
            pair_per_leg: components,
            venues: vec![
                VenueExposure {
                    venue_id: long_venue.clone(),
                    exposure: components,
                },
                VenueExposure {
                    venue_id: short_venue.clone(),
                    exposure: components,
                },
            ],
            global_delta: GlobalDeltaComponents {
                actual: Delta::new(Decimal::ZERO),
                reserved: Delta::new(Decimal::ZERO),
                pending: Delta::new(Decimal::ZERO),
            },
        };
        let healthy = |venue_id| VenueRiskHealth {
            venue_id,
            market_feed: HealthStatus::Healthy,
            connectivity: HealthStatus::Healthy,
            account_private_stream: HealthStatus::Healthy,
        };
        Ok(Self {
            config,
            inventory,
            exposure: Some(exposure),
            health: Some(RiskHealthSnapshot {
                venues: vec![healthy(long_venue), healthy(short_venue)],
                reconciliation: HealthStatus::Healthy,
                state_freshness: HealthStatus::Healthy,
                latency: HealthStatus::Healthy,
                outstanding_operations: 0,
                unknown_operations: 0,
                outstanding_exposure_included: true,
            }),
            session_pnl: Some(Money::new(Decimal::ZERO)),
            regime: Regime::Normal,
            kill_state: KillState::Ready,
            evaluated_at: NOW,
        })
    }

    fn assess(&self) -> Result<riftbot::domain::risk::RiskAssessment, Box<dyn Error>> {
        let fingerprint = risk_config_fingerprint(&self.config)?;
        let manager = RiskManager::new(&self.config.risk, &fingerprint)?;
        Ok(manager.assess(RiskEvaluationInput {
            inventory: &self.inventory,
            regime: self.regime,
            kill_state: self.kill_state,
            evaluated_at: self.evaluated_at,
            exposure: self.exposure.as_ref(),
            health: self.health.as_ref(),
            session_pnl: self.session_pnl,
        })?)
    }
}

fn increase_inventory(observed_at: UnixNanos) -> Result<InventoryDecision, Box<dyn Error>> {
    let decision_id = DecisionId::try_from("p5-increase-test")?;
    let pair_id = PairId::try_from("sndk_entropy_lighter")?;
    let symbol = Symbol::try_from("SNDK")?;
    let entropy = VenueId::try_from("entropy")?;
    let lighter = VenueId::try_from("lighter")?;
    let direction = TargetDirection::LongShort {
        long_venue: entropy,
        short_venue: lighter,
    };
    Ok(InventoryDecision {
        decision_id: decision_id.clone(),
        pair_id: pair_id.clone(),
        symbol: symbol.clone(),
        action: InventoryAction::IncreaseRisk,
        selected_target: Some(TargetInventory::new(TargetInventoryParams {
            symbol,
            pair_id,
            target_fraction: TargetFraction::new(Decimal::new(8, 1))?,
            target_notional: Notional::new(Decimal::from(400))?,
            direction: direction.clone(),
            reason: "p5 increase fixture".to_owned(),
            model_version: ModelVersion::try_from("p4-grid-inventory-v1")?,
            decision_id,
        })?),
        effective_actual: Some(EffectiveActual {
            direction,
            actual_notional_per_leg: Notional::new(Decimal::from(200))?,
            reserved_notional_per_leg: Notional::new(Decimal::from(50))?,
            pending_notional_per_leg: Notional::new(Decimal::from(25))?,
            total_notional_per_leg: Notional::new(Decimal::from(275))?,
        }),
        required_change_notional_per_leg: Money::new(Decimal::from(125)),
        proposed_change_notional_per_leg: Notional::new(Decimal::from(50))?,
        increase_size_basis: Some(IncreaseSizeBasis {
            requested_base_quantity: BaseQty::new(Decimal::new(5, 1))?,
            long_measured_notional: Notional::new(Decimal::from(50))?,
            short_measured_notional: Notional::new(Decimal::new(505, 1))?,
            measured_matched_notional_cap: Notional::new(Decimal::from(50))?,
            observed_at,
            measurement_model_version: ModelVersion::try_from("p3-measurement-v1")?,
            measurement_config_fingerprint: "measurement-config-sha256".to_owned(),
        }),
        block_reason: None,
    })
}

fn reduction_inventory() -> Result<InventoryDecision, Box<dyn Error>> {
    let decision_id = DecisionId::try_from("p5-reduction-test")?;
    let pair_id = PairId::try_from("sndk_entropy_lighter")?;
    let symbol = Symbol::try_from("SNDK")?;
    let direction = TargetDirection::LongShort {
        long_venue: VenueId::try_from("entropy")?,
        short_venue: VenueId::try_from("lighter")?,
    };
    Ok(InventoryDecision {
        decision_id: decision_id.clone(),
        pair_id: pair_id.clone(),
        symbol: symbol.clone(),
        action: InventoryAction::ReduceRisk,
        selected_target: Some(TargetInventory::new(TargetInventoryParams {
            symbol,
            pair_id,
            target_fraction: TargetFraction::new(Decimal::new(2, 1))?,
            target_notional: Notional::new(Decimal::from(100))?,
            direction: direction.clone(),
            reason: "p5 reduction fixture".to_owned(),
            model_version: ModelVersion::try_from("p4-grid-inventory-v1")?,
            decision_id,
        })?),
        effective_actual: Some(EffectiveActual {
            direction,
            actual_notional_per_leg: Notional::new(Decimal::from(125))?,
            reserved_notional_per_leg: Notional::new(Decimal::from(50))?,
            pending_notional_per_leg: Notional::new(Decimal::from(25))?,
            total_notional_per_leg: Notional::new(Decimal::from(200))?,
        }),
        required_change_notional_per_leg: Money::new(Decimal::from(-100)),
        proposed_change_notional_per_leg: Notional::new(Decimal::from(100))?,
        increase_size_basis: None,
        block_reason: None,
    })
}

fn direction_venues(direction: &TargetDirection) -> Result<(VenueId, VenueId), Box<dyn Error>> {
    match direction {
        TargetDirection::LongShort {
            long_venue,
            short_venue,
        } => Ok((long_venue.clone(), short_venue.clone())),
        TargetDirection::Flat => Err("fixture requires an oriented route".into()),
    }
}

fn has_reason(assessment: &riftbot::domain::risk::RiskAssessment, reason: RiskReasonCode) -> bool {
    assessment.reason_codes().contains(&reason)
}

fn set_effective_exposure(
    fixture: &mut Fixture,
    actual: Decimal,
    reserved: Decimal,
    pending: Decimal,
) -> Result<(), Box<dyn Error>> {
    let components = ExposureComponents {
        actual: Notional::new(actual)?,
        reserved: Notional::new(reserved)?,
        pending: Notional::new(pending)?,
    };
    let exposure = fixture
        .exposure
        .as_mut()
        .ok_or("fixture exposure is missing")?;
    exposure.pair_per_leg = components;
    for venue in &mut exposure.venues {
        venue.exposure = components;
    }
    let effective = fixture
        .inventory
        .effective_actual
        .as_mut()
        .ok_or("fixture effective actual is missing")?;
    effective.actual_notional_per_leg = components.actual;
    effective.reserved_notional_per_leg = components.reserved;
    effective.pending_notional_per_leg = components.pending;
    effective.total_notional_per_leg = Notional::new(
        actual
            .checked_add(reserved)
            .and_then(|value| value.checked_add(pending))
            .ok_or("fixture arithmetic overflow")?,
    )?;
    Ok(())
}

#[test]
fn normal_valid_increase_is_approved_and_auditable() -> TestResult {
    let fixture = Fixture::increase()?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Approve);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::from(50)
    );
    assert_eq!(assessment.decision_id(), &fixture.inventory.decision_id);
    assert_eq!(assessment.evaluated_at(), NOW);
    assert_eq!(
        assessment.input_action(),
        riftbot::domain::risk::RiskInputAction::IncreaseRisk
    );
    assert_eq!(
        assessment.requested_change_notional_per_leg().value(),
        Decimal::from(125)
    );
    assert_eq!(
        assessment.proposed_change_notional_per_leg().value(),
        Decimal::from(50)
    );
    assert_eq!(assessment.regime(), Regime::Normal);
    assert_eq!(assessment.kill_state(), KillState::Ready);
    assert_eq!(assessment.measurement_age_ms().map(|age| age.0), Some(100));
    assert_eq!(
        assessment
            .measurement_safe_matched_notional_cap()
            .map(Notional::value),
        Some(Decimal::from(50))
    );
    assert!(has_reason(&assessment, RiskReasonCode::Approved));
    let audit = assessment.exposure().ok_or("missing exposure audit")?;
    assert_eq!(
        audit.pair_candidate_projected_notional_per_leg.value(),
        Decimal::from(325)
    );
    assert_eq!(
        audit.pair_authorized_projected_notional_per_leg.value(),
        Decimal::from(325)
    );
    assert_eq!(assessment.config_fingerprint().len(), 64);
    assert!(!assessment.explanation().is_empty());
    Ok(())
}

#[test]
fn risk_never_enlarges_p4_proposal_and_degraded_policy_clips() -> TestResult {
    let normal = Fixture::increase()?.assess()?;
    assert!(
        normal.authorized_change_notional_per_leg().value()
            <= normal.proposed_change_notional_per_leg().value()
    );

    let mut degraded = Fixture::increase()?;
    degraded.regime = Regime::Degraded;
    let assessment = degraded.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Approve);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::from(25)
    );
    assert!(has_reason(&assessment, RiskReasonCode::RegimeDegradedClip));
    Ok(())
}

#[test]
fn p4_change_above_p3_safe_cap_fails_closed() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let basis = fixture
        .inventory
        .increase_size_basis
        .as_mut()
        .ok_or("missing basis")?;
    basis.measured_matched_notional_cap = Notional::new(Decimal::from(49))?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(
        &assessment,
        RiskReasonCode::MeasurementSizeCapInvalid
    ));
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    Ok(())
}

#[test]
fn stale_measurement_is_denied() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let basis = fixture
        .inventory
        .increase_size_basis
        .as_mut()
        .ok_or("missing basis")?;
    basis.observed_at =
        UnixNanos(NOW.0 - fixture.config.risk.max_measurement_age_ms.0 * 1_000_000 - 1);
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    assert!(has_reason(&assessment, RiskReasonCode::MeasurementStale));
    Ok(())
}

#[test]
fn future_measurement_timestamp_is_denied() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let basis = fixture
        .inventory
        .increase_size_basis
        .as_mut()
        .ok_or("missing basis")?;
    basis.observed_at = UnixNanos(NOW.0 + 1);
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(
        &assessment,
        RiskReasonCode::MeasurementTimestampFuture
    ));
    Ok(())
}

#[test]
fn missing_increase_size_basis_is_denied() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.inventory.increase_size_basis = None;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(
        &assessment,
        RiskReasonCode::MeasurementBasisMissing
    ));
    Ok(())
}

#[test]
fn reduction_allows_missing_or_stale_entry_economics() -> TestResult {
    let mut missing = Fixture::reduction()?;
    missing.health = None;
    missing.session_pnl = None;
    let missing_assessment = missing.assess()?;
    assert_eq!(missing_assessment.decision(), RiskDecision::Approve);
    assert_eq!(
        missing_assessment
            .authorized_change_notional_per_leg()
            .value(),
        Decimal::from(100)
    );

    let mut stale = Fixture::reduction()?;
    stale.inventory.increase_size_basis = Some(IncreaseSizeBasis {
        requested_base_quantity: BaseQty::new(Decimal::ONE)?,
        long_measured_notional: Notional::new(Decimal::from(100))?,
        short_measured_notional: Notional::new(Decimal::from(100))?,
        measured_matched_notional_cap: Notional::new(Decimal::from(100))?,
        observed_at: UnixNanos(0),
        measurement_model_version: ModelVersion::try_from("stale-entry-economics")?,
        measurement_config_fingerprint: "stale".to_owned(),
    });
    let stale_assessment = stale.assess()?;
    assert_eq!(stale_assessment.decision(), RiskDecision::Approve);
    assert_eq!(
        stale_assessment
            .authorized_change_notional_per_leg()
            .value(),
        Decimal::from(100)
    );
    Ok(())
}

#[test]
fn pause_new_and_reduce_only_block_increase() -> TestResult {
    let mut pause = Fixture::increase()?;
    pause.kill_state = KillState::PauseNew;
    let pause_assessment = pause.assess()?;
    assert_eq!(pause_assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&pause_assessment, RiskReasonCode::KillPauseNew));

    let mut reduce_only = Fixture::increase()?;
    reduce_only.kill_state = KillState::ReduceOnly;
    let reduce_assessment = reduce_only.assess()?;
    assert_eq!(reduce_assessment.decision(), RiskDecision::ReduceOnly);
    assert_eq!(
        reduce_assessment
            .authorized_change_notional_per_leg()
            .value(),
        Decimal::ZERO
    );
    Ok(())
}

#[test]
fn flatten_requires_reduction_toward_zero() -> TestResult {
    let mut increase = Fixture::increase()?;
    increase.kill_state = KillState::Flatten;
    let blocked = increase.assess()?;
    assert_eq!(blocked.decision(), RiskDecision::FlattenRequired);
    assert_eq!(
        blocked.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );

    let mut reduction = Fixture::reduction()?;
    reduction.kill_state = KillState::Flatten;
    let allowed = reduction.assess()?;
    assert_eq!(allowed.decision(), RiskDecision::FlattenRequired);
    assert_eq!(
        allowed.authorized_change_notional_per_leg().value(),
        Decimal::from(100)
    );
    let audit = allowed.exposure().ok_or("missing reduction audit")?;
    assert_eq!(
        audit.pair_authorized_projected_notional_per_leg.value(),
        Decimal::from(100)
    );
    Ok(())
}

#[test]
fn halt_blocks_new_risk() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.kill_state = KillState::Halt;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::HaltRequired);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    Ok(())
}

#[test]
fn most_restrictive_regime_and_kill_state_wins() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.regime = Regime::ReduceOnly;
    fixture.kill_state = KillState::Flatten;
    let flatten = fixture.assess()?;
    assert_eq!(flatten.decision(), RiskDecision::FlattenRequired);
    assert!(has_reason(&flatten, RiskReasonCode::RegimeReduceOnly));
    assert!(has_reason(&flatten, RiskReasonCode::KillFlatten));

    fixture.kill_state = KillState::Halt;
    let halt = fixture.assess()?;
    assert_eq!(halt.decision(), RiskDecision::HaltRequired);
    Ok(())
}

#[test]
fn restrictive_regimes_block_increase_and_halted_escalates() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.regime = Regime::ReduceOnly;
    let reduce_only = fixture.assess()?;
    assert_eq!(reduce_only.decision(), RiskDecision::ReduceOnly);
    assert_eq!(
        reduce_only.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );

    fixture.regime = Regime::Halted;
    let halted = fixture.assess()?;
    assert_eq!(halted.decision(), RiskDecision::FlattenRequired);
    assert_eq!(
        halted.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    Ok(())
}

#[test]
fn per_venue_limit_breach_is_denied() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.config.risk.max_venue_notional = Notional::new(Decimal::from(1_000))?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    for venue in &mut exposure.venues {
        venue.exposure = ExposureComponents {
            actual: Notional::new(Decimal::from(951))?,
            reserved: Notional::new(Decimal::ZERO)?,
            pending: Notional::new(Decimal::ZERO)?,
        };
    }
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::VenueLimitExceeded));
    Ok(())
}

#[test]
fn pair_projected_limit_breach_is_denied() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.config.risk.max_pair_notional = Notional::new(Decimal::from(300))?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::PairLimitExceeded));
    let audit = assessment.exposure().ok_or("missing exposure audit")?;
    assert_eq!(
        audit.pair_current_notional_per_leg.value(),
        Decimal::from(275)
    );
    assert_eq!(
        audit.pair_candidate_projected_notional_per_leg.value(),
        Decimal::from(325)
    );
    assert_eq!(
        audit.pair_authorized_projected_notional_per_leg.value(),
        Decimal::from(275)
    );
    Ok(())
}

#[test]
fn reserved_and_pending_exposure_are_counted_in_projection() -> TestResult {
    let mut fixture = Fixture::increase()?;
    set_effective_exposure(
        &mut fixture,
        Decimal::from(200),
        Decimal::from(50),
        Decimal::from(25),
    )?;
    fixture.config.risk.max_pair_notional = Notional::new(Decimal::from(300))?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::PairLimitExceeded));
    Ok(())
}

#[test]
fn global_delta_breach_requires_flatten() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.global_delta.actual = Delta::new(Decimal::from(26));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::FlattenRequired);
    assert!(has_reason(
        &assessment,
        RiskReasonCode::GlobalDeltaLimitExceeded
    ));
    Ok(())
}

#[test]
fn no_change_with_current_pair_breach_is_restrictive() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    fixture.config.risk.max_pair_notional = Notional::new(Decimal::from(150))?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::ReduceOnly);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    assert!(has_reason(&assessment, RiskReasonCode::PairLimitExceeded));
    assert!(!has_reason(&assessment, RiskReasonCode::NoRiskChange));
    assert!(!has_reason(&assessment, RiskReasonCode::Approved));
    Ok(())
}

#[test]
fn no_change_with_current_venue_breach_is_restrictive() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.venues[0].exposure = ExposureComponents {
        actual: Notional::new(Decimal::from(1_001))?,
        reserved: Notional::new(Decimal::ZERO)?,
        pending: Notional::new(Decimal::ZERO)?,
    };
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::ReduceOnly);
    assert!(has_reason(&assessment, RiskReasonCode::VenueLimitExceeded));
    assert!(!has_reason(&assessment, RiskReasonCode::NoRiskChange));
    Ok(())
}

#[test]
fn no_change_with_current_global_delta_breach_is_restrictive() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.global_delta.actual = Delta::new(Decimal::from(26));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::FlattenRequired);
    assert!(has_reason(
        &assessment,
        RiskReasonCode::GlobalDeltaLimitExceeded
    ));
    assert!(!has_reason(&assessment, RiskReasonCode::NoRiskChange));
    Ok(())
}

#[test]
fn reduction_from_pair_breach_is_allowed_but_breach_visible() -> TestResult {
    let mut fixture = Fixture::reduction()?;
    fixture.config.risk.max_pair_notional = Notional::new(Decimal::from(150))?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::ReduceOnly);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::from(100)
    );
    assert!(has_reason(&assessment, RiskReasonCode::PairLimitExceeded));
    let audit = assessment.exposure().ok_or("missing reduction audit")?;
    assert_eq!(
        audit.pair_current_notional_per_leg.value(),
        Decimal::from(200)
    );
    assert_eq!(
        audit.pair_authorized_projected_notional_per_leg.value(),
        Decimal::from(100)
    );
    Ok(())
}

#[test]
fn reduction_does_not_hide_global_delta_breach() -> TestResult {
    let mut fixture = Fixture::reduction()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.global_delta.actual = Delta::new(Decimal::from(26));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::FlattenRequired);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::from(100)
    );
    assert!(has_reason(
        &assessment,
        RiskReasonCode::GlobalDeltaLimitExceeded
    ));
    Ok(())
}

#[test]
fn exactly_at_current_limit_is_not_a_breach() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    fixture.config.risk.max_pair_notional = Notional::new(Decimal::from(200))?;
    fixture.config.risk.max_venue_notional = Notional::new(Decimal::from(200))?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.global_delta.actual = Delta::new(Decimal::from(25));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Approve);
    assert!(has_reason(&assessment, RiskReasonCode::NoRiskChange));
    assert!(!has_reason(&assessment, RiskReasonCode::PairLimitExceeded));
    assert!(!has_reason(&assessment, RiskReasonCode::VenueLimitExceeded));
    assert!(!has_reason(
        &assessment,
        RiskReasonCode::GlobalDeltaLimitExceeded
    ));
    Ok(())
}

#[test]
fn current_exposure_arithmetic_failure_fails_closed() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.pair_per_leg = ExposureComponents {
        actual: Notional::new(Decimal::MAX)?,
        reserved: Notional::new(Decimal::ONE)?,
        pending: Notional::new(Decimal::ZERO)?,
    };
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    assert!(has_reason(&assessment, RiskReasonCode::ArithmeticFailure));
    assert!(!has_reason(&assessment, RiskReasonCode::NoRiskChange));
    Ok(())
}

#[test]
fn no_change_with_current_exposure_identity_failure_fails_closed() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.symbol = Symbol::try_from("OTHER")?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(
        &assessment,
        RiskReasonCode::ExposureIdentityMismatch
    ));
    assert!(!has_reason(&assessment, RiskReasonCode::NoRiskChange));
    Ok(())
}

#[test]
fn no_change_with_missing_current_exposure_fails_closed() -> TestResult {
    let mut fixture = Fixture::no_change()?;
    fixture.exposure = None;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::ExposureMissing));
    assert!(!has_reason(&assessment, RiskReasonCode::NoRiskChange));
    Ok(())
}

#[test]
fn session_loss_at_limit_requires_configured_flatten() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.session_pnl = Some(Money::new(Decimal::from(-100)));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::FlattenRequired);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::ZERO
    );
    assert!(has_reason(
        &assessment,
        RiskReasonCode::SessionLossLimitReached
    ));
    Ok(())
}

#[test]
fn session_loss_can_require_configured_halt() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.config.risk.session_loss_action = riftbot::config::SessionLossAction::Halt;
    fixture.session_pnl = Some(Money::new(Decimal::from(-100)));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::HaltRequired);
    assert_eq!(
        assessment.limits().session_loss_required_state,
        KillState::Halt
    );
    Ok(())
}

#[test]
fn delta_neutral_basket_is_still_denied_on_venue_limit() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.config.risk.max_venue_notional = Notional::new(Decimal::from(300))?;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::VenueLimitExceeded));
    assert!(!has_reason(
        &assessment,
        RiskReasonCode::GlobalDeltaLimitExceeded
    ));
    let audit = assessment.exposure().ok_or("missing audit")?;
    assert_eq!(audit.global_delta_current.value(), Decimal::ZERO);
    Ok(())
}

#[test]
fn exactly_at_hard_limit_boundary_is_allowed() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.config.risk.max_pair_notional = Notional::new(Decimal::from(325))?;
    fixture.config.risk.max_venue_notional = Notional::new(Decimal::from(325))?;
    fixture.config.risk.max_global_delta = Delta::new(Decimal::from(25));
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.global_delta.actual = Delta::new(Decimal::from(25));
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Approve);
    assert_eq!(
        assessment.authorized_change_notional_per_leg().value(),
        Decimal::from(50)
    );
    Ok(())
}

#[test]
fn fixed_decimal_arithmetic_overflow_fails_closed() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let exposure = fixture.exposure.as_mut().ok_or("missing exposure")?;
    exposure.pair_per_leg = ExposureComponents {
        actual: Notional::new(Decimal::MAX)?,
        reserved: Notional::new(Decimal::ONE)?,
        pending: Notional::new(Decimal::ZERO)?,
    };
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::ArithmeticFailure));
    Ok(())
}

#[test]
fn missing_health_input_fails_closed_for_increase() -> TestResult {
    let mut fixture = Fixture::increase()?;
    fixture.health = None;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    assert!(has_reason(&assessment, RiskReasonCode::HealthMissing));
    Ok(())
}

#[test]
fn unhealthy_stale_and_unknown_operational_inputs_block_increase() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let health = fixture.health.as_mut().ok_or("missing health")?;
    let first = health.venues.get_mut(0).ok_or("missing venue health")?;
    first.market_feed = HealthStatus::Stale;
    first.connectivity = HealthStatus::Unhealthy;
    first.account_private_stream = HealthStatus::Unknown;
    health.reconciliation = HealthStatus::Unhealthy;
    health.state_freshness = HealthStatus::Stale;
    health.latency = HealthStatus::Degraded;
    health.unknown_operations = 1;
    health.outstanding_operations = 1;
    health.outstanding_exposure_included = false;
    let assessment = fixture.assess()?;
    assert_eq!(assessment.decision(), RiskDecision::Deny);
    for reason in [
        RiskReasonCode::MarketFeedNotHealthy,
        RiskReasonCode::VenueConnectivityNotHealthy,
        RiskReasonCode::AccountStreamNotHealthy,
        RiskReasonCode::ReconciliationNotHealthy,
        RiskReasonCode::StateNotFresh,
        RiskReasonCode::LatencyNotHealthy,
        RiskReasonCode::UnknownOperations,
        RiskReasonCode::OutstandingExposureUnaccounted,
    ] {
        assert!(has_reason(&assessment, reason));
    }
    Ok(())
}

#[test]
fn identical_input_produces_identical_risk_assessment() -> TestResult {
    let fixture = Fixture::increase()?;
    assert_eq!(fixture.assess()?, fixture.assess()?);
    Ok(())
}

#[test]
fn authorization_uses_only_supplied_logical_time() -> TestResult {
    let mut fixture = Fixture::increase()?;
    let observed_at = fixture
        .inventory
        .increase_size_basis
        .as_ref()
        .ok_or("missing basis")?
        .observed_at;
    fixture.evaluated_at =
        UnixNanos(observed_at.0 + fixture.config.risk.max_measurement_age_ms.0 * 1_000_000);
    let boundary = fixture.assess()?;
    assert_eq!(boundary.decision(), RiskDecision::Approve);

    fixture.evaluated_at = UnixNanos(fixture.evaluated_at.0 + 1);
    let stale = fixture.assess()?;
    assert_eq!(stale.decision(), RiskDecision::Deny);
    assert!(has_reason(&stale, RiskReasonCode::MeasurementStale));
    Ok(())
}

#[test]
fn kill_state_transitions_are_timestamped_and_fail_closed() -> TestResult {
    let mut machine = KillStateMachine::new(KillState::Ready, UnixNanos(100));
    let transition = machine.transition(
        KillState::Flatten,
        "session loss reached",
        UnixNanos(101),
        "risk_manager",
    )?;
    assert_eq!(transition.from(), KillState::Ready);
    assert_eq!(transition.to(), KillState::Flatten);
    assert_eq!(transition.timestamp(), UnixNanos(101));
    assert_eq!(transition.reason(), "session loss reached");
    assert_eq!(transition.trigger(), "risk_manager");
    assert!(
        machine
            .transition(
                KillState::Ready,
                "unsafe recovery",
                UnixNanos(102),
                "operator"
            )
            .is_err()
    );
    assert!(
        machine
            .transition(
                KillState::Halt,
                "regressive clock",
                UnixNanos(99),
                "operator"
            )
            .is_err()
    );
    assert_eq!(machine.state(), KillState::Flatten);
    Ok(())
}

#[test]
fn p5_risk_module_has_no_execution_or_order_path() {
    let source = include_str!("../src/risk/manager.rs");
    let execution_intent = ["Execution", "Intent"].concat();
    let order_submission = ["submit", "_order"].concat();
    let nautilus = ["Nauti", "lus"].concat();
    assert!(!source.contains(&execution_intent));
    assert!(!source.contains(&order_submission));
    assert!(!source.contains(&nautilus));
}
