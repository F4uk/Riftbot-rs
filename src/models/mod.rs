//! Pure P3 measurement model boundaries.

/// Stable implementation version included in every P3 output and evidence artifact.
pub const P3_MODEL_VERSION: &str = "p3-measurement-v1";

pub mod fair_value;
pub mod grid_inventory;
pub mod opportunity;
pub mod regime;
pub mod spread_engine;
