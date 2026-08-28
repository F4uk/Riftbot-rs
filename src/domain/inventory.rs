//! Target and actual inventory contracts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    ids::{DecisionId, InstrumentId, ModelVersion, PairId, Symbol, VenueId},
    numeric::{Delta, Fraction, Money, Notional, PositionQty},
};

/// Explicit venue orientation for a target; `Flat` has no directional legs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TargetDirection {
    Flat,
    LongShort {
        long_venue: VenueId,
        short_venue: VenueId,
    },
}

/// The sole strategy output for desired arbitrage inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetInventory {
    pub symbol: Symbol,
    pub pair_id: PairId,
    pub target_fraction: Fraction,
    pub target_notional: Notional,
    pub direction: TargetDirection,
    pub reason: String,
    pub model_version: ModelVersion,
    pub decision_id: DecisionId,
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
