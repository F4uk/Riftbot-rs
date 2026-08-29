//! Cross-field configuration invariants.

use std::collections::HashSet;

use rust_decimal::Decimal;
use thiserror::Error;

use super::schema::{AppConfig, FundingStateConfig};
use crate::domain::ids::VenueId;

/// Invalid configuration detected before any runtime component starts.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// TOML or typed-field deserialization failed.
    #[error("configuration parse failed: {0}")]
    Parse(#[from] toml::de::Error),
    /// Schema version is unsupported.
    #[error("unsupported configuration schema version {0}")]
    UnsupportedSchema(u16),
    /// Too few venues were enabled for cross-venue measurement.
    #[error("at least two venues must be enabled")]
    InsufficientEnabledVenues,
    /// A venue identifier appeared twice.
    #[error("duplicate venue ID: {0}")]
    DuplicateVenue(VenueId),
    /// A configured fee or execution-buffer assumption was negative.
    #[error("{field} must be non-negative")]
    NegativeBps { field: &'static str },
    /// An optional pair violated its two-venue shape.
    #[error("configured pair must contain two distinct enabled venues")]
    InvalidPairVenues,
    /// A fair-value logical-time schedule cannot satisfy warmup.
    #[error(
        "fair-value sampling durations must be positive and minimum samples must fit the window"
    )]
    InvalidFairValueWindow,
    /// Route funding state/value shape or pair orientation was invalid.
    #[error("funding routes must exactly cover both pair orientations and match state/value rules")]
    InvalidFundingRoutes,
    /// Measurement regime thresholds were negative or non-monotonic.
    #[error("regime thresholds must be non-negative with reduce-only below halted")]
    InvalidRegimeThresholds,
    /// Grid levels were empty or non-monotonic.
    #[error(
        "grid levels must have strictly increasing non-negative deviations and target fractions"
    )]
    InvalidGrid,
    /// A hard limit was zero or negative.
    #[error("{field} must be positive")]
    NonPositiveLimit { field: &'static str },
    /// Strategy target capacity consumed the hard pair limit and left no risk headroom.
    #[error("strategy.max_target_notional must be less than risk.max_pair_notional")]
    InvalidStrategyRiskHeadroom,
    /// Execution safety limits were invalid.
    #[error("execution limits and expiry must be non-negative, with a non-zero expiry")]
    InvalidExecutionLimits,
    /// Recording cannot be configured without a path and buffer.
    #[error("recording directory must be non-empty and channel capacity must be positive")]
    InvalidRecording,
}

impl AppConfig {
    /// Validates invariants that involve multiple typed fields.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 2 {
            return Err(ConfigError::UnsupportedSchema(self.schema_version));
        }

        let mut ids = HashSet::with_capacity(self.venues.len());
        for venue in &self.venues {
            if !ids.insert(&venue.id) {
                return Err(ConfigError::DuplicateVenue(venue.id.clone()));
            }
            if venue
                .taker_fee_bps
                .is_some_and(|fee| fee.value() < Decimal::ZERO)
            {
                return Err(ConfigError::NegativeBps {
                    field: "venues.taker_fee_bps",
                });
            }
        }

        let enabled: HashSet<&VenueId> = self
            .venues
            .iter()
            .filter(|venue| venue.enabled)
            .map(|venue| &venue.id)
            .collect();
        if enabled.len() < 2 {
            return Err(ConfigError::InsufficientEnabledVenues);
        }

        if let Some(pair) = &self.pair
            && (pair.venues[0] == pair.venues[1]
                || pair.instruments[0] == pair.instruments[1]
                || !pair.venues.iter().all(|venue| enabled.contains(venue)))
        {
            return Err(ConfigError::InvalidPairVenues);
        }

        if self.market_data.execution_buffer_bps.value() < Decimal::ZERO {
            return Err(ConfigError::NegativeBps {
                field: "market_data.execution_buffer_bps",
            });
        }
        if self.market_data.stale_after_ms.0 == 0 {
            return Err(ConfigError::NonPositiveLimit {
                field: "market_data.stale_after_ms",
            });
        }
        if self.market_data.max_receive_skew_ms.0 == 0 {
            return Err(ConfigError::NonPositiveLimit {
                field: "market_data.max_receive_skew_ms",
            });
        }

        let sample_interval_ms = self.fair_value.sample_interval_ms.0;
        let window_duration_ms = self.fair_value.window_duration_ms.0;
        let minimum_samples = u64::try_from(self.fair_value.minimum_samples).unwrap_or(u64::MAX);
        if sample_interval_ms == 0
            || window_duration_ms == 0
            || self.fair_value.max_sample_age_ms.0 == 0
            || minimum_samples == 0
            || minimum_samples > window_duration_ms / sample_interval_ms
            || sample_interval_ms.checked_mul(1_000_000).is_none()
            || window_duration_ms.checked_mul(1_000_000).is_none()
        {
            return Err(ConfigError::InvalidFairValueWindow);
        }

        validate_grid(self)?;
        validate_funding(self)?;
        validate_regime(self)?;
        validate_positive_limits(self)?;

        if self.strategy.max_target_notional.value() >= self.risk.max_pair_notional.value() {
            return Err(ConfigError::InvalidStrategyRiskHeadroom);
        }

        if self.execution.max_residual_delta.value() < Decimal::ZERO
            || self.execution.max_slippage_bps.value() < Decimal::ZERO
            || self.execution.intent_expiry_ms.0 == 0
        {
            return Err(ConfigError::InvalidExecutionLimits);
        }

        if self.recording.directory.as_os_str().is_empty() || self.recording.channel_capacity == 0 {
            return Err(ConfigError::InvalidRecording);
        }

        Ok(())
    }
}

fn validate_grid(config: &AppConfig) -> Result<(), ConfigError> {
    if config.grid.levels.is_empty() {
        return Err(ConfigError::InvalidGrid);
    }

    let mut previous_deviation = Decimal::ZERO;
    let mut previous_target = Decimal::ZERO;
    for level in &config.grid.levels {
        let deviation = level.deviation_bps.value();
        let target = level.target_fraction.value();
        if deviation <= previous_deviation || target <= previous_target || target < Decimal::ZERO {
            return Err(ConfigError::InvalidGrid);
        }
        previous_deviation = deviation;
        previous_target = target;
    }
    Ok(())
}

fn validate_funding(config: &AppConfig) -> Result<(), ConfigError> {
    let Some(pair) = &config.pair else {
        return if config.funding.routes.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::InvalidFundingRoutes)
        };
    };
    if config.funding.routes.len() != 2 {
        return Err(ConfigError::InvalidFundingRoutes);
    }
    let expected = [
        (&pair.venues[0], &pair.venues[1]),
        (&pair.venues[1], &pair.venues[0]),
    ];
    for (long_venue, short_venue) in expected {
        let Some(route) = config
            .funding
            .routes
            .iter()
            .find(|route| &route.long_venue == long_venue && &route.short_venue == short_venue)
        else {
            return Err(ConfigError::InvalidFundingRoutes);
        };
        let valid_shape = match route.state {
            FundingStateConfig::Available => route.adjustment_bps.is_some(),
            FundingStateConfig::Unavailable | FundingStateConfig::Disabled => {
                route.adjustment_bps.is_none()
            }
        };
        if !valid_shape {
            return Err(ConfigError::InvalidFundingRoutes);
        }
    }
    Ok(())
}

fn validate_regime(config: &AppConfig) -> Result<(), ConfigError> {
    let degraded = config.regime.degraded_dispersion_bps.value();
    let reduce_only = config.regime.reduce_only_deviation_bps.value();
    let halted = config.regime.halted_deviation_bps.value();
    if degraded < Decimal::ZERO
        || reduce_only < Decimal::ZERO
        || halted < Decimal::ZERO
        || reduce_only >= halted
    {
        return Err(ConfigError::InvalidRegimeThresholds);
    }
    Ok(())
}

fn validate_positive_limits(config: &AppConfig) -> Result<(), ConfigError> {
    let limits = [
        (
            "market_data.minimum_depth_notional",
            config.market_data.minimum_depth_notional.value(),
        ),
        (
            "strategy.max_target_notional",
            config.strategy.max_target_notional.value(),
        ),
        (
            "risk.max_venue_notional",
            config.risk.max_venue_notional.value(),
        ),
        (
            "risk.max_pair_notional",
            config.risk.max_pair_notional.value(),
        ),
        (
            "risk.max_session_loss",
            config.risk.max_session_loss.value(),
        ),
    ];

    for (field, value) in limits {
        if value <= Decimal::ZERO {
            return Err(ConfigError::NonPositiveLimit { field });
        }
    }
    if config.risk.max_global_delta.value() <= Decimal::ZERO {
        return Err(ConfigError::NonPositiveLimit {
            field: "risk.max_global_delta",
        });
    }
    Ok(())
}
