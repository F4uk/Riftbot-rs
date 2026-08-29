//! Pure P3 measurement and P4 target-inventory model boundaries.

/// Stable implementation version included in every P3 output and evidence artifact.
pub const P3_MODEL_VERSION: &str = "p3-measurement-v1";

/// Stable implementation version included in every P4 grid target.
pub const P4_MODEL_VERSION: &str = "p4-grid-inventory-v1";

pub mod fair_value;
pub mod grid_inventory;
pub mod opportunity;
pub mod regime;
pub mod spread_engine;
