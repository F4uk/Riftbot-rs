//! P0 foundation for Riftbot.
//!
//! Business behavior is introduced stage-by-stage. P0 exposes validated contracts and module
//! boundaries only; it cannot connect to venues or submit orders.

#![deny(unsafe_code)]

pub mod app;
pub mod config;
pub mod domain;
pub mod execution;
pub mod market;
pub mod models;
pub mod pnl;
pub mod reconciliation;
pub mod recording;
pub mod risk;
pub mod strategy;
