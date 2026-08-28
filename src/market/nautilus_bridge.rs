//! Narrow conversion boundary between Nautilus identifiers and pure domain identifiers.

use std::str::FromStr;

use nautilus_model::identifiers::{
    InstrumentId as NautilusInstrumentId, InstrumentIdError as NautilusInstrumentIdError,
};
use thiserror::Error;

use crate::domain::ids::InstrumentId;

/// Failed conversion at the Nautilus boundary.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NautilusBridgeError {
    /// The domain value did not satisfy Nautilus' instrument ID syntax.
    #[error("invalid Nautilus instrument ID: {0}")]
    InvalidInstrument(#[from] NautilusInstrumentIdError),
}

/// Parses a domain instrument identifier using the pinned Nautilus API.
pub fn to_nautilus_instrument_id(
    instrument_id: &InstrumentId,
) -> Result<NautilusInstrumentId, NautilusBridgeError> {
    NautilusInstrumentId::from_str(instrument_id.as_str()).map_err(Into::into)
}

/// Compile-only names for the official target adapter configuration APIs.
#[cfg(feature = "nautilus-adapters")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OfficialAdapterApi {
    pub hyperliquid_data_config: &'static str,
    pub hyperliquid_execution_config: &'static str,
    pub lighter_data_config: &'static str,
    pub lighter_execution_config: &'static str,
}

/// References public types from both official adapters without constructing clients or networking.
#[cfg(feature = "nautilus-adapters")]
#[must_use]
pub fn official_adapter_api() -> OfficialAdapterApi {
    use std::any::type_name;

    use nautilus_hyperliquid::{HyperliquidDataClientConfig, HyperliquidExecutionClientConfig};
    use nautilus_lighter::config::{LighterDataClientConfig, LighterExecutionClientConfig};

    OfficialAdapterApi {
        hyperliquid_data_config: type_name::<HyperliquidDataClientConfig>(),
        hyperliquid_execution_config: type_name::<HyperliquidExecutionClientConfig>(),
        lighter_data_config: type_name::<LighterDataClientConfig>(),
        lighter_execution_config: type_name::<LighterExecutionClientConfig>(),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::to_nautilus_instrument_id;
    use crate::domain::ids::InstrumentId;

    #[test]
    fn pinned_nautilus_instrument_parser_is_available() -> Result<(), Box<dyn Error>> {
        let domain_id = InstrumentId::try_from("BTC-PERP.HYPERLIQUID")?;
        let nautilus_id = to_nautilus_instrument_id(&domain_id)?;
        assert_eq!(nautilus_id.to_string(), domain_id.as_str());
        Ok(())
    }

    #[cfg(feature = "nautilus-adapters")]
    #[test]
    fn official_target_adapter_apis_compile() {
        let api = super::official_adapter_api();
        assert!(api.hyperliquid_data_config.contains("Hyperliquid"));
        assert!(api.lighter_data_config.contains("Lighter"));
    }
}
