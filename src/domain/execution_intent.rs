//! Validated P6 execution intents and evidence-backed recovery contracts.

use std::collections::HashSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    ids::{DecisionId, EvidenceId, FillId, InstrumentId, IntentId, PairId, Symbol, VenueId},
    inventory::TargetDirection,
    numeric::{BaseQty, Bps, Delta, DurationMillis, Notional, Price, UnixNanos},
    risk::{KillState, Regime, RiskContext, RiskDecision, RiskInputAction},
};
use crate::strategy::inventory_manager::{InventoryAction, InventoryDecision};

const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Why an intent is allowed to affect exposure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIntentPurpose {
    IncreaseRisk,
    ReduceRisk,
    ResidualHedge,
    EmergencyFlatten,
}

/// Order side for one execution leg.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Allowed V1 order policies. There is intentionally no unbounded market-order policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderPolicy {
    MarketableLimit,
    ImmediateOrCancel,
}

/// Side-aware finite executable price boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "price", rename_all = "snake_case")]
pub enum PriceGuard {
    MaximumBuy(Price),
    MinimumSell(Price),
}

impl PriceGuard {
    #[must_use]
    pub const fn price(self) -> Price {
        match self {
            Self::MaximumBuy(price) | Self::MinimumSell(price) => price,
        }
    }
}

/// Quantity constraints supplied by instrument metadata rather than strategy assumptions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstrumentExecutionMetadata {
    pub instrument: InstrumentId,
    pub lot_size: BaseQty,
    pub quantity_precision: u32,
    pub supports_reduce_only: bool,
}

/// Caller facts from which one normal strategy leg is derived conservatively.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalLegRequest {
    pub venue: VenueId,
    pub instrument: InstrumentId,
    pub reference_price: Price,
    pub metadata: InstrumentExecutionMetadata,
    pub order_policy: OrderPolicy,
    pub price_guard: PriceGuard,
}

/// A validated order leg. Quantity and notional are derived, never caller-enlarged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLeg {
    pub venue: VenueId,
    pub instrument: InstrumentId,
    pub side: OrderSide,
    pub target_qty: BaseQty,
    pub target_notional: Notional,
    pub reference_price: Price,
    pub lot_size: BaseQty,
    pub quantity_precision: u32,
    pub supports_reduce_only: bool,
    pub reduce_only: bool,
    pub order_policy: OrderPolicy,
    pub price_guard: PriceGuard,
}

/// Actual parent-basket fill used to prove a residual recovery boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualFillEvidence {
    pub fill_id: FillId,
    pub parent_intent_id: IntentId,
    pub leg_index: usize,
    pub venue: VenueId,
    pub instrument: InstrumentId,
    pub side: OrderSide,
    pub filled_qty: BaseQty,
    pub filled_notional: Notional,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
}

/// Evidence frozen into an internally generated residual hedge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualHedgeEvidence {
    parent_intent_id: IntentId,
    fills: Vec<ResidualFillEvidence>,
    current_residual: Delta,
    projected_residual: Delta,
    unmatched_filled_notional: Notional,
}

impl ResidualHedgeEvidence {
    #[must_use]
    pub fn parent_intent_id(&self) -> &IntentId {
        &self.parent_intent_id
    }

    #[must_use]
    pub fn fills(&self) -> &[ResidualFillEvidence] {
        &self.fills
    }

    #[must_use]
    pub const fn current_residual(&self) -> Delta {
        self.current_residual
    }

    #[must_use]
    pub const fn projected_residual(&self) -> Delta {
        self.projected_residual
    }

    #[must_use]
    pub const fn unmatched_filled_notional(&self) -> Notional {
        self.unmatched_filled_notional
    }
}

/// Known position fact required before constructing an emergency flatten intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyPositionEvidence {
    pub evidence_id: EvidenceId,
    pub current_delta: Delta,
    pub observed_at: UnixNanos,
}

/// Evidence frozen into an emergency flatten intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyFlattenEvidence {
    position: EmergencyPositionEvidence,
    projected_delta: Delta,
}

impl EmergencyFlattenEvidence {
    #[must_use]
    pub const fn position(&self) -> &EmergencyPositionEvidence {
        &self.position
    }

    #[must_use]
    pub const fn projected_delta(&self) -> Delta {
        self.projected_delta
    }
}

/// Purpose-specific evidence. Normal intents instead carry their complete P4 source decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionSafetyEvidence {
    ResidualHedge(ResidualHedgeEvidence),
    EmergencyFlatten(EmergencyFlattenEvidence),
}

/// Complete normal-strategy construction request.
#[derive(Clone, Debug)]
pub struct NormalExecutionIntentRequest {
    pub intent_id: IntentId,
    pub purpose: ExecutionIntentPurpose,
    pub source_inventory: InventoryDecision,
    pub risk_context: RiskContext,
    pub leg_requests: [NormalLegRequest; 2],
    pub created_at: UnixNanos,
    pub expiry: UnixNanos,
    pub max_residual_delta: Delta,
    pub max_slippage_bps: Bps,
}

/// Internal residual-recovery construction request.
#[derive(Clone, Debug)]
pub(crate) struct ResidualHedgeIntentRequest {
    pub intent_id: IntentId,
    pub parent: ExecutionIntent,
    pub fill_evidence: Vec<ResidualFillEvidence>,
    pub recovery_leg: NormalLegRequest,
    pub maximum_hedge_notional: Notional,
    pub created_at: UnixNanos,
    pub expiry: UnixNanos,
}

/// Emergency flatten construction request. Safety evidence is mandatory.
#[derive(Clone, Debug)]
pub struct EmergencyFlattenIntentRequest {
    pub intent_id: IntentId,
    pub decision_id: DecisionId,
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub risk_context: RiskContext,
    pub position_evidence: EmergencyPositionEvidence,
    pub flatten_leg: NormalLegRequest,
    pub maximum_flatten_notional: Notional,
    pub created_at: UnixNanos,
    pub expiry: UnixNanos,
    pub max_residual_delta: Delta,
    pub max_slippage_bps: Bps,
}

/// Fields accepted by the validated serde boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionIntentParams {
    intent_id: IntentId,
    decision_id: DecisionId,
    purpose: ExecutionIntentPurpose,
    pair_id: PairId,
    symbol: Symbol,
    source_inventory: Option<InventoryDecision>,
    risk_context: RiskContext,
    authorized_matched_notional_per_leg: Notional,
    target_net_delta: Delta,
    max_residual_delta: Delta,
    max_slippage_bps: Bps,
    legs: Vec<ExecutionLeg>,
    created_at: UnixNanos,
    expiry: UnixNanos,
    safety_evidence: Option<ExecutionSafetyEvidence>,
}

/// Invalid or unsafe execution-intent construction.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExecutionIntentError {
    #[error("normal V1 intent requires exactly two legs; recovery requires exactly one")]
    InvalidLegCount,
    #[error("execution leg route, side, instrument, or reduce-only shape is invalid")]
    InvalidLegShape,
    #[error("execution leg metadata or quantity precision is invalid")]
    InvalidInstrumentMetadata,
    #[error("execution leg price guard does not match its side")]
    PriceGuardSideMismatch,
    #[error("execution intent expiry or logical timestamp ordering is invalid")]
    InvalidTime,
    #[error("execution intent residual or slippage limit is invalid")]
    InvalidLimit,
    #[error("intent, P4, P5, symbol, pair, regime, or kill-state identity differs")]
    IdentityMismatch,
    #[error("intent purpose is incompatible with its P4/P5 authorization")]
    PurposeNotAuthorized,
    #[error("normal strategy size exceeds P4, P5, or P3 authority")]
    AuthorizationExceeded,
    #[error("quantity rounding produced no safe executable size")]
    NoExecutableQuantity,
    #[error("planned initial residual exceeds the frozen tolerance")]
    PlannedResidualExceeded,
    #[error("residual hedge lacks coherent actual fill evidence")]
    InvalidResidualEvidence,
    #[error("residual hedge is unbounded or does not strictly reduce residual risk")]
    ResidualNotReduced,
    #[error("emergency flatten lacks coherent known-position evidence")]
    InvalidEmergencyEvidence,
    #[error("emergency flatten is unbounded, crosses zero, or does not reduce exposure")]
    EmergencyDoesNotReduce,
    #[error("checked fixed-decimal execution arithmetic failed")]
    Arithmetic,
}

/// Immutable, validated execution authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "ExecutionIntentParams")]
pub struct ExecutionIntent {
    intent_id: IntentId,
    decision_id: DecisionId,
    purpose: ExecutionIntentPurpose,
    pair_id: PairId,
    symbol: Symbol,
    source_inventory: Option<InventoryDecision>,
    risk_context: RiskContext,
    authorized_matched_notional_per_leg: Notional,
    target_net_delta: Delta,
    max_residual_delta: Delta,
    max_slippage_bps: Bps,
    legs: Vec<ExecutionLeg>,
    created_at: UnixNanos,
    expiry: UnixNanos,
    safety_evidence: Option<ExecutionSafetyEvidence>,
}

impl ExecutionIntent {
    /// Derives a normal two-leg basket from P4/P5 authority and instrument metadata.
    pub fn new_normal(request: NormalExecutionIntentRequest) -> Result<Self, ExecutionIntentError> {
        let source = &request.source_inventory;
        let assessment = &request.risk_context.assessment;
        validate_normal_authority(
            request.purpose,
            source,
            &request.risk_context,
            request.created_at,
        )?;
        let authorized = assessment.authorized_change_notional_per_leg();
        let (long_venue, short_venue, long_side, short_side) =
            normal_route(source, request.purpose)?;
        let [long_request, short_request] = request.leg_requests;
        if long_request.venue != *long_venue || short_request.venue != *short_venue {
            return Err(ExecutionIntentError::InvalidLegShape);
        }
        let measured_cap = if request.purpose == ExecutionIntentPurpose::IncreaseRisk {
            Some(
                source
                    .increase_size_basis
                    .as_ref()
                    .ok_or(ExecutionIntentError::PurposeNotAuthorized)?,
            )
        } else {
            None
        };
        let long_leg = derive_leg(
            long_request,
            long_side,
            request.purpose == ExecutionIntentPurpose::ReduceRisk,
            authorized,
            measured_cap.map(|basis| basis.requested_base_quantity),
        )?;
        let short_leg = derive_leg(
            short_request,
            short_side,
            request.purpose == ExecutionIntentPurpose::ReduceRisk,
            authorized,
            measured_cap.map(|basis| basis.requested_base_quantity),
        )?;
        if let Some(basis) = measured_cap
            && (long_leg.target_notional.value() > basis.long_measured_notional.value()
                || short_leg.target_notional.value() > basis.short_measured_notional.value())
        {
            return Err(ExecutionIntentError::AuthorizationExceeded);
        }
        let target_net_delta = signed_leg_total([&long_leg, &short_leg])?;
        if absolute(target_net_delta.value())? > request.max_residual_delta.value() {
            return Err(ExecutionIntentError::PlannedResidualExceeded);
        }
        Self::try_from(ExecutionIntentParams {
            intent_id: request.intent_id,
            decision_id: source.decision_id.clone(),
            purpose: request.purpose,
            pair_id: source.pair_id.clone(),
            symbol: source.symbol.clone(),
            source_inventory: Some(request.source_inventory),
            risk_context: request.risk_context,
            authorized_matched_notional_per_leg: authorized,
            target_net_delta,
            max_residual_delta: request.max_residual_delta,
            max_slippage_bps: request.max_slippage_bps,
            legs: vec![long_leg, short_leg],
            created_at: request.created_at,
            expiry: request.expiry,
            safety_evidence: None,
        })
    }

    /// Constructs one bounded recovery leg from actual parent fill imbalance.
    pub(crate) fn new_residual_hedge(
        request: ResidualHedgeIntentRequest,
    ) -> Result<Self, ExecutionIntentError> {
        if request.maximum_hedge_notional.value() <= Decimal::ZERO
            || request.created_at < request.parent.created_at
            || request
                .fill_evidence
                .iter()
                .any(|fill| fill.receive_ts > request.created_at)
        {
            return Err(ExecutionIntentError::InvalidResidualEvidence);
        }
        let current_residual =
            residual_from_fill_evidence(request.parent.intent_id(), &request.fill_evidence)?;
        let current_abs = absolute(current_residual.value())?;
        if current_abs == Decimal::ZERO || request.maximum_hedge_notional.value() > current_abs {
            return Err(ExecutionIntentError::ResidualNotReduced);
        }
        let expected_side = if current_residual.value() > Decimal::ZERO {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        };
        let backs_filled_side = request.fill_evidence.iter().any(|fill| {
            fill.venue == request.recovery_leg.venue
                && fill.instrument == request.recovery_leg.instrument
                && fill.side != expected_side
        });
        if !backs_filled_side {
            return Err(ExecutionIntentError::InvalidResidualEvidence);
        }
        let recovery_leg = derive_leg(
            request.recovery_leg,
            expected_side,
            true,
            request.maximum_hedge_notional,
            None,
        )?;
        let projected = current_residual
            .value()
            .checked_add(signed_notional(
                expected_side,
                recovery_leg.target_notional,
            )?)
            .ok_or(ExecutionIntentError::Arithmetic)?;
        if absolute(projected)? >= current_abs {
            return Err(ExecutionIntentError::ResidualNotReduced);
        }
        let unmatched_filled_notional = Notional::new(current_abs)
            .map_err(|_| ExecutionIntentError::InvalidResidualEvidence)?;
        let evidence = ResidualHedgeEvidence {
            parent_intent_id: request.parent.intent_id.clone(),
            fills: request.fill_evidence,
            current_residual,
            projected_residual: Delta::new(projected),
            unmatched_filled_notional,
        };
        let target_notional = recovery_leg.target_notional;
        let target_net_delta = Delta::new(signed_notional(expected_side, target_notional)?);
        Self::try_from(ExecutionIntentParams {
            intent_id: request.intent_id,
            decision_id: request.parent.decision_id.clone(),
            purpose: ExecutionIntentPurpose::ResidualHedge,
            pair_id: request.parent.pair_id.clone(),
            symbol: request.parent.symbol.clone(),
            source_inventory: None,
            risk_context: request.parent.risk_context.clone(),
            authorized_matched_notional_per_leg: target_notional,
            target_net_delta,
            max_residual_delta: request.parent.max_residual_delta,
            max_slippage_bps: request.parent.max_slippage_bps,
            legs: vec![recovery_leg],
            created_at: request.created_at,
            expiry: request.expiry,
            safety_evidence: Some(ExecutionSafetyEvidence::ResidualHedge(evidence)),
        })
    }

    /// Constructs one bounded emergency leg from known-position evidence.
    pub fn new_emergency_flatten(
        request: EmergencyFlattenIntentRequest,
    ) -> Result<Self, ExecutionIntentError> {
        let assessment = &request.risk_context.assessment;
        if assessment.decision() != RiskDecision::FlattenRequired
            || assessment.decision_id() != &request.decision_id
            || request.risk_context.regime != assessment.regime()
            || request.risk_context.kill_state != assessment.kill_state()
            || request.position_evidence.current_delta.value() == Decimal::ZERO
            || request.position_evidence.observed_at > request.created_at
        {
            return Err(ExecutionIntentError::InvalidEmergencyEvidence);
        }
        let current = request.position_evidence.current_delta.value();
        let current_abs = absolute(current)?;
        if request.maximum_flatten_notional.value() <= Decimal::ZERO
            || request.maximum_flatten_notional.value() > current_abs
        {
            return Err(ExecutionIntentError::EmergencyDoesNotReduce);
        }
        let expected_side = if current > Decimal::ZERO {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        };
        let leg = derive_leg(
            request.flatten_leg,
            expected_side,
            true,
            request.maximum_flatten_notional,
            None,
        )?;
        let projected = current
            .checked_add(signed_notional(expected_side, leg.target_notional)?)
            .ok_or(ExecutionIntentError::Arithmetic)?;
        let crosses_zero = (current > Decimal::ZERO && projected < Decimal::ZERO)
            || (current < Decimal::ZERO && projected > Decimal::ZERO);
        if crosses_zero || absolute(projected)? >= current_abs {
            return Err(ExecutionIntentError::EmergencyDoesNotReduce);
        }
        let evidence = EmergencyFlattenEvidence {
            position: request.position_evidence,
            projected_delta: Delta::new(projected),
        };
        let target_notional = leg.target_notional;
        let target_net_delta = Delta::new(signed_notional(expected_side, target_notional)?);
        Self::try_from(ExecutionIntentParams {
            intent_id: request.intent_id,
            decision_id: request.decision_id,
            purpose: ExecutionIntentPurpose::EmergencyFlatten,
            pair_id: request.pair_id,
            symbol: request.symbol,
            source_inventory: None,
            risk_context: request.risk_context,
            authorized_matched_notional_per_leg: target_notional,
            target_net_delta,
            max_residual_delta: request.max_residual_delta,
            max_slippage_bps: request.max_slippage_bps,
            legs: vec![leg],
            created_at: request.created_at,
            expiry: request.expiry,
            safety_evidence: Some(ExecutionSafetyEvidence::EmergencyFlatten(evidence)),
        })
    }

    #[must_use]
    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    #[must_use]
    pub fn decision_id(&self) -> &DecisionId {
        &self.decision_id
    }

    #[must_use]
    pub const fn purpose(&self) -> ExecutionIntentPurpose {
        self.purpose
    }

    #[must_use]
    pub fn pair_id(&self) -> &PairId {
        &self.pair_id
    }

    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    #[must_use]
    pub const fn source_inventory(&self) -> Option<&InventoryDecision> {
        self.source_inventory.as_ref()
    }

    #[must_use]
    pub const fn risk_context(&self) -> &RiskContext {
        &self.risk_context
    }

    #[must_use]
    pub const fn authorized_matched_notional_per_leg(&self) -> Notional {
        self.authorized_matched_notional_per_leg
    }

    #[must_use]
    pub const fn target_net_delta(&self) -> Delta {
        self.target_net_delta
    }

    #[must_use]
    pub const fn max_residual_delta(&self) -> Delta {
        self.max_residual_delta
    }

    #[must_use]
    pub const fn max_slippage_bps(&self) -> Bps {
        self.max_slippage_bps
    }

    #[must_use]
    pub fn legs(&self) -> &[ExecutionLeg] {
        &self.legs
    }

    #[must_use]
    pub const fn created_at(&self) -> UnixNanos {
        self.created_at
    }

    #[must_use]
    pub const fn expiry(&self) -> UnixNanos {
        self.expiry
    }

    #[must_use]
    pub const fn safety_evidence(&self) -> Option<&ExecutionSafetyEvidence> {
        self.safety_evidence.as_ref()
    }
}

impl TryFrom<ExecutionIntentParams> for ExecutionIntent {
    type Error = ExecutionIntentError;

    fn try_from(params: ExecutionIntentParams) -> Result<Self, Self::Error> {
        validate_params(&params)?;
        Ok(Self {
            intent_id: params.intent_id,
            decision_id: params.decision_id,
            purpose: params.purpose,
            pair_id: params.pair_id,
            symbol: params.symbol,
            source_inventory: params.source_inventory,
            risk_context: params.risk_context,
            authorized_matched_notional_per_leg: params.authorized_matched_notional_per_leg,
            target_net_delta: params.target_net_delta,
            max_residual_delta: params.max_residual_delta,
            max_slippage_bps: params.max_slippage_bps,
            legs: params.legs,
            created_at: params.created_at,
            expiry: params.expiry,
            safety_evidence: params.safety_evidence,
        })
    }
}

fn validate_params(params: &ExecutionIntentParams) -> Result<(), ExecutionIntentError> {
    if params.expiry <= params.created_at
        || params.created_at < params.risk_context.assessment.evaluated_at()
    {
        return Err(ExecutionIntentError::InvalidTime);
    }
    if params.max_residual_delta.value() < Decimal::ZERO
        || params.max_slippage_bps.value() < Decimal::ZERO
        || params.authorized_matched_notional_per_leg.value() <= Decimal::ZERO
    {
        return Err(ExecutionIntentError::InvalidLimit);
    }
    let assessment = &params.risk_context.assessment;
    if assessment.decision_id() != &params.decision_id
        || assessment.regime() != params.risk_context.regime
        || assessment.kill_state() != params.risk_context.kill_state
    {
        return Err(ExecutionIntentError::IdentityMismatch);
    }
    for leg in &params.legs {
        validate_leg(leg)?;
        if leg.target_notional.value() > params.authorized_matched_notional_per_leg.value() {
            return Err(ExecutionIntentError::AuthorizationExceeded);
        }
    }
    let target = signed_leg_total(params.legs.iter())?;
    if target != params.target_net_delta {
        return Err(ExecutionIntentError::InvalidLegShape);
    }

    match params.purpose {
        ExecutionIntentPurpose::IncreaseRisk | ExecutionIntentPurpose::ReduceRisk => {
            if params.legs.len() != 2
                || params.safety_evidence.is_some()
                || params.source_inventory.is_none()
            {
                return Err(ExecutionIntentError::InvalidLegCount);
            }
            let source = params
                .source_inventory
                .as_ref()
                .ok_or(ExecutionIntentError::PurposeNotAuthorized)?;
            if source.pair_id != params.pair_id || source.symbol != params.symbol {
                return Err(ExecutionIntentError::IdentityMismatch);
            }
            validate_normal_authority(
                params.purpose,
                source,
                &params.risk_context,
                params.created_at,
            )?;
            if params.authorized_matched_notional_per_leg
                != assessment.authorized_change_notional_per_leg()
            {
                return Err(ExecutionIntentError::AuthorizationExceeded);
            }
            validate_normal_legs(params.purpose, source, &params.legs)?;
            if absolute(target.value())? > params.max_residual_delta.value() {
                return Err(ExecutionIntentError::PlannedResidualExceeded);
            }
            if params.purpose == ExecutionIntentPurpose::IncreaseRisk {
                let basis = source
                    .increase_size_basis
                    .as_ref()
                    .ok_or(ExecutionIntentError::PurposeNotAuthorized)?;
                if params
                    .legs
                    .iter()
                    .any(|leg| leg.target_qty.value() > basis.requested_base_quantity.value())
                    || params.legs[0].target_notional.value() > basis.long_measured_notional.value()
                    || params.legs[1].target_notional.value()
                        > basis.short_measured_notional.value()
                {
                    return Err(ExecutionIntentError::AuthorizationExceeded);
                }
            }
        }
        ExecutionIntentPurpose::ResidualHedge => validate_residual_params(params)?,
        ExecutionIntentPurpose::EmergencyFlatten => validate_emergency_params(params)?,
    }
    Ok(())
}

fn validate_normal_authority(
    purpose: ExecutionIntentPurpose,
    source: &InventoryDecision,
    risk_context: &RiskContext,
    created_at: UnixNanos,
) -> Result<(), ExecutionIntentError> {
    let assessment = &risk_context.assessment;
    let target = source
        .selected_target
        .as_ref()
        .ok_or(ExecutionIntentError::IdentityMismatch)?;
    let actual = source
        .effective_actual
        .as_ref()
        .ok_or(ExecutionIntentError::IdentityMismatch)?;
    let exposure = assessment
        .exposure()
        .ok_or(ExecutionIntentError::IdentityMismatch)?;
    let (actual_long, actual_short) = match &actual.direction {
        TargetDirection::LongShort {
            long_venue,
            short_venue,
        } => (long_venue, short_venue),
        TargetDirection::Flat => return Err(ExecutionIntentError::IdentityMismatch),
    };
    if source.decision_id != *assessment.decision_id()
        || target.decision_id() != &source.decision_id
        || target.pair_id() != &source.pair_id
        || target.symbol() != &source.symbol
        || risk_context.regime != assessment.regime()
        || risk_context.kill_state != assessment.kill_state()
        || created_at < assessment.evaluated_at()
        || source.block_reason.is_some()
        || assessment.requested_change_notional_per_leg() != source.required_change_notional_per_leg
        || assessment.proposed_change_notional_per_leg() != source.proposed_change_notional_per_leg
        || exposure.pair_current_notional_per_leg != actual.total_notional_per_leg
        || &exposure.long_venue != actual_long
        || &exposure.short_venue != actual_short
        || assessment.authorized_change_notional_per_leg().value() <= Decimal::ZERO
        || assessment.authorized_change_notional_per_leg().value()
            > source.proposed_change_notional_per_leg.value()
    {
        return Err(ExecutionIntentError::IdentityMismatch);
    }
    match purpose {
        ExecutionIntentPurpose::IncreaseRisk => {
            let basis = source
                .increase_size_basis
                .as_ref()
                .ok_or(ExecutionIntentError::PurposeNotAuthorized)?;
            let expected_required = target
                .target_notional()
                .value()
                .checked_sub(actual.total_notional_per_leg.value())
                .ok_or(ExecutionIntentError::Arithmetic)?;
            let expected_age = assessment
                .evaluated_at()
                .0
                .checked_sub(basis.observed_at.0)
                .map(|nanos| nanos / NANOS_PER_MILLISECOND);
            if source.action != InventoryAction::IncreaseRisk
                || assessment.input_action() != RiskInputAction::IncreaseRisk
                || assessment.decision() != RiskDecision::Approve
                || !matches!(risk_context.regime, Regime::Normal | Regime::Degraded)
                || risk_context.kill_state != KillState::Ready
                || target.direction() != &actual.direction
                || source.required_change_notional_per_leg.value() != expected_required
                || source.proposed_change_notional_per_leg.value()
                    != expected_required.min(basis.measured_matched_notional_cap.value())
                || basis.measured_matched_notional_cap.value()
                    != basis
                        .long_measured_notional
                        .value()
                        .min(basis.short_measured_notional.value())
                || !is_sha256(&basis.measurement_config_fingerprint)
                || assessment.measurement_age_ms().is_none()
                || expected_age.map(DurationMillis) != assessment.measurement_age_ms()
                || assessment.measurement_safe_matched_notional_cap()
                    != Some(basis.measured_matched_notional_cap)
                || assessment
                    .measurement_safe_matched_notional_cap()
                    .is_none_or(|cap| {
                        assessment.authorized_change_notional_per_leg().value() > cap.value()
                    })
            {
                return Err(ExecutionIntentError::PurposeNotAuthorized);
            }
        }
        ExecutionIntentPurpose::ReduceRisk => {
            let input_matches = matches!(
                (source.action, assessment.input_action()),
                (InventoryAction::ReduceRisk, RiskInputAction::ReduceRisk)
                    | (
                        InventoryAction::FlattenForReversal,
                        RiskInputAction::FlattenForReversal
                    )
            );
            if !input_matches
                || !matches!(
                    assessment.decision(),
                    RiskDecision::Approve
                        | RiskDecision::ReduceOnly
                        | RiskDecision::FlattenRequired
                )
                || source.required_change_notional_per_leg.value() >= Decimal::ZERO
                || source.proposed_change_notional_per_leg.value()
                    != absolute(source.required_change_notional_per_leg.value())?
                || (source.action == InventoryAction::ReduceRisk
                    && (target.direction() != &actual.direction
                        || target
                            .target_notional()
                            .value()
                            .checked_sub(actual.total_notional_per_leg.value())
                            != Some(source.required_change_notional_per_leg.value())))
                || (source.action == InventoryAction::FlattenForReversal
                    && source.required_change_notional_per_leg.value()
                        != Decimal::ZERO
                            .checked_sub(actual.total_notional_per_leg.value())
                            .ok_or(ExecutionIntentError::Arithmetic)?)
            {
                return Err(ExecutionIntentError::PurposeNotAuthorized);
            }
        }
        ExecutionIntentPurpose::ResidualHedge | ExecutionIntentPurpose::EmergencyFlatten => {
            return Err(ExecutionIntentError::PurposeNotAuthorized);
        }
    }
    Ok(())
}

fn normal_route(
    source: &InventoryDecision,
    purpose: ExecutionIntentPurpose,
) -> Result<(&VenueId, &VenueId, OrderSide, OrderSide), ExecutionIntentError> {
    let direction = if purpose == ExecutionIntentPurpose::IncreaseRisk {
        source
            .selected_target
            .as_ref()
            .map(|target| target.direction())
    } else {
        source
            .effective_actual
            .as_ref()
            .map(|actual| &actual.direction)
    };
    let Some(TargetDirection::LongShort {
        long_venue,
        short_venue,
    }) = direction
    else {
        return Err(ExecutionIntentError::InvalidLegShape);
    };
    let (long_side, short_side) = if purpose == ExecutionIntentPurpose::IncreaseRisk {
        (OrderSide::Buy, OrderSide::Sell)
    } else {
        (OrderSide::Sell, OrderSide::Buy)
    };
    Ok((long_venue, short_venue, long_side, short_side))
}

fn validate_normal_legs(
    purpose: ExecutionIntentPurpose,
    source: &InventoryDecision,
    legs: &[ExecutionLeg],
) -> Result<(), ExecutionIntentError> {
    let (long_venue, short_venue, long_side, short_side) = normal_route(source, purpose)?;
    if legs[0].venue != *long_venue
        || legs[1].venue != *short_venue
        || legs[0].side != long_side
        || legs[1].side != short_side
        || legs[0].venue == legs[1].venue
        || legs[0].instrument == legs[1].instrument
        || (purpose == ExecutionIntentPurpose::IncreaseRisk
            && legs.iter().any(|leg| leg.reduce_only))
        || (purpose == ExecutionIntentPurpose::ReduceRisk
            && legs
                .iter()
                .any(|leg| leg.supports_reduce_only && !leg.reduce_only))
    {
        return Err(ExecutionIntentError::InvalidLegShape);
    }
    Ok(())
}

fn validate_residual_params(params: &ExecutionIntentParams) -> Result<(), ExecutionIntentError> {
    if params.legs.len() != 1 || params.source_inventory.is_some() {
        return Err(ExecutionIntentError::InvalidLegCount);
    }
    let Some(ExecutionSafetyEvidence::ResidualHedge(evidence)) = &params.safety_evidence else {
        return Err(ExecutionIntentError::InvalidResidualEvidence);
    };
    if evidence.parent_intent_id == params.intent_id
        || evidence.fills.is_empty()
        || evidence.fills.iter().any(|fill| {
            fill.parent_intent_id != evidence.parent_intent_id
                || fill.receive_ts > params.created_at
        })
    {
        return Err(ExecutionIntentError::InvalidResidualEvidence);
    }
    let current = residual_from_evidence_only(&evidence.fills)?;
    let projected = current
        .value()
        .checked_add(signed_notional(
            params.legs[0].side,
            params.legs[0].target_notional,
        )?)
        .ok_or(ExecutionIntentError::Arithmetic)?;
    let current_abs = absolute(current.value())?;
    if current != evidence.current_residual
        || Delta::new(projected) != evidence.projected_residual
        || evidence.unmatched_filled_notional.value() != current_abs
        || params.legs[0].target_notional.value() > current_abs
        || absolute(projected)? >= current_abs
    {
        return Err(ExecutionIntentError::ResidualNotReduced);
    }
    Ok(())
}

fn validate_emergency_params(params: &ExecutionIntentParams) -> Result<(), ExecutionIntentError> {
    if params.legs.len() != 1 || params.source_inventory.is_some() {
        return Err(ExecutionIntentError::InvalidLegCount);
    }
    let Some(ExecutionSafetyEvidence::EmergencyFlatten(evidence)) = &params.safety_evidence else {
        return Err(ExecutionIntentError::InvalidEmergencyEvidence);
    };
    if params.risk_context.assessment.decision() != RiskDecision::FlattenRequired
        || evidence.position.current_delta.value() == Decimal::ZERO
        || evidence.position.observed_at > params.created_at
    {
        return Err(ExecutionIntentError::InvalidEmergencyEvidence);
    }
    let current = evidence.position.current_delta.value();
    let projected = current
        .checked_add(signed_notional(
            params.legs[0].side,
            params.legs[0].target_notional,
        )?)
        .ok_or(ExecutionIntentError::Arithmetic)?;
    let crosses_zero = (current > Decimal::ZERO && projected < Decimal::ZERO)
        || (current < Decimal::ZERO && projected > Decimal::ZERO);
    if crosses_zero
        || absolute(projected)? >= absolute(current)?
        || evidence.projected_delta != Delta::new(projected)
    {
        return Err(ExecutionIntentError::EmergencyDoesNotReduce);
    }
    Ok(())
}

fn derive_leg(
    request: NormalLegRequest,
    side: OrderSide,
    reduction: bool,
    budget: Notional,
    quantity_cap: Option<BaseQty>,
) -> Result<ExecutionLeg, ExecutionIntentError> {
    if request.instrument != request.metadata.instrument
        || request.metadata.quantity_precision > 28
        || request.metadata.lot_size.value().scale() > request.metadata.quantity_precision
    {
        return Err(ExecutionIntentError::InvalidInstrumentMetadata);
    }
    let mut candidate = budget
        .value()
        .checked_div(request.reference_price.value())
        .ok_or(ExecutionIntentError::Arithmetic)?;
    if let Some(cap) = quantity_cap {
        candidate = candidate.min(cap.value());
    }
    let steps = candidate
        .checked_div(request.metadata.lot_size.value())
        .ok_or(ExecutionIntentError::Arithmetic)?
        .trunc();
    let quantity = steps
        .checked_mul(request.metadata.lot_size.value())
        .ok_or(ExecutionIntentError::Arithmetic)?;
    if quantity <= Decimal::ZERO {
        return Err(ExecutionIntentError::NoExecutableQuantity);
    }
    let target_qty =
        BaseQty::new(quantity).map_err(|_| ExecutionIntentError::InvalidInstrumentMetadata)?;
    let target_notional_value = quantity
        .checked_mul(request.reference_price.value())
        .ok_or(ExecutionIntentError::Arithmetic)?;
    if target_notional_value > budget.value() {
        return Err(ExecutionIntentError::AuthorizationExceeded);
    }
    let target_notional =
        Notional::new(target_notional_value).map_err(|_| ExecutionIntentError::Arithmetic)?;
    let reduce_only = reduction && request.metadata.supports_reduce_only;
    let leg = ExecutionLeg {
        venue: request.venue,
        instrument: request.instrument,
        side,
        target_qty,
        target_notional,
        reference_price: request.reference_price,
        lot_size: request.metadata.lot_size,
        quantity_precision: request.metadata.quantity_precision,
        supports_reduce_only: request.metadata.supports_reduce_only,
        reduce_only,
        order_policy: request.order_policy,
        price_guard: request.price_guard,
    };
    validate_leg(&leg)?;
    Ok(leg)
}

pub(crate) fn validate_leg(leg: &ExecutionLeg) -> Result<(), ExecutionIntentError> {
    if !guard_matches_side(leg) {
        return Err(ExecutionIntentError::PriceGuardSideMismatch);
    }
    if leg.quantity_precision > 28
        || leg.lot_size.value().scale() > leg.quantity_precision
        || leg.target_qty.value().scale() > leg.quantity_precision
        || leg.target_qty.value().checked_rem(leg.lot_size.value()) != Some(Decimal::ZERO)
    {
        return Err(ExecutionIntentError::InvalidInstrumentMetadata);
    }
    let expected_notional = leg
        .target_qty
        .value()
        .checked_mul(leg.reference_price.value())
        .ok_or(ExecutionIntentError::Arithmetic)?;
    if expected_notional != leg.target_notional.value() || expected_notional <= Decimal::ZERO {
        return Err(ExecutionIntentError::InvalidLegShape);
    }
    Ok(())
}

fn guard_matches_side(leg: &ExecutionLeg) -> bool {
    matches!(
        (leg.side, leg.price_guard),
        (OrderSide::Buy, PriceGuard::MaximumBuy(_)) | (OrderSide::Sell, PriceGuard::MinimumSell(_))
    )
}

fn signed_leg_total<'a>(
    legs: impl IntoIterator<Item = &'a ExecutionLeg>,
) -> Result<Delta, ExecutionIntentError> {
    let value = legs.into_iter().try_fold(Decimal::ZERO, |total, leg| {
        total
            .checked_add(signed_notional(leg.side, leg.target_notional)?)
            .ok_or(ExecutionIntentError::Arithmetic)
    })?;
    Ok(Delta::new(value))
}

fn signed_notional(side: OrderSide, notional: Notional) -> Result<Decimal, ExecutionIntentError> {
    match side {
        OrderSide::Buy => Ok(notional.value()),
        OrderSide::Sell => Decimal::ZERO
            .checked_sub(notional.value())
            .ok_or(ExecutionIntentError::Arithmetic),
    }
}

fn residual_from_fill_evidence(
    parent_intent_id: &IntentId,
    fills: &[ResidualFillEvidence],
) -> Result<Delta, ExecutionIntentError> {
    if fills.is_empty() {
        return Err(ExecutionIntentError::InvalidResidualEvidence);
    }
    let mut ids = HashSet::with_capacity(fills.len());
    for fill in fills {
        if &fill.parent_intent_id != parent_intent_id
            || !ids.insert(&fill.fill_id)
            || fill.filled_notional.value() <= Decimal::ZERO
        {
            return Err(ExecutionIntentError::InvalidResidualEvidence);
        }
    }
    residual_from_evidence_only(fills)
}

fn residual_from_evidence_only(
    fills: &[ResidualFillEvidence],
) -> Result<Delta, ExecutionIntentError> {
    let mut ids = HashSet::with_capacity(fills.len());
    let value = fills.iter().try_fold(Decimal::ZERO, |total, fill| {
        if !ids.insert(&fill.fill_id) || fill.filled_notional.value() <= Decimal::ZERO {
            return Err(ExecutionIntentError::InvalidResidualEvidence);
        }
        total
            .checked_add(signed_notional(fill.side, fill.filled_notional)?)
            .ok_or(ExecutionIntentError::Arithmetic)
    })?;
    Ok(Delta::new(value))
}

fn absolute(value: Decimal) -> Result<Decimal, ExecutionIntentError> {
    if value < Decimal::ZERO {
        Decimal::ZERO
            .checked_sub(value)
            .ok_or(ExecutionIntentError::Arithmetic)
    } else {
        Ok(value)
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
