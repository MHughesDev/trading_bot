//! Many paper deployments, bounded (Set T T-2.1; SPEC capacity, COMP-002,
//! ADR-P6-08).
//!
//! A fleet is what makes Gate 15 evidence arrive at a useful rate, and it is also
//! how the platform grows into a crowd it never measured. Three ceilings bound it,
//! and each is checked at **proposal-decision time**, so a deployment that would
//! break one is refused before it exists, with the binding ceiling named:
//!
//! * `max_concurrent` — running deployments;
//! * `max_aggregate_capital` — summed paper capital across them;
//! * `max_instrument_exposure` — per instrument, **aggregated across every
//!   instance of a revision** ([`crate::capacity::Exposure`]), not per instance.
//!   Thirty instances of one revision on one instrument are one position at thirty
//!   times the size.
//!
//! # No ceiling is "unlimited"
//!
//! [`FleetCeilings`] has no `Default` and no constructor that takes an
//! `Option`: a ceiling that was never declared is [`CeilingError::Undeclared`],
//! and an undeclared ceiling refuses every deployment. "No value" must not
//! silently permit everything (the same rule `capacity::Admission::Unmeasured`
//! states for Gate 4).

use serde::{Deserialize, Serialize};

use crate::capacity::{admit, Admission, Ceiling, Exposure, Instance};

/// The ceiling that stopped a deployment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    MaxConcurrent,
    MaxAggregateCapital,
    MaxInstrumentExposure,
}

impl Binding {
    pub const ALL: [Self; 3] = [
        Self::MaxConcurrent,
        Self::MaxAggregateCapital,
        Self::MaxInstrumentExposure,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxConcurrent => "max_concurrent",
            Self::MaxAggregateCapital => "max_aggregate_capital",
            Self::MaxInstrumentExposure => "max_instrument_exposure",
        }
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CeilingError {
    #[error("the `{}` ceiling has not been declared; an undeclared ceiling refuses every deployment rather than permitting all of them", .0.as_str())]
    Undeclared(Binding),
    #[error("the `{}` ceiling must be a positive finite number", .0.as_str())]
    Invalid(Binding),
    #[error("the `{binding}` ceiling bound: {message}", binding = .binding.as_str())]
    Bound { binding: Binding, message: String },
}

impl CeilingError {
    /// The ceiling that bound, if one did (an undeclared one counts as bound).
    #[must_use]
    pub fn binding(&self) -> Binding {
        match self {
            Self::Undeclared(b) | Self::Invalid(b) => *b,
            Self::Bound { binding, .. } => *binding,
        }
    }
}

/// The three declared ceilings. Constructed only from values that were all
/// declared.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
// The `max_` prefix is the declared ceiling's own name in the spec and in the stored
// JSON; dropping it would rename the wire format.
#[allow(clippy::struct_field_names)]
pub struct FleetCeilings {
    max_concurrent: u32,
    max_aggregate_capital: f64,
    max_instrument_exposure: f64,
}

impl FleetCeilings {
    /// # Errors
    /// [`CeilingError::Undeclared`] for the first `None`, [`CeilingError::Invalid`]
    /// for a non-positive or non-finite value.
    pub fn declared(
        max_concurrent: Option<f64>,
        max_aggregate_capital: Option<f64>,
        max_instrument_exposure: Option<f64>,
    ) -> Result<Self, CeilingError> {
        let get = |v: Option<f64>, b: Binding| -> Result<f64, CeilingError> {
            let v = v.ok_or(CeilingError::Undeclared(b))?;
            if v.is_finite() && v > 0.0 {
                Ok(v)
            } else {
                Err(CeilingError::Invalid(b))
            }
        };
        let c = get(max_concurrent, Binding::MaxConcurrent)?;
        if c.fract() != 0.0 || c > f64::from(u32::MAX) {
            return Err(CeilingError::Invalid(Binding::MaxConcurrent));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_concurrent = c as u32;
        Ok(Self {
            max_concurrent,
            max_aggregate_capital: get(max_aggregate_capital, Binding::MaxAggregateCapital)?,
            max_instrument_exposure: get(max_instrument_exposure, Binding::MaxInstrumentExposure)?,
        })
    }

    #[must_use]
    pub fn max_concurrent(&self) -> u32 {
        self.max_concurrent
    }
}

/// One running (or proposed) deployment, as the fleet check sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub revision_hash: String,
    pub instrument_id: String,
    /// Paper capital, USD. A deployment's exposure cannot exceed it.
    pub capital_usd: f64,
}

/// May `proposed` join `running`?
///
/// # Errors
/// [`CeilingError::Bound`] naming the first ceiling it would break: concurrency,
/// then aggregate capital, then the per-instrument aggregate for its revision.
pub fn admit_deployment(
    ceilings: &FleetCeilings,
    running: &[Running],
    proposed: &Running,
) -> Result<(), CeilingError> {
    if !(proposed.capital_usd.is_finite() && proposed.capital_usd > 0.0) {
        return Err(CeilingError::Invalid(Binding::MaxAggregateCapital));
    }
    let n = running.len() as u64;
    if n + 1 > u64::from(ceilings.max_concurrent) {
        return Err(CeilingError::Bound {
            binding: Binding::MaxConcurrent,
            message: format!(
                "{n} deployment(s) are running and the ceiling is {}",
                ceilings.max_concurrent
            ),
        });
    }
    let total: f64 = running.iter().map(|r| r.capital_usd).sum::<f64>() + proposed.capital_usd;
    if total > ceilings.max_aggregate_capital {
        return Err(CeilingError::Bound {
            binding: Binding::MaxAggregateCapital,
            message: format!(
                "aggregate paper capital would be {total:.0} against a ceiling of {:.0}",
                ceilings.max_aggregate_capital
            ),
        });
    }
    // Aggregated across every instance of this revision (capacity::Exposure),
    // whatever account or instance holds it.
    let instances: Vec<Instance> = running
        .iter()
        .map(|r| Instance {
            revision_hash: r.revision_hash.clone(),
            account_id: "paper".to_owned(),
            instrument_id: r.instrument_id.clone(),
            notional_usd: r.capital_usd,
        })
        .collect();
    let exposure = Exposure::across(&proposed.revision_hash, &instances);
    let ceiling = Ceiling {
        instrument_id: proposed.instrument_id.clone(),
        ceiling_usd: ceilings.max_instrument_exposure,
        single_venue: false,
        source_trial_id: "fleet_ceiling".to_owned(),
    };
    match admit(
        exposure.as_ref(),
        &proposed.instrument_id,
        proposed.capital_usd,
        Some(&ceiling),
    ) {
        Admission::Admitted { .. } => Ok(()),
        Admission::Refused { reason } | Admission::Unmeasured { reason } => {
            Err(CeilingError::Bound {
                binding: Binding::MaxInstrumentExposure,
                message: reason,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ceilings() -> FleetCeilings {
        FleetCeilings::declared(Some(3.0), Some(100_000.0), Some(25_000.0)).unwrap()
    }

    fn r(rev: &str, instr: &str, cap: f64) -> Running {
        Running {
            revision_hash: rev.into(),
            instrument_id: instr.into(),
            capital_usd: cap,
        }
    }

    #[test]
    fn an_undeclared_ceiling_is_a_refusal_not_unlimited() {
        for (a, b, c, which) in [
            (None, Some(1.0), Some(1.0), Binding::MaxConcurrent),
            (Some(1.0), None, Some(1.0), Binding::MaxAggregateCapital),
            (Some(1.0), Some(1.0), None, Binding::MaxInstrumentExposure),
        ] {
            assert_eq!(
                FleetCeilings::declared(a, b, c),
                Err(CeilingError::Undeclared(which))
            );
        }
        assert_eq!(
            FleetCeilings::declared(Some(0.0), Some(1.0), Some(1.0)),
            Err(CeilingError::Invalid(Binding::MaxConcurrent))
        );
        assert_eq!(
            FleetCeilings::declared(Some(2.5), Some(1.0), Some(1.0)),
            Err(CeilingError::Invalid(Binding::MaxConcurrent))
        );
        assert!(FleetCeilings::declared(Some(1.0), Some(f64::INFINITY), Some(1.0)).is_err());
    }

    #[test]
    fn each_ceiling_refuses_and_names_itself() {
        let c = ceilings();
        // concurrency
        let full = [r("a", "X", 1.0), r("b", "Y", 1.0), r("c", "Z", 1.0)];
        assert_eq!(
            admit_deployment(&c, &full, &r("d", "W", 1.0))
                .unwrap_err()
                .binding(),
            Binding::MaxConcurrent
        );
        // aggregate capital
        let heavy = [r("a", "X", 60_000.0)];
        assert_eq!(
            admit_deployment(&c, &heavy, &r("b", "Y", 50_000.0))
                .unwrap_err()
                .binding(),
            Binding::MaxAggregateCapital
        );
        // within every ceiling
        assert!(admit_deployment(&c, &heavy, &r("b", "Y", 20_000.0)).is_ok());
    }

    /// Aggregated across instances of the same revision: two 15k instances of
    /// revision `a` on BTC are 30k, over the 25k per-instrument ceiling, though
    /// each is under it alone.
    #[test]
    fn exposure_is_aggregated_across_instances_of_a_revision_not_per_instance() {
        let c = ceilings();
        let running = [r("a", "BTC", 15_000.0)];
        let err = admit_deployment(&c, &running, &r("a", "BTC", 15_000.0)).unwrap_err();
        assert_eq!(err.binding(), Binding::MaxInstrumentExposure);
        assert!(err.to_string().contains("correlation 1.0"), "{err}");
        // A different revision on the same instrument is a separate aggregate.
        assert!(admit_deployment(&c, &running, &r("b", "BTC", 15_000.0)).is_ok());
        // The same revision on another instrument is a separate aggregate too.
        assert!(admit_deployment(&c, &running, &r("a", "ETH", 15_000.0)).is_ok());
    }

    #[test]
    fn the_first_binding_in_order_is_the_one_named() {
        let c = FleetCeilings::declared(Some(1.0), Some(10.0), Some(5.0)).unwrap();
        let running = [r("a", "X", 9.0)];
        // Breaks all three; concurrency is named first.
        assert_eq!(
            admit_deployment(&c, &running, &r("a", "X", 9.0))
                .unwrap_err()
                .binding(),
            Binding::MaxConcurrent
        );
    }
}
