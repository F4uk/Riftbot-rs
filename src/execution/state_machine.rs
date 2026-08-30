//! Explicit validated child-order and execution-basket state contracts.

use std::collections::HashSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{
    execution_intent::{ExecutionLeg, OrderSide, PriceGuard, validate_leg},
    ids::{ClientOrderId, CommandId, FillId, IntentId, VenueOrderId},
    numeric::{BaseQty, Delta, Notional, Price, UnixNanos},
    risk::RiskDecision,
};

/// Authoritative lifecycle state for one concrete child generation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildOrderState {
    NotSent,
    Submitting,
    AcceptedOpen,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Unknown,
}

impl ChildOrderState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Filled | Self::Canceled | Self::Rejected)
    }
}

/// Cancel has its own lifecycle; requesting it does not make the order canceled.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelState {
    NotRequested,
    Requested,
    Confirmed,
    Rejected,
    Unknown,
}

/// Frozen P6 basket states.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BasketState {
    Planned,
    Submitting,
    Pending,
    Partial,
    Imbalanced,
    Hedging,
    Balanced,
    Complete,
    Unknown,
    Reconciling,
    Aborting,
    FailedSafe,
}

/// One idempotent fill fact retained by a child order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedFill {
    pub fill_id: FillId,
    pub cumulative_filled_qty: BaseQty,
    pub cumulative_filled_notional: Notional,
    pub average_fill_price: Price,
    pub cumulative_fees: Option<Notional>,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
}

/// Auditable state of one stable child order identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildOrderSnapshot {
    pub intent_id: IntentId,
    pub leg_index: usize,
    pub generation: u32,
    pub command_id: CommandId,
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub leg: ExecutionLeg,
    pub state: ChildOrderState,
    pub cancel_state: CancelState,
    pub cumulative_filled_qty: Decimal,
    pub cumulative_filled_notional: Decimal,
    pub average_fill_price: Option<Price>,
    pub cumulative_fees: Option<Notional>,
    pub last_exchange_ts: Option<UnixNanos>,
    pub last_receive_ts: Option<UnixNanos>,
    pub fills: Vec<AppliedFill>,
    pub recovery: bool,
}

impl ChildOrderSnapshot {
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionBasketSnapshotParams {
    intent_id: IntentId,
    initial_leg_count: usize,
    state: BasketState,
    children: Vec<ChildOrderSnapshot>,
    starting_residual: Delta,
    residual: Delta,
    max_residual_delta: Delta,
    recovery_attempts: u32,
    max_recovery_attempts: u32,
    required_authority: Option<RiskDecision>,
    terminal_reason: Option<String>,
}

/// Serde-hardened externally inspectable execution state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "ExecutionBasketSnapshotParams")]
pub struct ExecutionBasketSnapshot {
    intent_id: IntentId,
    initial_leg_count: usize,
    state: BasketState,
    children: Vec<ChildOrderSnapshot>,
    starting_residual: Delta,
    residual: Delta,
    max_residual_delta: Delta,
    recovery_attempts: u32,
    max_recovery_attempts: u32,
    required_authority: Option<RiskDecision>,
    terminal_reason: Option<String>,
}

pub(crate) struct ExecutionBasketSnapshotParts {
    pub intent_id: IntentId,
    pub initial_leg_count: usize,
    pub state: BasketState,
    pub children: Vec<ChildOrderSnapshot>,
    pub starting_residual: Delta,
    pub residual: Delta,
    pub max_residual_delta: Delta,
    pub recovery_attempts: u32,
    pub max_recovery_attempts: u32,
    pub required_authority: Option<RiskDecision>,
    pub terminal_reason: Option<String>,
}

impl ExecutionBasketSnapshot {
    pub(crate) fn from_parts(
        parts: ExecutionBasketSnapshotParts,
    ) -> Result<Self, ExecutionStateError> {
        Self::try_from(ExecutionBasketSnapshotParams {
            intent_id: parts.intent_id,
            initial_leg_count: parts.initial_leg_count,
            state: parts.state,
            children: parts.children,
            starting_residual: parts.starting_residual,
            residual: parts.residual,
            max_residual_delta: parts.max_residual_delta,
            recovery_attempts: parts.recovery_attempts,
            max_recovery_attempts: parts.max_recovery_attempts,
            required_authority: parts.required_authority,
            terminal_reason: parts.terminal_reason,
        })
    }

    #[must_use]
    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    #[must_use]
    pub const fn state(&self) -> BasketState {
        self.state
    }

    #[must_use]
    pub fn children(&self) -> &[ChildOrderSnapshot] {
        &self.children
    }

    #[must_use]
    pub const fn starting_residual(&self) -> Delta {
        self.starting_residual
    }

    #[must_use]
    pub const fn residual(&self) -> Delta {
        self.residual
    }

    #[must_use]
    pub const fn recovery_attempts(&self) -> u32 {
        self.recovery_attempts
    }

    #[must_use]
    pub const fn required_authority(&self) -> Option<RiskDecision> {
        self.required_authority
    }

    #[must_use]
    pub fn terminal_reason(&self) -> Option<&str> {
        self.terminal_reason.as_deref()
    }
}

impl TryFrom<ExecutionBasketSnapshotParams> for ExecutionBasketSnapshot {
    type Error = ExecutionStateError;

    fn try_from(params: ExecutionBasketSnapshotParams) -> Result<Self, Self::Error> {
        validate_snapshot(&params)?;
        Ok(Self {
            intent_id: params.intent_id,
            initial_leg_count: params.initial_leg_count,
            state: params.state,
            children: params.children,
            starting_residual: params.starting_residual,
            residual: params.residual,
            max_residual_delta: params.max_residual_delta,
            recovery_attempts: params.recovery_attempts,
            max_recovery_attempts: params.max_recovery_attempts,
            required_authority: params.required_authority,
            terminal_reason: params.terminal_reason,
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExecutionStateError {
    #[error("execution basket snapshot is internally inconsistent")]
    InvalidSnapshot,
    #[error("checked residual arithmetic failed")]
    Arithmetic,
}

fn validate_snapshot(params: &ExecutionBasketSnapshotParams) -> Result<(), ExecutionStateError> {
    if params.max_residual_delta.value() < Decimal::ZERO
        || !matches!(params.initial_leg_count, 1 | 2)
        || (params.initial_leg_count == 2 && params.starting_residual.value() != Decimal::ZERO)
        || (params.initial_leg_count == 1
            && (params.starting_residual.value() == Decimal::ZERO || params.recovery_attempts != 0))
        || params.max_recovery_attempts == 0
        || params.recovery_attempts > params.max_recovery_attempts
    {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    let mut child_ids = HashSet::with_capacity(params.children.len());
    let mut fill_ids = HashSet::new();
    for (expected_index, child) in params.children.iter().enumerate() {
        let expected_generation = if expected_index < params.initial_leg_count {
            0
        } else {
            u32::try_from(expected_index - params.initial_leg_count + 1)
                .map_err(|_| ExecutionStateError::InvalidSnapshot)?
        };
        let expected_tag = if expected_index < params.initial_leg_count {
            if params.initial_leg_count == 1 {
                "emergency"
            } else {
                "initial"
            }
        } else {
            "hedge"
        };
        let expected_client_order_id = format!(
            "{}-{expected_tag}-l{expected_index}-g{expected_generation}",
            params.intent_id.as_str()
        );
        let expected_command_id = format!("{expected_client_order_id}-submit");
        if child.intent_id != params.intent_id
            || child.leg_index != expected_index
            || child.generation != expected_generation
            || child.client_order_id.as_str() != expected_client_order_id
            || child.command_id.as_str() != expected_command_id
            || validate_leg(&child.leg).is_err()
            || child.state == ChildOrderState::NotSent
            || (expected_index < params.initial_leg_count
                && (child.recovery || child.generation != 0))
            || (expected_index >= params.initial_leg_count
                && (!child.recovery || child.generation == 0))
            || !child_ids.insert(&child.client_order_id)
            || child.cumulative_filled_qty < Decimal::ZERO
            || child.cumulative_filled_notional < Decimal::ZERO
            || child.cumulative_filled_qty > child.leg.target_qty.value()
            || (child.cumulative_filled_qty == Decimal::ZERO) != child.average_fill_price.is_none()
            || (child.state == ChildOrderState::Filled
                && child.cumulative_filled_qty != child.leg.target_qty.value())
            || (child.state == ChildOrderState::PartiallyFilled
                && (child.cumulative_filled_qty <= Decimal::ZERO
                    || child.cumulative_filled_qty >= child.leg.target_qty.value()))
            || (child.cancel_state == CancelState::Confirmed
                && !matches!(
                    child.state,
                    ChildOrderState::Canceled | ChildOrderState::Filled
                ))
            || (child.state == ChildOrderState::Canceled
                && child.cancel_state != CancelState::Confirmed)
            || (child.state == ChildOrderState::Rejected
                && child.cumulative_filled_qty > Decimal::ZERO)
        {
            return Err(ExecutionStateError::InvalidSnapshot);
        }
        let mut previous_qty = Decimal::ZERO;
        let mut previous_notional = Decimal::ZERO;
        for fill in &child.fills {
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
            if !fill_ids.insert(&fill.fill_id)
                || !within_guard
                || fill.cumulative_filled_qty.value() < previous_qty
                || fill.cumulative_filled_notional.value() < previous_notional
                || fill.cumulative_filled_qty.value() > child.leg.target_qty.value()
                || fill
                    .cumulative_filled_qty
                    .value()
                    .checked_mul(fill.average_fill_price.value())
                    != Some(fill.cumulative_filled_notional.value())
            {
                return Err(ExecutionStateError::InvalidSnapshot);
            }
            previous_qty = fill.cumulative_filled_qty.value();
            previous_notional = fill.cumulative_filled_notional.value();
        }
        if child.fills.last().is_some_and(|fill| {
            fill.cumulative_filled_qty.value() != child.cumulative_filled_qty
                || fill.cumulative_filled_notional.value() != child.cumulative_filled_notional
                || Some(fill.average_fill_price) != child.average_fill_price
        }) || (child.fills.is_empty() && child.cumulative_filled_qty != Decimal::ZERO)
        {
            return Err(ExecutionStateError::InvalidSnapshot);
        }
    }
    let residual = residual_from_children(params.starting_residual, &params.children)?;
    if residual != params.residual {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    let has_unknown = params.children.iter().any(|child| {
        child.state == ChildOrderState::Unknown || child.cancel_state == CancelState::Unknown
    });
    let residual_within = absolute(residual.value())? <= params.max_residual_delta.value();
    let all_terminal = params.children.iter().all(ChildOrderSnapshot::is_terminal);
    let has_fill = params
        .children
        .iter()
        .any(|child| child.cumulative_filled_notional > Decimal::ZERO);
    let active_recovery = params
        .children
        .iter()
        .skip(params.initial_leg_count)
        .any(|child| !child.is_terminal());
    let cancel_requested = params
        .children
        .iter()
        .any(|child| child.cancel_state == CancelState::Requested);
    let initial_aborted = params
        .children
        .iter()
        .take(params.initial_leg_count)
        .any(|child| {
            matches!(
                child.state,
                ChildOrderState::Rejected | ChildOrderState::Canceled
            )
        });
    if params
        .children
        .len()
        .saturating_sub(params.initial_leg_count)
        != params.recovery_attempts as usize
        || (params.state == BasketState::Planned && !params.children.is_empty())
        || (params.state != BasketState::Planned
            && params.children.len() < params.initial_leg_count)
        || (params.state == BasketState::Submitting
            && (cancel_requested
                || has_fill
                || !params
                    .children
                    .iter()
                    .any(|child| child.state == ChildOrderState::Submitting)))
        || (params.state == BasketState::Pending
            && (cancel_requested || has_fill || has_unknown || all_terminal || active_recovery))
        || (params.state == BasketState::Partial
            && (!has_fill
                || (params.starting_residual.value() == Decimal::ZERO && !residual_within)
                || all_terminal
                || has_unknown
                || active_recovery
                || initial_aborted
                || cancel_requested))
        || (params.state == BasketState::Imbalanced
            && (residual_within || has_unknown || active_recovery))
        || (params.state == BasketState::Hedging && !active_recovery)
        || (params.state == BasketState::Balanced
            && (!residual_within || has_unknown || !all_terminal))
        || (params.state == BasketState::Aborting && (!initial_aborted && !cancel_requested))
    {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    if params.state == BasketState::Complete
        && (has_unknown
            || !residual_within
            || !all_terminal
            || params.terminal_reason.as_deref().is_none_or(str::is_empty))
    {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    if matches!(
        params.state,
        BasketState::Unknown | BasketState::Reconciling
    ) && !has_unknown
    {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    if params.state == BasketState::Balanced && !residual_within {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    if params.state == BasketState::FailedSafe
        && (params.required_authority.is_none()
            || params.terminal_reason.as_deref().is_none_or(str::is_empty))
    {
        return Err(ExecutionStateError::InvalidSnapshot);
    }
    Ok(())
}

pub(crate) fn residual_from_children(
    starting_residual: Delta,
    children: &[ChildOrderSnapshot],
) -> Result<Delta, ExecutionStateError> {
    let value = children
        .iter()
        .try_fold(starting_residual.value(), |total, child| {
            let signed = match child.leg.side {
                OrderSide::Buy => Some(child.cumulative_filled_notional),
                OrderSide::Sell => Decimal::ZERO.checked_sub(child.cumulative_filled_notional),
            }
            .ok_or(ExecutionStateError::Arithmetic)?;
            total
                .checked_add(signed)
                .ok_or(ExecutionStateError::Arithmetic)
        })?;
    Ok(Delta::new(value))
}

pub(crate) fn absolute(value: Decimal) -> Result<Decimal, ExecutionStateError> {
    if value < Decimal::ZERO {
        Decimal::ZERO
            .checked_sub(value)
            .ok_or(ExecutionStateError::Arithmetic)
    } else {
        Ok(value)
    }
}
