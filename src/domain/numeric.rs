//! Fixed-decimal units used instead of untyped floating-point values.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Invalid construction of a typed numeric value.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NumericError {
    /// A value that must be strictly positive was zero or negative.
    #[error("{unit} must be positive, received {value}")]
    MustBePositive { unit: &'static str, value: Decimal },
    /// A value that must be non-negative was negative.
    #[error("{unit} must be non-negative, received {value}")]
    MustBeNonNegative { unit: &'static str, value: Decimal },
    /// A fraction was outside the inclusive range from -1 to 1.
    #[error("fraction must be between -1 and 1, received {value}")]
    FractionOutOfRange { value: Decimal },
}

macro_rules! signed_decimal_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        #[serde(transparent)]
        pub struct $name(Decimal);

        impl $name {
            /// Creates a value from its fixed-decimal representation.
            #[must_use]
            pub const fn new(value: Decimal) -> Self {
                Self(value)
            }

            /// Returns the underlying fixed-decimal value.
            #[must_use]
            pub const fn value(self) -> Decimal {
                self.0
            }
        }
    };
}

signed_decimal_newtype!(Bps, "Basis points; may be positive, zero, or negative.");
signed_decimal_newtype!(Delta, "Signed USD delta.");
signed_decimal_newtype!(Money, "Signed monetary amount.");
signed_decimal_newtype!(PositionQty, "Signed base-asset position quantity.");

/// Strictly positive price.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "Decimal")]
pub struct Price(Decimal);

impl Price {
    /// Creates a strictly positive price.
    pub fn new(value: Decimal) -> Result<Self, NumericError> {
        if value <= Decimal::ZERO {
            return Err(NumericError::MustBePositive {
                unit: "price",
                value,
            });
        }
        Ok(Self(value))
    }

    /// Returns the underlying fixed-decimal price.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Price {
    type Error = NumericError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Strictly positive base-asset quantity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "Decimal")]
pub struct BaseQty(Decimal);

impl BaseQty {
    /// Creates a strictly positive quantity.
    pub fn new(value: Decimal) -> Result<Self, NumericError> {
        if value <= Decimal::ZERO {
            return Err(NumericError::MustBePositive {
                unit: "base quantity",
                value,
            });
        }
        Ok(Self(value))
    }

    /// Returns the underlying fixed-decimal quantity.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for BaseQty {
    type Error = NumericError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Non-negative quote notional.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "Decimal")]
pub struct Notional(Decimal);

impl Notional {
    /// Creates a non-negative notional.
    pub fn new(value: Decimal) -> Result<Self, NumericError> {
        if value < Decimal::ZERO {
            return Err(NumericError::MustBeNonNegative {
                unit: "notional",
                value,
            });
        }
        Ok(Self(value))
    }

    /// Returns the underlying fixed-decimal notional.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Notional {
    type Error = NumericError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Signed target fraction in the inclusive range from -1 to 1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "Decimal")]
pub struct Fraction(Decimal);

impl Fraction {
    /// Creates a bounded target fraction.
    pub fn new(value: Decimal) -> Result<Self, NumericError> {
        if value < Decimal::NEGATIVE_ONE || value > Decimal::ONE {
            return Err(NumericError::FractionOutOfRange { value });
        }
        Ok(Self(value))
    }

    /// Returns the underlying fixed-decimal fraction.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Fraction {
    type Error = NumericError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Explicit Unix timestamp in nanoseconds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct UnixNanos(pub u64);

/// Explicit duration in milliseconds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DurationMillis(pub u64);

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{Fraction, NumericError, Price};

    #[test]
    fn price_rejects_zero() {
        assert!(matches!(
            Price::new(Decimal::ZERO),
            Err(NumericError::MustBePositive { .. })
        ));
    }

    #[test]
    fn fraction_enforces_unit_interval() {
        assert!(Fraction::new(Decimal::NEGATIVE_ONE).is_ok());
        assert!(Fraction::new(Decimal::ONE).is_ok());
        assert!(matches!(
            Fraction::new(Decimal::new(101, 2)),
            Err(NumericError::FractionOutOfRange { .. })
        ));
    }
}
