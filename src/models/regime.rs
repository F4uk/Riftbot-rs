//! Deterministic market/system classification for Risk input; never a per-decision authorization.

use rust_decimal::Decimal;

use crate::{
    config::RegimeConfig,
    domain::{numeric::Bps, risk::Regime, spread::MeasurementValidity},
};

/// Measurement facts used only to classify the current market/system regime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegimeInput {
    pub market_validity: MeasurementValidity,
    pub deviation_bps: Option<Bps>,
    pub dispersion_bps: Option<Bps>,
}

/// Explainable threshold classifier. It has no timer and no stateful kill authority.
#[derive(Clone, Copy, Debug)]
pub struct RegimeFilter {
    degraded_dispersion_bps: Bps,
    reduce_only_deviation_bps: Bps,
    halted_deviation_bps: Bps,
}

impl RegimeFilter {
    #[must_use]
    pub const fn new(config: &RegimeConfig) -> Self {
        Self {
            degraded_dispersion_bps: config.degraded_dispersion_bps,
            reduce_only_deviation_bps: config.reduce_only_deviation_bps,
            halted_deviation_bps: config.halted_deviation_bps,
        }
    }

    #[must_use]
    pub fn classify(&self, input: RegimeInput) -> Regime {
        match input.market_validity {
            MeasurementValidity::VenueUnhealthy | MeasurementValidity::CrossedOrCorruptBook => {
                return Regime::Halted;
            }
            MeasurementValidity::WarmingUp
            | MeasurementValidity::StaleBook
            | MeasurementValidity::ReceiveTimeSkew
            | MeasurementValidity::EmptyBookSide
            | MeasurementValidity::InsufficientDepth
            | MeasurementValidity::InvalidFairValue => return Regime::ReduceOnly,
            MeasurementValidity::Valid
            | MeasurementValidity::FeeUnavailable
            | MeasurementValidity::FundingUnavailable => {}
        }

        let absolute_deviation = input
            .deviation_bps
            .map_or(Decimal::ZERO, |value| value.value().abs());
        if absolute_deviation >= self.halted_deviation_bps.value() {
            return Regime::Halted;
        }
        if absolute_deviation >= self.reduce_only_deviation_bps.value() {
            return Regime::ReduceOnly;
        }
        if input
            .dispersion_bps
            .is_some_and(|value| value.value().abs() >= self.degraded_dispersion_bps.value())
        {
            return Regime::Degraded;
        }
        Regime::Normal
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{RegimeFilter, RegimeInput};
    use crate::{
        config::RegimeConfig,
        domain::{numeric::Bps, risk::Regime, spread::MeasurementValidity},
    };

    fn filter() -> RegimeFilter {
        RegimeFilter::new(&RegimeConfig {
            degraded_dispersion_bps: Bps::new(Decimal::from(8)),
            reduce_only_deviation_bps: Bps::new(Decimal::from(50)),
            halted_deviation_bps: Bps::new(Decimal::from(100)),
        })
    }

    #[test]
    fn classifies_all_four_regimes() {
        let valid = |deviation, dispersion| RegimeInput {
            market_validity: MeasurementValidity::Valid,
            deviation_bps: Some(Bps::new(Decimal::from(deviation))),
            dispersion_bps: Some(Bps::new(Decimal::from(dispersion))),
        };
        assert_eq!(filter().classify(valid(10, 2)), Regime::Normal);
        assert_eq!(filter().classify(valid(10, 8)), Regime::Degraded);
        assert_eq!(filter().classify(valid(50, 2)), Regime::ReduceOnly);
        assert_eq!(filter().classify(valid(100, 2)), Regime::Halted);
    }

    #[test]
    fn hard_data_failures_are_more_restrictive_than_cost_unavailability() {
        let input = |market_validity| RegimeInput {
            market_validity,
            deviation_bps: Some(Bps::new(Decimal::ZERO)),
            dispersion_bps: Some(Bps::new(Decimal::ZERO)),
        };
        assert_eq!(
            filter().classify(input(MeasurementValidity::VenueUnhealthy)),
            Regime::Halted
        );
        assert_eq!(
            filter().classify(input(MeasurementValidity::StaleBook)),
            Regime::ReduceOnly
        );
        assert_eq!(
            filter().classify(input(MeasurementValidity::FundingUnavailable)),
            Regime::Normal
        );
    }
}
