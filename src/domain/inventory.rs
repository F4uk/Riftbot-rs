//! Target and actual inventory contracts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
