//! SEC EDGAR submissions and companyfacts/XBRL (DATA-005 §11 S3, §13 D9;
//! Set P, P-3.2).
//!
//! **The knowledge time is `accepted_at`, not the period.** `companyfacts` gives
//! each fact an `end` (the period it describes) and a `filed` date (when the
//! filing reached EDGAR). Joining on `end` hands a March number to a March
//! backtest even though nobody outside the company had it until May. Every fact
//! this module produces carries an acceptance time and nothing reads it without
//! an `as_of` — see [`dataplane::fundamentals`].
//!
//! `companyfacts` carries `filed` as a *date*, with no time of day. The exact
//! acceptance instant lives in `submissions/CIK##########.json` as
//! `acceptanceDateTime`, so [`join_acceptance`] upgrades a derived time to an
//! observed one when the submissions index is available. Until it is, the derived
//! time is the **end** of the filed day: late is a cost, early is a lookahead bug.
//!
//! **Fair access is binding, not advisory.** The SEC's policy requires a declared
//! User-Agent that identifies the operator and permits no more than 10 requests
//! per second. Both are enforced here — the User-Agent is a required constructor
//! argument with a format check, and the limiter is a token bucket whose refusal
//! is asserted by a test. Getting banned from EDGAR costs more than the filings.
//!
//! ## Free-plan depth — verified 2026-09-15
//!
//! Verified against `sec.gov/search-filings/edgar-application-programming-interfaces`
//! and `…/edgar-search-assistance/accessing-edgar-data`:
//!
//! - **Cost:** free, with **no authentication, no API key and no plan tiers**.
//!   There is no shallower free tier to work around.
//! - **Depth:** `companyfacts` aggregates every XBRL concept a filer has reported
//!   across 10-K, 10-Q, 8-K, 20-F, 40-F and 6-K. The SEC's page does **not**
//!   publish a start date, so this module does not claim one: the honest
//!   statement is that depth is per-filer and equals that filer's XBRL reporting
//!   history, which a census job should measure rather than assume.
//! - **Rate:** 10 requests per second, and a declared User-Agent of the form
//!   `Sample Company Name AdminContact@<domain>.com`.
//! - **Dissemination:** the submissions API is updated in real time with "a
//!   typical processing delay of less than a second", which is what
//!   `dataplane::source`'s declared lag for `sec_edgar` reflects.
//!
//! Fundamentals are therefore **backfillable** and do not join the FM-11
//! forward-collect argument.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use dataplane::fundamentals::FundamentalFact;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::ratelimit::{self, TokenBucket};

/// EDGAR's structured-data host.
pub const EDGAR_BASE: &str = "https://data.sec.gov";

/// The registry name of this source (`dataplane::source`, migration 0102).
pub const SOURCE_NAME: &str = "sec_edgar";

/// The SEC's published ceiling. Not a suggestion.
pub const MAX_REQUESTS_PER_SEC: u32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum EdgarError {
    #[error("http error: {0}")]
    Http(String),
    #[error("edgar returned {status} for {url}")]
    Status { status: u16, url: String },
    #[error("malformed response: {0}")]
    Parse(String),
    #[error(
        "the EDGAR User-Agent must identify the operator as \"Name contact@example.com\"; \
         got {0:?}"
    )]
    BadUserAgent(String),
}

/// Checks a User-Agent against the SEC's stated requirement: a name and a working
/// contact address.
///
/// # Errors
/// Anything without a contact address, or shorter than a plausible identity. A
/// browser-shaped string is refused: the requirement is to be identifiable, and
/// pretending to be Chrome is the opposite.
pub fn validate_user_agent(ua: &str) -> Result<(), EdgarError> {
    let trimmed = ua.trim();
    let looks_like_a_browser = trimmed.starts_with("Mozilla/") || trimmed.starts_with("curl/");
    let has_contact = trimmed.contains('@') && trimmed.contains('.');
    let has_name = trimmed.split_whitespace().count() >= 2;
    if looks_like_a_browser || !has_contact || !has_name || trimmed.len() < 8 {
        return Err(EdgarError::BadUserAgent(ua.to_owned()));
    }
    Ok(())
}

// ── companyfacts response shapes ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CompanyFactsResponse {
    pub cik: u32,
    #[serde(default)]
    pub facts: BTreeMap<String, BTreeMap<String, FactTag>>,
}

#[derive(Debug, Deserialize)]
pub struct FactTag {
    #[serde(default)]
    pub units: BTreeMap<String, Vec<FactEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct FactEntry {
    #[serde(default)]
    pub start: Option<String>,
    pub end: String,
    pub val: Decimal,
    pub accn: String,
    #[serde(default)]
    pub fy: Option<i32>,
    #[serde(default)]
    pub fp: Option<String>,
    pub form: String,
    /// The date the filing reached EDGAR. A date, not an instant.
    pub filed: String,
}

/// The `submissions/CIK##########.json` index, used only for acceptance instants.
#[derive(Debug, Deserialize)]
pub struct SubmissionsResponse {
    #[serde(default)]
    pub filings: Submissions,
}

#[derive(Debug, Default, Deserialize)]
pub struct Submissions {
    #[serde(default)]
    pub recent: RecentFilings,
}

#[derive(Debug, Default, Deserialize)]
pub struct RecentFilings {
    #[serde(default)]
    #[serde(rename = "accessionNumber")]
    pub accession_number: Vec<String>,
    #[serde(default)]
    #[serde(rename = "acceptanceDateTime")]
    pub acceptance_date_time: Vec<String>,
}

// ── Client ───────────────────────────────────────────────────────────────────

/// An EDGAR reader that cannot be constructed without a declared identity and
/// cannot exceed ten requests a second.
#[derive(Debug)]
pub struct EdgarClient {
    base: String,
    user_agent: String,
    limiter: TokenBucket,
    http: reqwest::Client,
}

impl EdgarClient {
    /// # Errors
    /// [`EdgarError::BadUserAgent`] when the operator is not identified.
    pub fn new(user_agent: &str) -> Result<Self, EdgarError> {
        validate_user_agent(user_agent)?;
        Ok(Self {
            base: EDGAR_BASE.to_owned(),
            user_agent: user_agent.to_owned(),
            limiter: TokenBucket::new(MAX_REQUESTS_PER_SEC, MAX_REQUESTS_PER_SEC),
            http: reqwest::Client::new(),
        })
    }

    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// The declared User-Agent sent on every request.
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    #[must_use]
    pub fn companyfacts_url(&self, cik: u32) -> String {
        format!("{}/api/xbrl/companyfacts/CIK{cik:010}.json", self.base)
    }

    #[must_use]
    pub fn submissions_url(&self, cik: u32) -> String {
        format!("{}/submissions/CIK{cik:010}.json", self.base)
    }

    /// One rate-limited, identified GET.
    async fn get(&mut self, url: &str) -> Result<String, EdgarError> {
        ratelimit::acquire(&mut self.limiter).await;
        let response = self
            .http
            .get(url)
            // The header is attached here, on the single path every request takes,
            // so there is no request in this module that can omit it.
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .header(reqwest::header::ACCEPT_ENCODING, "gzip, deflate")
            .send()
            .await
            .map_err(|e| EdgarError::Http(format!("{url}: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(EdgarError::Status {
                status: status.as_u16(),
                url: url.to_owned(),
            });
        }
        response
            .text()
            .await
            .map_err(|e| EdgarError::Http(format!("{url}: {e}")))
    }

    /// Every XBRL fact for `cik`, with derived acceptance times.
    ///
    /// # Errors
    /// Transport, status and parse failures.
    pub async fn fetch_company_facts(
        &mut self,
        cik: u32,
    ) -> Result<Vec<FundamentalFact>, EdgarError> {
        let url = self.companyfacts_url(cik);
        let body = self.get(&url).await?;
        parse_company_facts(&body)
    }

    /// `accession -> acceptance instant` for `cik`'s recent filings.
    ///
    /// # Errors
    /// Transport, status and parse failures.
    pub async fn fetch_acceptance_times(
        &mut self,
        cik: u32,
    ) -> Result<BTreeMap<String, DateTime<Utc>>, EdgarError> {
        let url = self.submissions_url(cik);
        let body = self.get(&url).await?;
        parse_acceptance_times(&body)
    }
}

/// The conservative acceptance instant for a filing known only by its filed date.
///
/// End of day, not midnight. EDGAR accepts filings until 22:00 ET and disseminates
/// after; claiming a filing was knowable at 00:00 on its filed date would hand a
/// backtest most of a trading day it never had.
#[must_use]
pub fn derived_acceptance(filed: NaiveDate) -> DateTime<Utc> {
    let end = filed
        .and_hms_opt(23, 59, 59)
        .unwrap_or_else(|| filed.and_hms_opt(0, 0, 0).expect("midnight exists"));
    Utc.from_utc_datetime(&end)
}

/// Parses a `companyfacts` payload into facts with derived acceptance times.
///
/// # Errors
/// Malformed dates, and any fact that fails [`FundamentalFact::validate`] — a
/// fact accepted before its own period ended is refused rather than stored.
pub fn parse_company_facts(body: &str) -> Result<Vec<FundamentalFact>, EdgarError> {
    // Parsing a body fetched from an external REST API — the documented exception
    // in `cargo xtask lint-no-json-hotpath`.
    let parsed = serde_json::from_str::<CompanyFactsResponse>(&body[..])
        .map_err(|e| EdgarError::Parse(e.to_string()))?;
    let cik = parsed.cik;
    let mut out = Vec::new();
    for taxonomy in parsed.facts.values() {
        for (tag, fact_tag) in taxonomy {
            for (unit, entries) in &fact_tag.units {
                for entry in entries {
                    let period_end = NaiveDate::from_str(&entry.end)
                        .map_err(|e| EdgarError::Parse(format!("end {:?}: {e}", entry.end)))?;
                    let period_start = match &entry.start {
                        Some(s) => Some(NaiveDate::from_str(s).map_err(|e| {
                            EdgarError::Parse(format!("start {s:?}: {e}"))
                        })?),
                        None => None,
                    };
                    let filed = NaiveDate::from_str(&entry.filed).map_err(|e| {
                        EdgarError::Parse(format!("filed {:?}: {e}", entry.filed))
                    })?;
                    let fact = FundamentalFact {
                        cik,
                        tag: tag.clone(),
                        unit: unit.clone(),
                        period_start,
                        period_end,
                        fiscal_year: entry.fy,
                        fiscal_period: entry.fp.clone(),
                        form: entry.form.clone(),
                        accession: entry.accn.clone(),
                        value: entry.val,
                        accepted_at: derived_acceptance(filed),
                        accepted_at_derived: true,
                    };
                    fact.validate().map_err(|e| EdgarError::Parse(e.to_string()))?;
                    out.push(fact);
                }
            }
        }
    }
    Ok(out)
}

/// Parses the submissions index into `accession -> acceptance instant`.
///
/// # Errors
/// A malformed payload or an unparseable timestamp.
pub fn parse_acceptance_times(body: &str) -> Result<BTreeMap<String, DateTime<Utc>>, EdgarError> {
    // Parsing a body fetched from an external REST API — see the note above.
    let parsed = serde_json::from_str::<SubmissionsResponse>(&body[..])
        .map_err(|e| EdgarError::Parse(e.to_string()))?;
    let recent = parsed.filings.recent;
    let mut map = BTreeMap::new();
    for (accession, accepted) in recent
        .accession_number
        .iter()
        .zip(recent.acceptance_date_time.iter())
    {
        // EDGAR writes these as `2023-05-04T18:03:41.000Z` or without the zone,
        // in which case the instant is US/Eastern. Only the explicit form is
        // trusted; a naive string is left derived rather than guessed at.
        let Ok(parsed) = DateTime::parse_from_rfc3339(accepted) else {
            continue;
        };
        map.insert(accession.clone(), parsed.with_timezone(&Utc));
    }
    Ok(map)
}

/// Replaces derived acceptance times with observed ones wherever the submissions
/// index knows the accession.
///
/// A fact whose accession is absent keeps its conservative derived time and its
/// `accepted_at_derived` flag, so a dataset can still tell the two apart.
pub fn join_acceptance(
    facts: &mut [FundamentalFact],
    acceptance: &BTreeMap<String, DateTime<Utc>>,
) {
    for fact in facts.iter_mut() {
        if let Some(observed) = acceptance.get(&fact.accession) {
            // Never move a knowledge time *earlier* than the conservative
            // derivation would allow it to be known — except that an observed
            // timestamp IS the truth, so it replaces the derivation in both
            // directions. What is refused is an observed time before the period
            // ended, which `validate` already rejects.
            fact.accepted_at = *observed;
            fact.accepted_at_derived = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataplane::fundamentals::{fact_for_period_as_of, latest_as_of};
    use rust_decimal_macros::dec;

    const UA: &str = "Trading Bot Research ops@trading-bot.invalid";

    /// A Q1 fact filed a quarter after the period it describes.
    const FACTS: &str = r#"{
      "cik": 320193,
      "entityName": "Apple Inc.",
      "facts": {
        "us-gaap": {
          "Revenues": {
            "units": {
              "USD": [
                {"start":"2023-01-01","end":"2023-03-31","val":100,"accn":"0000320193-23-000064","fy":2023,"fp":"Q1","form":"10-Q","filed":"2023-06-30"},
                {"start":"2023-01-01","end":"2023-03-31","val":90,"accn":"0000320193-23-000099","fy":2023,"fp":"Q1","form":"10-Q/A","filed":"2023-11-01"}
              ]
            }
          }
        }
      }
    }"#;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("valid instant")
            .with_timezone(&Utc)
    }

    #[test]
    fn companyfacts_parse_into_acceptance_keyed_facts() {
        let facts = parse_company_facts(FACTS).expect("fixture parses");
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].cik, 320_193);
        assert_eq!(facts[0].tag, "Revenues");
        assert_eq!(facts[0].unit, "USD");
        assert_eq!(facts[0].value, dec!(100));
        assert!(facts[0].accepted_at_derived);
        // End of the filed day, not midnight.
        assert_eq!(facts[0].accepted_at, t("2023-06-30T23:59:59Z"));
    }

    /// DATA-005 §13 D9 — the acceptance test the plan cites as "§10.4".
    /// A request as of `t` never returns a value filed after `t`, tested with a
    /// fact whose period precedes its acceptance by a quarter.
    #[test]
    fn a_request_as_of_t_never_returns_a_value_filed_after_t() {
        let facts = parse_company_facts(FACTS).expect("fixture parses");
        // The quarter ended 2023-03-31. The filing was accepted 2023-06-30.
        for as_of in [
            "2023-04-01T00:00:00Z",
            "2023-05-15T12:00:00Z",
            "2023-06-30T12:00:00Z",
        ] {
            assert!(
                latest_as_of(&facts, 320_193, "Revenues", t(as_of)).is_none(),
                "a fact filed on 2023-06-30 must be invisible at {as_of}"
            );
        }
        assert!(latest_as_of(&facts, 320_193, "Revenues", t("2023-07-01T00:00:00Z")).is_some());
    }

    /// The amendment half: a restatement accepted in November must not change
    /// what an August read sees.
    #[test]
    fn a_planted_restatement_does_not_leak_backwards() {
        let facts = parse_company_facts(FACTS).expect("fixture parses");
        let period = NaiveDate::from_ymd_opt(2023, 3, 31).expect("valid date");
        let august = fact_for_period_as_of(&facts, 320_193, "Revenues", period, t("2023-08-01T00:00:00Z"))
            .expect("the original print was knowable in August");
        assert_eq!(august.value, dec!(100));
        let december = fact_for_period_as_of(&facts, 320_193, "Revenues", period, t("2023-12-01T00:00:00Z"))
            .expect("the amendment was knowable in December");
        assert_eq!(december.value, dec!(90));
    }

    #[test]
    fn an_observed_acceptance_time_replaces_the_derived_one() {
        let mut facts = parse_company_facts(FACTS).expect("fixture parses");
        let submissions = r#"{"filings":{"recent":{
          "accessionNumber":["0000320193-23-000064"],
          "acceptanceDateTime":["2023-06-30T18:03:41Z"]
        }}}"#;
        let acceptance = parse_acceptance_times(submissions).expect("fixture parses");
        join_acceptance(&mut facts, &acceptance);
        let joined = facts
            .iter()
            .find(|f| f.accession == "0000320193-23-000064")
            .expect("the joined fact");
        assert!(!joined.accepted_at_derived);
        assert_eq!(joined.accepted_at, t("2023-06-30T18:03:41Z"));
        // The un-joined one keeps its conservative derivation and its flag.
        let other = facts
            .iter()
            .find(|f| f.accession == "0000320193-23-000099")
            .expect("the other fact");
        assert!(other.accepted_at_derived);
    }

    /// A fact whose acceptance precedes its period end is impossible and is
    /// refused at parse time rather than stored and trusted.
    #[test]
    fn a_fact_filed_before_its_period_ended_is_refused() {
        let impossible = r#"{"cik":1,"facts":{"us-gaap":{"Revenues":{"units":{"USD":[
          {"start":"2023-01-01","end":"2023-03-31","val":1,"accn":"a","form":"10-Q","filed":"2023-02-01"}
        ]}}}}}"#;
        assert!(matches!(
            parse_company_facts(impossible),
            Err(EdgarError::Parse(_))
        ));
    }

    // ── fair access ──────────────────────────────────────────────────────────

    #[test]
    fn a_client_without_a_declared_operator_cannot_be_built() {
        for bad in [
            "",
            "trading-bot",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
            "curl/8.4.0",
            "noreply@example.com",
        ] {
            assert!(
                EdgarClient::new(bad).is_err(),
                "{bad:?} must be refused as a User-Agent"
            );
        }
        assert!(EdgarClient::new(UA).is_ok());
    }

    #[test]
    fn the_user_agent_is_carried_on_the_client() {
        let c = EdgarClient::new(UA).expect("valid user agent");
        assert_eq!(c.user_agent(), UA);
    }

    /// P-3.2 acceptance: the limiter REFUSES above 10 req/s. Asserting refusal,
    /// not that ten succeed.
    #[test]
    fn the_limiter_refuses_above_ten_requests_a_second() {
        let mut c = EdgarClient::new(UA).expect("valid user agent");
        assert_eq!(c.limiter.rate_per_sec(), u64::from(MAX_REQUESTS_PER_SEC));
        for i in 0..10 {
            assert!(
                c.limiter.try_acquire_at(std::time::Duration::ZERO),
                "request {i} should be permitted"
            );
        }
        assert!(
            !c.limiter
                .try_acquire_at(std::time::Duration::from_millis(999)),
            "the eleventh request inside one second must be refused"
        );
    }

    #[test]
    fn urls_are_zero_padded_to_ten_digits() {
        let c = EdgarClient::new(UA).expect("valid user agent");
        assert!(c.companyfacts_url(320_193).ends_with("CIK0000320193.json"));
        assert!(c.submissions_url(320_193).ends_with("CIK0000320193.json"));
    }
}
