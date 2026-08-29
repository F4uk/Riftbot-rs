//! Target and actual inventory contracts.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    ids::{DecisionId, InstrumentId, ModelVersion, PairId, Symbol, VenueId},
    numeric::{Delta, Money, Notional, PositionQty, TargetFraction},
};

/// Explicit venue orientation for a target; `Flat` has no directional legs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetDirection {
    Flat,
    LongShort {
        long_venue: VenueId,
        short_venue: VenueId,
    },
}

impl TargetDirection {
    #[must_use]
    pub fn matches_route(&self, long_venue: &VenueId, short_venue: &VenueId) -> bool {
        matches!(
            self,
            Self::LongShort {
                long_venue: target_long,
                short_venue: target_short,
            } if target_long == long_venue && target_short == short_venue
        )
    }

    #[must_use]
    pub const fn is_flat(&self) -> bool {
        matches!(self, Self::Flat)
    }
}

/// The sole strategy output for desired arbitrage inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetInventory {
    pub symbol: Symbol,
    pub pair_id: PairId,
    pub target_fraction: TargetFraction,
    pub target_notional: Notional,
    pub direction: TargetDirection,
    pub reason: String,
    pub model_version: ModelVersion,
    pub decision_id: DecisionId,
}

/// Actual and not-yet-settled matched exposure for one explicit route, all measured per leg.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrientedExposure {
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub actual_notional_per_leg: Notional,
    pub reserved_notional_per_leg: Notional,
    pub pending_notional_per_leg: Notional,
}

impl OrientedExposure {
    pub fn new(
        long_venue: VenueId,
        short_venue: VenueId,
        actual_notional_per_leg: Notional,
        reserved_notional_per_leg: Notional,
        pending_notional_per_leg: Notional,
    ) -> Result<Self, InventoryDomainError> {
        if long_venue == short_venue {
            return Err(InventoryDomainError::SameVenue);
        }
        Ok(Self {
            long_venue,
            short_venue,
            actual_notional_per_leg,
            reserved_notional_per_leg,
            pending_notional_per_leg,
        })
    }

    pub fn effective_notional_per_leg(&self) -> Result<Notional, InventoryDomainError> {
        let value = self
            .actual_notional_per_leg
            .value()
            .checked_add(self.reserved_notional_per_leg.value())
            .and_then(|total| total.checked_add(self.pending_notional_per_leg.value()))
            .ok_or(InventoryDomainError::Arithmetic)?;
        Notional::new(value).map_err(InventoryDomainError::from)
    }

    #[must_use]
    pub fn direction(&self) -> TargetDirection {
        TargetDirection::LongShort {
            long_venue: self.long_venue.clone(),
            short_venue: self.short_venue.clone(),
        }
    }
}

/// Pair-level state used by P4. Reserved and pending exposure are first-class, not annotations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveInventory {
    pub symbol: Symbol,
    pub pair_id: PairId,
    pub exposures: Vec<OrientedExposure>,
}

impl EffectiveInventory {
    pub fn new(
        symbol: Symbol,
        pair_id: PairId,
        exposures: Vec<OrientedExposure>,
    ) -> Result<Self, InventoryDomainError> {
        let inventory = Self {
            symbol,
            pair_id,
            exposures,
        };
        inventory.validate()?;
        Ok(inventory)
    }

    pub fn validate(&self) -> Result<(), InventoryDomainError> {
        for (index, exposure) in self.exposures.iter().enumerate() {
            if exposure.long_venue == exposure.short_venue {
                return Err(InventoryDomainError::SameVenue);
            }
            if self.exposures[..index].iter().any(|previous| {
                previous.long_venue == exposure.long_venue
                    && previous.short_venue == exposure.short_venue
            }) {
                return Err(InventoryDomainError::DuplicateOrientation);
            }
            exposure.effective_notional_per_leg()?;
        }
        Ok(())
    }

    pub fn project(
        &self,
        long_venue: &VenueId,
        short_venue: &VenueId,
    ) -> Result<EffectiveActual, InventoryDomainError> {
        if long_venue == short_venue {
            return Err(InventoryDomainError::SameVenue);
        }
        let exposure = self.exposures.iter().find(|exposure| {
            &exposure.long_venue == long_venue && &exposure.short_venue == short_venue
        });
        let (actual, reserved, pending) =
            exposure.map_or((Decimal::ZERO, Decimal::ZERO, Decimal::ZERO), |exposure| {
                (
                    exposure.actual_notional_per_leg.value(),
                    exposure.reserved_notional_per_leg.value(),
                    exposure.pending_notional_per_leg.value(),
                )
            });
        let total = actual
            .checked_add(reserved)
            .and_then(|value| value.checked_add(pending))
            .ok_or(InventoryDomainError::Arithmetic)?;
        Ok(EffectiveActual {
            direction: TargetDirection::LongShort {
                long_venue: long_venue.clone(),
                short_venue: short_venue.clone(),
            },
            actual_notional_per_leg: Notional::new(actual)?,
            reserved_notional_per_leg: Notional::new(reserved)?,
            pending_notional_per_leg: Notional::new(pending)?,
            total_notional_per_leg: Notional::new(total)?,
        })
    }
}

/// Comparable current exposure in the same direction and per-leg unit as a target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveActual {
    pub direction: TargetDirection,
    pub actual_notional_per_leg: Notional,
    pub reserved_notional_per_leg: Notional,
    pub pending_notional_per_leg: Notional,
    pub total_notional_per_leg: Notional,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum InventoryDomainError {
    #[error("inventory route venues must be distinct")]
    SameVenue,
    #[error("effective inventory contains a duplicate route orientation")]
    DuplicateOrientation,
    #[error("fixed-decimal effective inventory arithmetic failed")]
    Arithmetic,
    #[error(transparent)]
    Numeric(#[from] super::numeric::NumericError),
}

/// One venue's position view under a symbol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VenuePosition {
    pub instrument_id: InstrumentId,
    pub quantity: PositionQty,
    pub marked_value: Money,
}

/// Global-inventory-shaped state; V1 consumes a two-venue view of this map.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GlobalInventory {
    pub symbol: Symbol,
    pub positions: BTreeMap<VenueId, VenuePosition>,
    pub net_delta: Delta,
}

#[cfg(test)]
mod tests {
    use super::TargetInventory;

    #[test]
    fn target_inventory_deserialization_rejects_negative_fraction() {
        let json = r#"{
            "symbol":"SNDK",
            "pair_id":"SNDK-ENTROPY-LIGHTER",
            "target_fraction":"-0.10",
            "target_notional":"100.00",
            "direction":{
                "type":"long_short",
                "long_venue":"entropy",
                "short_venue":"lighter"
            },
            "reason":"invalid negative target",
            "model_version":"p4-grid-inventory-v1",
            "decision_id":"decision-1"
        }"#;
        assert!(serde_json::from_str::<TargetInventory>(json).is_err());
    }
}
