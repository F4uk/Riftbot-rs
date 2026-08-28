//! Validated identifiers supplied by configuration, adapters, or deterministic replay.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_IDENTIFIER_BYTES: usize = 128;

/// Why a domain identifier was rejected.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum IdentifierError {
    /// The identifier was empty.
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },
    /// The identifier exceeded the defensive length bound.
    #[error("{kind} exceeds the maximum length of {maximum} bytes")]
    TooLong { kind: &'static str, maximum: usize },
    /// The identifier contained whitespace or a control character.
    #[error("{kind} must not contain whitespace or control characters")]
    InvalidCharacter { kind: &'static str },
}

fn validate_identifier(kind: &'static str, value: String) -> Result<String, IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty { kind });
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(IdentifierError::TooLong {
            kind,
            maximum: MAX_IDENTIFIER_BYTES,
        });
    }
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(IdentifierError::InvalidCharacter { kind });
    }
    Ok(value)
}

macro_rules! identifier {
    ($name:ident, $kind:literal) => {
        #[doc = concat!("Validated ", $kind, ".")]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdentifierError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::try_from(value.to_owned())
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                validate_identifier($kind, value).map(Self)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

identifier!(VenueId, "venue ID");
identifier!(InstrumentId, "instrument ID");
identifier!(Symbol, "symbol");
identifier!(PairId, "pair ID");
identifier!(DecisionId, "decision ID");
identifier!(IntentId, "intent ID");
identifier!(ModelVersion, "model version");
identifier!(ClientOrderId, "client order ID");
identifier!(VenueOrderId, "venue order ID");

#[cfg(test)]
mod tests {
    use super::{IdentifierError, VenueId};

    #[test]
    fn identifier_accepts_compact_text() -> Result<(), IdentifierError> {
        let venue = VenueId::try_from("trade_xyz")?;
        assert_eq!(venue.as_str(), "trade_xyz");
        Ok(())
    }

    #[test]
    fn identifier_rejects_whitespace() {
        let result = VenueId::try_from("trade xyz");
        assert!(matches!(
            result,
            Err(IdentifierError::InvalidCharacter { .. })
        ));
    }

    #[test]
    fn deserialization_runs_validation() {
        let result = serde_json::from_str::<VenueId>("\"\"");
        assert!(result.is_err());
    }
}
