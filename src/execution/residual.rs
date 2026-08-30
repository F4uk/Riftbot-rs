//! Deterministic bounded residual-recovery planning.

use rust_decimal::Decimal;
use thiserror::Error;

use crate::domain::{
    execution_intent::{InstrumentExecutionMetadata, NormalLegRequest, OrderSide, PriceGuard},
    numeric::{Bps, Delta, Notional, Price},
};

use super::state_machine::{ChildOrderSnapshot, ExecutionStateError, absolute};

const BPS_SCALE: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

/// One recovery action which is guaranteed to address an actually filled side.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualRecoveryPlan {
    pub source_child_index: usize,
    pub leg_request: NormalLegRequest,
    pub maximum_hedge_notional: Notional,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ResidualPlanError {
    #[error("residual is already within zero or has no actually filled source side")]
    NoFilledExposure,
    #[error("recovery price bound cannot be represented safely")]
    InvalidPriceBound,
    #[error("checked residual recovery arithmetic failed")]
    Arithmetic,
    #[error(transparent)]
    State(#[from] ExecutionStateError),
}

/// Chooses a reduce-side recovery bounded by actual unmatched filled notional.
pub fn plan_residual_recovery(
    children: &[ChildOrderSnapshot],
    residual: Delta,
    max_slippage_bps: Bps,
) -> Result<ResidualRecoveryPlan, ResidualPlanError> {
    let residual_abs = absolute(residual.value())?;
    if residual_abs == Decimal::ZERO || max_slippage_bps.value() < Decimal::ZERO {
        return Err(ResidualPlanError::NoFilledExposure);
    }
    let source_side = if residual.value() > Decimal::ZERO {
        OrderSide::Buy
    } else {
        OrderSide::Sell
    };
    let recovery_side = if source_side == OrderSide::Buy {
        OrderSide::Sell
    } else {
        OrderSide::Buy
    };
    let Some((source_child_index, source)) = children
        .iter()
        .enumerate()
        .filter(|(_, child)| {
            child.leg.side == source_side && child.cumulative_filled_notional > Decimal::ZERO
        })
        .max_by_key(|(_, child)| child.cumulative_filled_notional)
    else {
        return Err(ResidualPlanError::NoFilledExposure);
    };
    let reference_price = source
        .average_fill_price
        .ok_or(ResidualPlanError::NoFilledExposure)?;
    let guard_price = bounded_guard_price(reference_price, recovery_side, max_slippage_bps)?;
    let maximum = residual_abs.min(source.cumulative_filled_notional);
    if maximum <= Decimal::ZERO {
        return Err(ResidualPlanError::NoFilledExposure);
    }
    Ok(ResidualRecoveryPlan {
        source_child_index,
        leg_request: NormalLegRequest {
            venue: source.leg.venue.clone(),
            instrument: source.leg.instrument.clone(),
            reference_price,
            metadata: InstrumentExecutionMetadata {
                instrument: source.leg.instrument.clone(),
                lot_size: source.leg.lot_size,
                quantity_precision: source.leg.quantity_precision,
                supports_reduce_only: source.leg.supports_reduce_only,
            },
            order_policy: source.leg.order_policy,
            price_guard: match recovery_side {
                OrderSide::Buy => PriceGuard::MaximumBuy(guard_price),
                OrderSide::Sell => PriceGuard::MinimumSell(guard_price),
            },
        },
        maximum_hedge_notional: Notional::new(maximum)
            .map_err(|_| ResidualPlanError::Arithmetic)?,
    })
}

fn bounded_guard_price(
    reference: Price,
    side: OrderSide,
    slippage: Bps,
) -> Result<Price, ResidualPlanError> {
    let ratio = slippage
        .value()
        .checked_div(BPS_SCALE)
        .ok_or(ResidualPlanError::Arithmetic)?;
    let factor = match side {
        OrderSide::Buy => Decimal::ONE.checked_add(ratio),
        OrderSide::Sell => Decimal::ONE.checked_sub(ratio),
    }
    .ok_or(ResidualPlanError::Arithmetic)?;
    let value = reference
        .value()
        .checked_mul(factor)
        .ok_or(ResidualPlanError::Arithmetic)?;
    Price::new(value).map_err(|_| ResidualPlanError::InvalidPriceBound)
}
