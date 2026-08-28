//! Auditable decision record schema. P2 will add versioned persistence and replay behavior.

use serde::{Deserialize, Serialize};

use super::{
    execution_intent::ExecutionIntent,
    ids::{ClientOrderId, DecisionId, InstrumentId, IntentId, VenueOrderId},
    inventory::{GlobalInventory, TargetInventory},
    market::VenueBook,
    numeric::{BaseQty, Delta, DurationMillis, Money, Price, UnixNanos},
    risk::{Regime, RiskAssessment},
    spread::SpreadSnapshot,
};

/// Order lifecycle fact associated with an intent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderEventKind {
    Submitted,
    Accepted,
    Rejected,
    Canceled,
    Unknown,
}

/// Minimal order event record for stage-independent auditability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrderRecord {
    pub intent_id: IntentId,
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub instrument_id: InstrumentId,
    pub event: OrderEventKind,
    pub timestamp: UnixNanos,
    pub reason: Option<String>,
}

/// Minimal fill record associated with its original intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FillRecord {
    pub intent_id: IntentId,
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub instrument_id: InstrumentId,
    pub quantity: BaseQty,
    pub price: Price,
    pub timestamp: UnixNanos,
}

/// Optional latency components; absence means unknown, never an invented zero.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LatencySnapshot {
    pub event: Option<DurationMillis>,
    pub decision: Option<DurationMillis>,
    pub submit: Option<DurationMillis>,
    pub fill: Option<DurationMillis>,
}

/// PnL components at record time. `None` explicitly means unavailable.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PnlAttributionSnapshot {
    pub gross_spread: Option<Money>,
    pub fees: Option<Money>,
    pub slippage: Option<Money>,
    pub funding: Option<Money>,
    pub emergency_hedge_cost: Option<Money>,
    pub inventory_mark: Option<Money>,
    pub net: Option<Money>,
}

/// Frozen explanation of one decision and its downstream facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionRecord {
    pub decision_id: DecisionId,
    pub timestamp: UnixNanos,
    pub input_books: Vec<VenueBook>,
    pub spread: Option<SpreadSnapshot>,
    pub regime: Regime,
    pub current_inventory: GlobalInventory,
    pub target_inventory: Option<TargetInventory>,
    pub risk: RiskAssessment,
    pub execution_intent: Option<ExecutionIntent>,
    pub orders: Vec<OrderRecord>,
    pub fills: Vec<FillRecord>,
    pub latency: LatencySnapshot,
    pub residual_delta: Delta,
    pub pnl: PnlAttributionSnapshot,
}
