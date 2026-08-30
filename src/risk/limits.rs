//! Checked fixed-decimal helpers for P5 exposure and loss evaluation.

use rust_decimal::Decimal;

use crate::domain::{
    numeric::{Delta, Money, Notional},
    risk::{ExposureComponents, GlobalDeltaComponents},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RiskArithmeticError {
    Overflow,
}

pub(crate) fn exposure_total(
    components: ExposureComponents,
) -> Result<Notional, RiskArithmeticError> {
    let total = components
        .actual
        .value()
        .checked_add(components.reserved.value())
        .and_then(|value| value.checked_add(components.pending.value()))
        .ok_or(RiskArithmeticError::Overflow)?;
    Notional::new(total).map_err(|_| RiskArithmeticError::Overflow)
}

pub(crate) fn global_delta_total(
    components: GlobalDeltaComponents,
) -> Result<Delta, RiskArithmeticError> {
    let total = components
        .actual
        .value()
        .checked_add(components.reserved.value())
        .and_then(|value| value.checked_add(components.pending.value()))
        .ok_or(RiskArithmeticError::Overflow)?;
    Ok(Delta::new(total))
}

pub(crate) fn session_loss(pnl: Money) -> Result<Notional, RiskArithmeticError> {
    let value = if pnl.value() < Decimal::ZERO {
        Decimal::ZERO
            .checked_sub(pnl.value())
            .ok_or(RiskArithmeticError::Overflow)?
    } else {
        Decimal::ZERO
    };
    Notional::new(value).map_err(|_| RiskArithmeticError::Overflow)
}

pub(crate) fn absolute(value: Decimal) -> Result<Decimal, RiskArithmeticError> {
    if value < Decimal::ZERO {
        Decimal::ZERO
            .checked_sub(value)
            .ok_or(RiskArithmeticError::Overflow)
    } else {
        Ok(value)
    }
}

pub(crate) fn add_notional(
    current: Notional,
    change: Notional,
) -> Result<Notional, RiskArithmeticError> {
    let projected = current
        .value()
        .checked_add(change.value())
        .ok_or(RiskArithmeticError::Overflow)?;
    Notional::new(projected).map_err(|_| RiskArithmeticError::Overflow)
}

pub(crate) fn subtract_notional(
    current: Notional,
    change: Notional,
) -> Result<Notional, RiskArithmeticError> {
    let projected = current
        .value()
        .checked_sub(change.value())
        .ok_or(RiskArithmeticError::Overflow)?;
    Notional::new(projected).map_err(|_| RiskArithmeticError::Overflow)
}
