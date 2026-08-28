//! Basket-shaped execution contracts with V1 construction invariants.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    ids::{DecisionId, InstrumentId, IntentId, Symbol, VenueId},
    numeric::{BaseQty, Bps, Delta, Notional, Price, UnixNanos},
    risk::RiskContext,
};

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

/// Side-aware executable price boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "price", rename_all = "snake_case")]
pub enum PriceGuard {
    MaximumBuy(Price),
    MinimumSell(Price),
}

/// One venue leg inside an execution basket.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionLeg {
    pub venue: VenueId,
    pub instrument: InstrumentId,
    pub side: OrderSide,
    pub target_qty: BaseQty,
    pub target_notional: Notional,
    pub reduce_only: bool,
    pub order_policy: OrderPolicy,
    pub price_guard: PriceGuard,
}

/// Complete caller-supplied data used to construct a V1 intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionIntentParams {
    pub intent_id: IntentId,
    pub decision_id: DecisionId,
    pub symbol: Symbol,
    pub target_net_delta: Delta,
    pub max_residual_delta: Delta,
    pub max_slippage_bps: Bps,
    pub legs: Vec<ExecutionLeg>,
    pub created_at: UnixNanos,
    pub expiry: UnixNanos,
    pub risk_context: RiskContext,
}

/// Invalid V1 execution basket construction.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExecutionIntentError {
    /// V1 permits exactly two legs.
    #[error("V1 execution intent requires exactly two legs, received {actual}")]
    InvalidLegCount { actual: usize },
    /// Both legs addressed the same venue.
    #[error("V1 execution legs must use distinct venues")]
    DuplicateVenue,
    /// Both legs had the same side.
    #[error("V1 execution legs must have opposite sides")]
    SameSide,
    /// A price guard did not correspond to the leg side.
    #[error("execution leg price guard does not match its side")]
    PriceGuardSideMismatch,
    /// Expiry did not follow creation time.
    #[error("execution intent expiry must be later than creation time")]
    InvalidExpiry,
    /// Maximum residual delta was negative.
    #[error("maximum residual delta must be non-negative")]
    NegativeResidualLimit,
    /// Maximum slippage was negative.
    #[error("maximum slippage must be non-negative")]
    NegativeSlippageLimit,
}

/// Immutable, validated V1 execution basket.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "ExecutionIntentParams")]
pub struct ExecutionIntent {
    intent_id: IntentId,
    decision_id: DecisionId,
    symbol: Symbol,
    target_net_delta: Delta,
    max_residual_delta: Delta,
    max_slippage_bps: Bps,
    legs: Vec<ExecutionLeg>,
    created_at: UnixNanos,
    expiry: UnixNanos,
    risk_context: RiskContext,
}

impl ExecutionIntent {
    /// Validates and constructs a two-leg V1 basket.
    pub fn new_v1(params: ExecutionIntentParams) -> Result<Self, ExecutionIntentError> {
        Self::try_from(params)
    }
}

impl TryFrom<ExecutionIntentParams> for ExecutionIntent {
    type Error = ExecutionIntentError;

    fn try_from(params: ExecutionIntentParams) -> Result<Self, Self::Error> {
        let [first, second] = params.legs.as_slice() else {
            return Err(ExecutionIntentError::InvalidLegCount {
                actual: params.legs.len(),
            });
        };

        if first.venue == second.venue {
            return Err(ExecutionIntentError::DuplicateVenue);
        }
        if first.side == second.side {
            return Err(ExecutionIntentError::SameSide);
        }
        if !guard_matches_side(first) || !guard_matches_side(second) {
            return Err(ExecutionIntentError::PriceGuardSideMismatch);
        }
        if params.expiry <= params.created_at {
            return Err(ExecutionIntentError::InvalidExpiry);
        }
        if params.max_residual_delta.value() < Decimal::ZERO {
            return Err(ExecutionIntentError::NegativeResidualLimit);
        }
        if params.max_slippage_bps.value() < Decimal::ZERO {
            return Err(ExecutionIntentError::NegativeSlippageLimit);
        }

        Ok(Self {
            intent_id: params.intent_id,
            decision_id: params.decision_id,
            symbol: params.symbol,
            target_net_delta: params.target_net_delta,
            max_residual_delta: params.max_residual_delta,
            max_slippage_bps: params.max_slippage_bps,
            legs: params.legs,
            created_at: params.created_at,
            expiry: params.expiry,
            risk_context: params.risk_context,
        })
    }
}

impl ExecutionIntent {
    /// Returns the intent identifier.
    #[must_use]
    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    /// Returns the source decision identifier.
    #[must_use]
    pub fn decision_id(&self) -> &DecisionId {
        &self.decision_id
    }

    /// Returns the two validated V1 legs.
    #[must_use]
    pub fn legs(&self) -> &[ExecutionLeg] {
        &self.legs
    }

    /// Returns the frozen risk context.
    #[must_use]
    pub const fn risk_context(&self) -> &RiskContext {
        &self.risk_context
    }
}

fn guard_matches_side(leg: &ExecutionLeg) -> bool {
    matches!(
        (leg.side, leg.price_guard),
        (OrderSide::Buy, PriceGuard::MaximumBuy(_)) | (OrderSide::Sell, PriceGuard::MinimumSell(_))
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{
        ExecutionIntent, ExecutionIntentError, ExecutionIntentParams, ExecutionLeg, OrderPolicy,
        OrderSide, PriceGuard,
    };
    use crate::domain::{
        ids::{DecisionId, InstrumentId, IntentId, Symbol, VenueId},
        numeric::{BaseQty, Bps, Delta, Notional, Price, UnixNanos},
        risk::{KillState, Regime, RiskAssessment, RiskContext, RiskDecision},
    };

    fn leg(venue: &str, side: OrderSide) -> Result<ExecutionLeg, Box<dyn Error>> {
        let price = Price::new(Decimal::from(100))?;
        let price_guard = match side {
            OrderSide::Buy => PriceGuard::MaximumBuy(price),
            OrderSide::Sell => PriceGuard::MinimumSell(price),
        };
        Ok(ExecutionLeg {
            venue: VenueId::try_from(venue)?,
            instrument: InstrumentId::try_from(format!("BTC-PERP.{venue}"))?,
            side,
            target_qty: BaseQty::new(Decimal::ONE)?,
            target_notional: Notional::new(Decimal::from(100))?,
            reduce_only: false,
            order_policy: OrderPolicy::ImmediateOrCancel,
            price_guard,
        })
    }

    fn params(legs: Vec<ExecutionLeg>) -> Result<ExecutionIntentParams, Box<dyn Error>> {
        Ok(ExecutionIntentParams {
            intent_id: IntentId::try_from("intent-1")?,
            decision_id: DecisionId::try_from("decision-1")?,
            symbol: Symbol::try_from("BTC")?,
            target_net_delta: Delta::new(Decimal::ZERO),
            max_residual_delta: Delta::new(Decimal::from(10)),
            max_slippage_bps: Bps::new(Decimal::from(5)),
            legs,
            created_at: UnixNanos(1),
            expiry: UnixNanos(2),
            risk_context: RiskContext {
                regime: Regime::Normal,
                kill_state: KillState::Ready,
                assessment: RiskAssessment {
                    decision: RiskDecision::Approve,
                    reason: "limits satisfied".to_owned(),
                    evaluated_at: UnixNanos(1),
                },
            },
        })
    }

    #[test]
    fn v1_accepts_two_opposite_distinct_venue_legs() -> Result<(), Box<dyn Error>> {
        let legs = vec![
            leg("entropy", OrderSide::Sell)?,
            leg("lighter", OrderSide::Buy)?,
        ];
        let intent = ExecutionIntent::new_v1(params(legs)?)?;
        assert_eq!(intent.legs().len(), 2);
        Ok(())
    }

    #[test]
    fn v1_rejects_non_two_leg_basket() -> Result<(), Box<dyn Error>> {
        let result = ExecutionIntent::new_v1(params(vec![leg("entropy", OrderSide::Sell)?])?);
        assert!(matches!(
            result,
            Err(ExecutionIntentError::InvalidLegCount { actual: 1 })
        ));
        Ok(())
    }

    #[test]
    fn v1_rejects_same_side_legs() -> Result<(), Box<dyn Error>> {
        let legs = vec![
            leg("entropy", OrderSide::Buy)?,
            leg("lighter", OrderSide::Buy)?,
        ];
        let result = ExecutionIntent::new_v1(params(legs)?);
        assert_eq!(result, Err(ExecutionIntentError::SameSide));
        Ok(())
    }

    #[test]
    fn deserialization_cannot_bypass_v1_invariants() -> Result<(), Box<dyn Error>> {
        let invalid = params(vec![leg("entropy", OrderSide::Sell)?])?;
        let serialized = serde_json::to_string(&invalid)?;
        let result = serde_json::from_str::<ExecutionIntent>(&serialized);
        assert!(result.is_err());
        Ok(())
    }
}
