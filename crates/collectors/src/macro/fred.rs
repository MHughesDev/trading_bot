//! FRED / ALFRED macro series with vintages (DATA-005 §11 S4, §12; Set P P-3.1).
//!
//! **Why ALFRED and not FRED.** FRED's `series/observations` takes
//! `realtime_start` and `realtime_end`, and the documentation gives both the
//! default "today's date". Ask it for GDP in 2023-Q1 and it answers with the
//! estimate current *today*, whatever date you had in mind. A strategy reading
//! that in a backtest of April 2023 is reading the future.
//!
//! ALFRED is the same endpoint with the real-time range opened up. Asking for
//! `realtime_start=1776-07-04&realtime_end=9999-12-31` returns one row per
//! *vintage*: the value as it stood, with the day it was published. That is what
//! this collector stores, keyed on `(series_id, observation_date, vintage_date)`,
//! and it is why [`dataplane::vintage`] can answer "what did the platform know on
//! 2023-05-15" instead of "what does the platform know now".
//!
//! **Credentials.** The API key is a [`Secret`] loaded from the encrypted
//! credential store. It never appears in `config/`, and the only URL any logging
//! path ever sees is [`AlfredClient::redacted_url`].
//!
//! ## Free-plan depth — re-verified 2026-09-22
//!
//! FRED/ALFRED has no paid tier: the API is free and an `api_key` is required
//! (`fred.stlouisfed.org/docs/api/api_key.html`: "All web service requests
//! require an API key to identify requests").
//!
//! - **Observation depth:** the whole series. `1776-07-04` is the documented
//!   earliest date the endpoint's date parameters accept, and it is what this
//!   module sends as the real-time floor.
//! - **Vintage depth is per series, not global.** ALFRED archives a series from
//!   the date it began archiving *that* series, which is not the same as the
//!   series' first observation: a long series can have decades of observations
//!   and a much shorter revision history. `fred/series/vintagedates` states it
//!   per series, and a backfill should record what it found rather than assume
//!   the observation range.
//! - **Rate: no published ceiling.** Neither the API overview, the key page nor
//!   the `series/observations` reference states one (checked 2026-09-22). The
//!   widely repeated "120 requests per minute" is not on any page St. Louis Fed
//!   serves as documentation, so [`VenueLimit::fred_api`] declares a *self-imposed*
//!   budget and says so rather than quoting a number nobody published.
//!
//! Macro series are therefore **backfillable** and do not join the FM-11
//! forward-collect argument — with the caveat above, that vintages older than
//! ALFRED's archive for a given series are not recoverable from anywhere.

use chrono::NaiveDate;
use dataplane::vintage::{derived_knowledge, MacroObservation};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::ratelimit::{Denied, RateLimiter, VenueLimit};
use crate::secret::{Credential, CredentialOrigin, Secret};

/// The public ALFRED/FRED host.
pub const FRED_BASE: &str = "https://api.stlouisfed.org/fred";

/// The `dataplane.source.name` this collector writes under. The row itself is a
/// migration this track does not write — see the report.
pub const SOURCE_NAME: &str = "alfred_rest";

/// The earliest real-time date the endpoint's date parameters accept.
const REALTIME_FLOOR: &str = "1776-07-04";
/// ALFRED's open-ended real-time end.
const REALTIME_CEILING: &str = "9999-12-31";

#[derive(Debug, thiserror::Error)]
pub enum FredError {
    #[error("http error: {0}")]
    Http(String),
    #[error("fred returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("malformed response: {0}")]
    Parse(String),
    #[error("the FRED API key must come from the credential store, not {0:?}")]
    BadCredentialOrigin(CredentialOrigin),
    #[error("the FRED API key is empty")]
    EmptyKey,
    /// The budget is spent and the request was **not** sent. The caller waits
    /// [`Denied::retry_after`] and asks again; it may not proceed past this.
    #[error("rate limited: {0}")]
    RateLimited(Denied),
}

// ── ALFRED response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlfredObservationsResponse {
    pub observations: Vec<AlfredObservation>,
}

#[derive(Debug, Deserialize)]
pub struct AlfredObservation {
    /// The day this value became the published value — the vintage date.
    pub realtime_start: String,
    /// The day it stopped being the published value; `9999-12-31` while current.
    #[serde(default)]
    pub realtime_end: String,
    /// The period the value describes.
    pub date: String,
    /// `"."` for missing.
    pub value: String,
}

// ── Client ───────────────────────────────────────────────────────────────────

/// An ALFRED reader.
///
/// `Debug` is derived deliberately: [`Secret`] redacts, so a `{:?}` of the client
/// in a log line prints the base URL and `Secret(***)`.
#[derive(Debug)]
pub struct AlfredClient {
    base: String,
    key: Secret,
    limiter: RateLimiter,
    http: reqwest::Client,
}

impl AlfredClient {
    /// # Errors
    /// Refuses a credential that did not come from the credential store, and an
    /// empty key. A key read from `config/` cannot reach this constructor,
    /// because [`CredentialOrigin`] has no `Config` variant.
    pub fn new(credential: &Credential) -> Result<Self, FredError> {
        if credential.origin != CredentialOrigin::CredentialStore {
            return Err(FredError::BadCredentialOrigin(credential.origin));
        }
        if credential.secret.is_empty() {
            return Err(FredError::EmptyKey);
        }
        Ok(Self {
            base: FRED_BASE.to_owned(),
            key: credential.secret.clone(),
            limiter: RateLimiter::new(VenueLimit::fred_api()),
            http: reqwest::Client::new(),
        })
    }

    /// Points the client at a different host (a local fixture server in tests).
    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// The budget this client spends against.
    #[must_use]
    pub const fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    /// The request URL, **key included**. Never log this.
    #[must_use]
    pub fn observations_url(&self, series_id: &str) -> String {
        format!(
            "{}/series/observations?series_id={series_id}&api_key={}&file_type=json\
             &realtime_start={REALTIME_FLOOR}&realtime_end={REALTIME_CEILING}",
            self.base,
            self.key.expose()
        )
    }

    /// The same URL with the key replaced. This is the one that goes in logs and
    /// errors, and it is the only URL-shaped string this module ever renders
    /// outside [`Self::observations_url`].
    #[must_use]
    pub fn redacted_url(&self, series_id: &str) -> String {
        format!(
            "{}/series/observations?series_id={series_id}&api_key=***&file_type=json\
             &realtime_start={REALTIME_FLOOR}&realtime_end={REALTIME_CEILING}",
            self.base
        )
    }

    /// Every vintage of every observation in `series_id`.
    ///
    /// # Errors
    /// [`FredError::RateLimited`] before anything is sent when the budget is
    /// spent; otherwise transport failures, non-2xx responses and malformed rows.
    /// The error text carries the redacted URL, never the key.
    pub async fn fetch_all_vintages(
        &self,
        series_id: &str,
    ) -> Result<Vec<MacroObservation>, FredError> {
        self.limiter.try_acquire().map_err(FredError::RateLimited)?;
        let response = self
            .http
            .get(self.observations_url(series_id))
            .send()
            .await
            .map_err(|e| FredError::Http(format!("{}: {e}", self.redacted_url(series_id))))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| FredError::Http(format!("{}: {e}", self.redacted_url(series_id))))?;
        if !status.is_success() {
            return Err(FredError::Status {
                status: status.as_u16(),
                body: redact_key(&body),
            });
        }
        parse_vintages(series_id, &body)
    }
}

/// Strips anything that looks like a key out of an upstream error body.
fn redact_key(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(idx) = rest.find("api_key=") {
        out.push_str(&rest[..idx]);
        out.push_str("api_key=***");
        rest = &rest[idx + "api_key=".len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Parses an ALFRED observations payload into vintage-keyed rows.
///
/// # Errors
/// A malformed date, an unparseable value, or a row that fails
/// [`MacroObservation::validate`] — a vintage that precedes its own observation
/// period is refused rather than stored.
pub fn parse_vintages(series_id: &str, body: &str) -> Result<Vec<MacroObservation>, FredError> {
    // json-at-boundary: ALFRED's observations REST response, not one of our envelopes
    let parsed = serde_json::from_str::<AlfredObservationsResponse>(body)
        .map_err(|e| FredError::Parse(e.to_string()))?;
    let mut out = Vec::with_capacity(parsed.observations.len());
    for obs in parsed.observations {
        let observation_date = NaiveDate::from_str(&obs.date)
            .map_err(|e| FredError::Parse(format!("date {:?}: {e}", obs.date)))?;
        let vintage_date = NaiveDate::from_str(&obs.realtime_start).map_err(|e| {
            FredError::Parse(format!("realtime_start {:?}: {e}", obs.realtime_start))
        })?;
        // ALFRED writes "." for a period the agency has not published. That is
        // missing, not zero, and zero would be a number a strategy could trade.
        let value = if obs.value.trim() == "." {
            None
        } else {
            Some(
                Decimal::from_str(obs.value.trim())
                    .map_err(|e| FredError::Parse(format!("value {:?}: {e}", obs.value)))?,
            )
        };
        let row = MacroObservation {
            series_id: series_id.to_owned(),
            observation_date,
            vintage_date,
            value,
            knowledge: derived_knowledge(vintage_date),
        };
        row.validate()
            .map_err(|e| FredError::Parse(e.to_string()))?;
        out.push(row);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataplane::vintage::value_as_of;
    use rust_decimal_macros::dec;

    fn client() -> AlfredClient {
        AlfredClient::new(&Credential::from_store(Secret::new("abcdef0123456789")))
            .expect("store credential is accepted")
    }

    /// Two vintages of one quarter, as ALFRED serves them.
    const REVISED_GDP: &str = r#"{
      "observations": [
        {"realtime_start":"2023-04-27","realtime_end":"2023-06-28","date":"2023-01-01","value":"2.0"},
        {"realtime_start":"2023-06-29","realtime_end":"9999-12-31","date":"2023-01-01","value":"2.6"},
        {"realtime_start":"2023-07-27","realtime_end":"9999-12-31","date":"2023-04-01","value":"."}
      ]
    }"#;

    #[test]
    fn vintages_parse_into_point_in_time_rows() {
        let rows = parse_vintages("GDPC1", REVISED_GDP).expect("fixture parses");
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].vintage_date,
            NaiveDate::from_ymd_opt(2023, 4, 27).expect("valid date")
        );
        assert_eq!(rows[0].value, Some(dec!(2.0)));
        assert!(rows[0].knowledge.is_derived());
        // "." is missing, never zero.
        assert_eq!(rows[2].value, None);
    }

    /// P-3.1 acceptance, end to end over the parser: a planted revision must not
    /// change what a read before it returns.
    #[test]
    fn a_planted_revision_does_not_change_the_earlier_read() {
        let rows = parse_vintages("GDPC1", REVISED_GDP).expect("fixture parses");
        let as_of = chrono::DateTime::parse_from_rfc3339("2023-05-15T00:00:00Z")
            .expect("valid instant")
            .with_timezone(&chrono::Utc);
        let seen = value_as_of(
            &rows,
            "GDPC1",
            NaiveDate::from_ymd_opt(2023, 1, 1).expect("valid date"),
            as_of,
        )
        .expect("the April vintage was knowable in May");
        assert_eq!(seen.value, Some(dec!(2.0)));
    }

    /// The same query run twice returns the same answer (P-3.1 acceptance).
    #[test]
    fn the_parse_is_deterministic() {
        let a = parse_vintages("GDPC1", REVISED_GDP).expect("fixture parses");
        let b = parse_vintages("GDPC1", REVISED_GDP).expect("fixture parses");
        assert_eq!(a, b);
    }

    /// The key is in the request URL because FRED puts it there, and nowhere else.
    #[test]
    fn the_redacted_url_never_carries_the_key() {
        let c = client();
        assert!(c.observations_url("GDPC1").contains("abcdef0123456789"));
        assert!(!c.redacted_url("GDPC1").contains("abcdef0123456789"));
        assert!(c.redacted_url("GDPC1").contains("api_key=***"));
    }

    /// The adversarial half of "the key is absent from logs": the Debug rendering
    /// a `tracing` macro would emit must not contain it.
    #[test]
    fn the_client_debug_rendering_never_carries_the_key() {
        let rendered = format!("{:?}", client());
        assert!(!rendered.contains("abcdef0123456789"), "{rendered}");
    }

    #[test]
    fn an_upstream_error_body_is_redacted() {
        let body = "Bad Request. api_key=abcdef0123456789 is not a valid key.";
        let out = redact_key(body);
        assert!(!out.contains("abcdef0123456789"), "{out}");
        assert!(out.contains("api_key=***"));
    }

    /// The credential-origin half of P-3.1's acceptance: a key that did not come
    /// from the credential store cannot construct a client at all.
    #[test]
    fn a_key_not_from_the_credential_store_is_refused() {
        let from_env = Credential::from_env(Secret::new("abcdef0123456789"));
        assert!(matches!(
            AlfredClient::new(&from_env),
            Err(FredError::BadCredentialOrigin(
                CredentialOrigin::Environment
            ))
        ));
        let empty = Credential::from_store(Secret::new(""));
        assert!(matches!(
            AlfredClient::new(&empty),
            Err(FredError::EmptyKey)
        ));
    }

    #[test]
    fn a_vintage_before_its_observation_period_is_refused() {
        let impossible = r#"{"observations":[
          {"realtime_start":"2023-01-01","realtime_end":"9999-12-31","date":"2023-04-01","value":"1.0"}
        ]}"#;
        assert!(matches!(
            parse_vintages("GDPC1", impossible),
            Err(FredError::Parse(_))
        ));
    }

    /// The budget is declared, and it is declared as self-imposed because FRED
    /// publishes no ceiling. See [`VenueLimit::fred_api`].
    #[test]
    fn the_client_spends_a_declared_budget() {
        let c = client();
        assert_eq!(c.limiter().limit().venue, "fred_api");
        assert!(c.limiter().limit().source_quote.contains("self-imposed"));
        // And it refuses once spent, rather than sending the request anyway.
        for _ in 0..c.limiter().limit().capacity {
            c.limiter().try_acquire().expect("inside the budget");
        }
        assert!(c.limiter().try_acquire().is_err());
    }
}
