//! Stable fingerprint for configuration which can change P3 measurement output.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    AppConfig, FairValueConfig, MarketDataConfig, PairConfig, RegimeConfig, RiskLimitsConfig,
    RouteFundingConfig, VenueConfig,
};

/// Lowercase SHA-256 of the canonical measurement-affecting configuration projection.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct MeasurementConfigFingerprint(String);

impl MeasurementConfigFingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MeasurementConfigFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Lowercase SHA-256 of the canonical P5 risk-policy configuration projection.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RiskConfigFingerprint(String);

impl RiskConfigFingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RiskConfigFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Serialize)]
struct MeasurementConfigProjection<'a> {
    config_schema_version: u16,
    pair: &'a Option<PairConfig>,
    venues: Vec<&'a VenueConfig>,
    market_data: &'a MarketDataConfig,
    fair_value: &'a FairValueConfig,
    funding_routes: Vec<&'a RouteFundingConfig>,
    regime: &'a RegimeConfig,
}

#[derive(Serialize)]
struct RiskConfigProjection<'a> {
    config_schema_version: u16,
    risk: &'a RiskLimitsConfig,
}

/// Hashes only fields which can affect P3 measurement output.
pub fn measurement_config_fingerprint(
    config: &AppConfig,
) -> Result<MeasurementConfigFingerprint, serde_json::Error> {
    let venues = match &config.pair {
        Some(pair) => pair
            .venues
            .iter()
            .filter_map(|venue_id| config.venues.iter().find(|venue| &venue.id == venue_id))
            .collect(),
        None => Vec::new(),
    };
    let projection = MeasurementConfigProjection {
        config_schema_version: config.schema_version,
        pair: &config.pair,
        venues,
        market_data: &config.market_data,
        fair_value: &config.fair_value,
        funding_routes: {
            let mut routes: Vec<_> = config.funding.routes.iter().collect();
            routes.sort_by(|left, right| {
                (&left.long_venue, &left.short_venue).cmp(&(&right.long_venue, &right.short_venue))
            });
            routes
        },
        regime: &config.regime,
    };
    let canonical = serde_json::to_vec(&projection)?;
    Ok(MeasurementConfigFingerprint(format!(
        "{:x}",
        Sha256::digest(canonical)
    )))
}

/// Hashes only fields which can affect P5 risk authorization.
pub fn risk_config_fingerprint(
    config: &AppConfig,
) -> Result<RiskConfigFingerprint, serde_json::Error> {
    let canonical = serde_json::to_vec(&RiskConfigProjection {
        config_schema_version: config.schema_version,
        risk: &config.risk,
    })?;
    Ok(RiskConfigFingerprint(format!(
        "{:x}",
        Sha256::digest(canonical)
    )))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{measurement_config_fingerprint, risk_config_fingerprint};
    use crate::{
        config::parse_toml,
        domain::numeric::{DurationMillis, Notional},
    };

    const EXAMPLE: &str = include_str!("../../config/example.toml");

    #[test]
    fn fingerprint_is_stable_and_measurement_sensitive() -> Result<(), Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let first = measurement_config_fingerprint(&config)?;
        let second = measurement_config_fingerprint(&config)?;
        assert_eq!(first, second);
        assert_eq!(first.as_str().len(), 64);

        let mut changed = config.clone();
        changed.market_data.execution_buffer_bps =
            crate::domain::numeric::Bps::new(Decimal::new(3, 0));
        assert_ne!(first, measurement_config_fingerprint(&changed)?);
        Ok(())
    }

    #[test]
    fn unrelated_risk_and_recording_fields_do_not_change_fingerprint() -> Result<(), Box<dyn Error>>
    {
        let config = parse_toml(EXAMPLE)?;
        let expected = measurement_config_fingerprint(&config)?;
        let mut changed = config;
        changed.risk.max_pair_notional = Notional::new(Decimal::new(2_000, 0))?;
        changed.recording.channel_capacity += 1;
        assert_eq!(expected, measurement_config_fingerprint(&changed)?);
        Ok(())
    }

    #[test]
    fn funding_route_order_does_not_change_fingerprint() -> Result<(), Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let expected = measurement_config_fingerprint(&config)?;
        let mut reordered = config;
        reordered.funding.routes.reverse();
        assert_eq!(expected, measurement_config_fingerprint(&reordered)?);
        Ok(())
    }

    #[test]
    fn risk_fingerprint_is_stable_and_risk_sensitive() -> Result<(), Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let expected = risk_config_fingerprint(&config)?;
        assert_eq!(expected, risk_config_fingerprint(&config)?);
        assert_eq!(expected.as_str().len(), 64);

        let mut changed = config;
        changed.risk.max_measurement_age_ms = DurationMillis(1_501);
        assert_ne!(expected, risk_config_fingerprint(&changed)?);
        Ok(())
    }
}
