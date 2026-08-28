//! Typed, secret-free application configuration.

mod schema;
mod validation;

pub use schema::{
    AppConfig, ExecutionConfig, FairValueConfig, GridConfig, GridLevelConfig, MarketDataConfig,
    PairConfig, RecordingConfig, RiskLimitsConfig, RuntimeConfig, RuntimeMode, VenueConfig,
    VenueKind,
};
pub use validation::ConfigError;

/// Parses TOML and runs all cross-field validation.
pub fn parse_toml(contents: &str) -> Result<AppConfig, ConfigError> {
    let config: AppConfig = toml::from_str(contents)?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, parse_toml};

    const EXAMPLE: &str = include_str!("../../config/example.toml");

    #[test]
    fn example_configuration_is_valid() -> Result<(), ConfigError> {
        let config = parse_toml(EXAMPLE)?;
        assert_eq!(config.venues.len(), 3);
        Ok(())
    }

    #[test]
    fn secret_shaped_unknown_fields_are_rejected() {
        let with_secret_field = EXAMPLE.replacen(
            "mode = \"signal_only\"",
            "mode = \"signal_only\"\napi_key = \"must-not-be-accepted\"",
            1,
        );
        assert!(matches!(
            parse_toml(&with_secret_field),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn duplicate_venue_is_rejected() -> Result<(), ConfigError> {
        let mut config = parse_toml(EXAMPLE)?;
        config.venues.push(config.venues[0].clone());
        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateVenue(_))
        ));
        Ok(())
    }
}
