//! SEC EDGAR submissions and companyfacts/XBRL (DATA-005 §11 S3; Set P, P-3.2).
//!
//! **The knowledge time is acceptance, not the period.** `companyfacts` gives
//! each fact an `end` (the period it describes) and a `filed` date (when the
//! filing reached EDGAR). Joining on `end` hands a March number to a March
//! backtest even though nobody outside the company had it until May. Every fact
//! this module produces carries an acceptance stamp and nothing reads it without
//! an `as_of` — see [`dataplane::fundamentals`].
//!
//! `companyfacts` carries `filed` as a *date*, with no time of day, so a fact
//! built from it alone carries a **derived** knowledge time: the end of the filed
//! day. The exact acceptance instant lives in `submissions/CIK##########.json` as
//! `acceptanceDateTime`, and [`join_acceptance`] upgrades the derived stamp to an
//! observed one wherever the submissions index supplies it. Until it does, late
//! is a cost and early is a lookahead bug, so the derivation is late.
//!
//! **Fair access is binding, not advisory.** The SEC publishes a max request rate
//! of ten per second and requires a User-Agent that identifies the operator. Both
//! are enforced here: the User-Agent is a required constructor argument with a
//! format check, and the budget is [`VenueLimit::sec_edgar`], whose refusal is
//! asserted by a test. Getting banned from EDGAR costs more than the filings.
//!
//! ## Free-plan depth — re-verified 2026-09-22
//!
//! Verified against
//! `sec.gov/search-filings/edgar-search-assistance/accessing-edgar-data`:
//!
//! - **Cost:** free, with **no authentication, no API key and no plan tiers**.
//!   There is no shallower free tier to work around.
//! - **Rate:** "Current max request rate: 10 requests/second", and a declared
//!   user agent in request headers, in the form
//!   `Sample Company Name AdminContact@<sample company domain>.com`.
//! - **Depth:** `companyfacts` aggregates every XBRL concept a filer has reported.
//!   The SEC's page does **not** publish a start date, so this module does not
//!   claim one: the honest statement is that depth is per-filer and equals that
//!   filer's XBRL reporting history, which a census job should measure rather
//!   than assume.
//!
//! Fundamentals are therefore **backfillable** and do not join the FM-11
//! forward-collect argument.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use dataplane::fundamentals::{derived_acceptance, FundamentalFact};
use dataplane::source::Knowledge;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::ratelimit::{Denied, RateLimiter, VenueLimit};

/// EDGAR's structured-data host.
pub const EDGAR_BASE: &str = "https://data.sec.gov";

/// The `dataplane.source.name` this collector writes under. The row itself is a
/// migration this track does not write — see the report.
pub const SOURCE_NAME: &str = "sec_edgar";

/// The SEC's published ceiling. Not a suggestion.
pub const MAX_REQUESTS_PER_SEC: u64 = 10;

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
    /// The budget is spent and the request was **not** sent.
    #[error("rate limited: {0}")]
    RateLimited(Denied),
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
    limiter: RateLimiter,
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
            limiter: RateLimiter::new(VenueLimit::sec_edgar()),
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

    /// The budget this client spends against.
    #[must_use]
    pub const fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    #[must_use]
    pub fn companyfacts_url(&self, cik: u32) -> String {
        format!("{}/api/xbrl/companyfacts/CIK{cik:010}.json", self.base)
    }

    #[must_use]
    pub fn submissions_url(&self, cik: u32) -> String {
        format!("{}/submissions/CIK{cik:010}.json", self.base)
    }

    /// One budgeted, identified GET.
    ///
    /// The header is attached here, on the single path every request in this
    /// module takes, so there is no request that can omit it. The budget is
    /// spent before the request exists, so a refusal cannot be raced past.
    async fn get(&self, url: &str) -> Result<String, EdgarError> {
        self.limiter
            .try_acquire()
            .map_err(EdgarError::RateLimited)?;
        let response = self
            .http
            .get(url)
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
    /// Rate-limit refusal, transport, status and parse failures.
    pub async fn fetch_company_facts(&self, cik: u32) -> Result<Vec<FundamentalFact>, EdgarError> {
        let url = self.companyfacts_url(cik);
        let body = self.get(&url).await?;
        parse_company_facts(&body)
    }

    /// `accession -> acceptance instant` for `cik`'s recent filings.
    ///
    /// # Errors
    /// Rate-limit refusal, transport, status and parse failures.
    pub async fn fetch_acceptance_times(
        &self,
        cik: u32,
    ) -> Result<BTreeMap<String, DateTime<Utc>>, EdgarError> {
        let url = self.submissions_url(cik);
        let body = self.get(&url).await?;
        parse_acceptance_times(&body)
    }
}

/// Parses a `companyfacts` payload into facts with derived acceptance times.
///
/// # Errors
/// Malformed dates, and any fact that fails [`FundamentalFact::validate`] — a
/// fact accepted before its own period ended is refused rather than stored and
/// trusted.
pub fn parse_company_facts(body: &str) -> Result<Vec<FundamentalFact>, EdgarError> {
    // json-at-boundary: EDGAR's companyfacts REST response, not one of our envelopes
    let parsed = serde_json::from_str::<CompanyFactsResponse>(body)
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
                        Some(s) => Some(
                            NaiveDate::from_str(s)
                                .map_err(|e| EdgarError::Parse(format!("start {s:?}: {e}")))?,
                        ),
                        None => None,
                    };
                    let filed = NaiveDate::from_str(&entry.filed)
                        .map_err(|e| EdgarError::Parse(format!("filed {:?}: {e}", entry.filed)))?;
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
                        knowledge: derived_acceptance(filed),
                    };
                    fact.validate()
                        .map_err(|e| EdgarError::Parse(e.to_string()))?;
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
/// A malformed payload.
pub fn parse_acceptance_times(body: &str) -> Result<BTreeMap<String, DateTime<Utc>>, EdgarError> {
    // json-at-boundary: EDGAR's submissions index, the venue's own REST response
    let parsed = serde_json::from_str::<SubmissionsResponse>(body)
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
        // trusted; a naive string is left derived rather than guessed at, because
        // guessing four or five hours in the wrong direction is a lookahead bug.
        let Ok(parsed) = DateTime::parse_from_rfc3339(accepted) else {
            continue;
        };
        map.insert(accession.clone(), parsed.with_timezone(&Utc));
    }
    Ok(map)
}

/// Replaces derived acceptance stamps with observed ones wherever the submissions
/// index knows the accession.
///
/// A fact whose accession is absent keeps its conservative derived stamp, so a
/// dataset can still tell the two apart — and a fact whose observed instant would
/// contradict its own period (accepted before the period ended) keeps the derived
/// stamp too, because an impossible observation is not evidence.
pub fn join_acceptance(
    facts: &mut [FundamentalFact],
    acceptance: &BTreeMap<String, DateTime<Utc>>,
) {
    for fact in facts.iter_mut() {
        let Some(observed) = acceptance.get(&fact.accession) else {
            continue;
        };
        let derived = fact.knowledge;
        fact.knowledge = Knowledge::Observed { at: *observed };
        if fact.validate().is_err() {
            fact.knowledge = derived;
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
        assert!(facts[0].knowledge.is_derived());
        // End of the filed day, not midnight.
        assert_eq!(facts[0].accepted_at(), t("2023-06-30T23:59:59Z"));
    }

    /// The P-3.2 acceptance criterion: a request as of `t` never returns a value
    /// filed after `t`, tested with a fact whose period precedes its acceptance by
    /// a quarter.
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
        let august = fact_for_period_as_of(
            &facts,
            320_193,
            "Revenues",
            period,
            t("2023-08-01T00:00:00Z"),
        )
        .expect("the original print was knowable in August");
        assert_eq!(august.value, dec!(100));
        let december = fact_for_period_as_of(
            &facts,
            320_193,
            "Revenues",
            period,
            t("2023-12-01T00:00:00Z"),
        )
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
        assert!(!joined.knowledge.is_derived());
        assert!(joined.knowledge.is_forward_evidence());
        assert_eq!(joined.accepted_at(), t("2023-06-30T18:03:41Z"));
        // The un-joined one keeps its conservative derivation and its flag.
        let other = facts
            .iter()
            .find(|f| f.accession == "0000320193-23-000099")
            .expect("the other fact");
        assert!(other.knowledge.is_derived());
    }

    /// Adversarial: a submissions row that would place acceptance before the
    /// period ended is not evidence, and must not overwrite the conservative
    /// stamp with an impossible one.
    #[test]
    fn an_impossible_observed_acceptance_does_not_overwrite_the_derivation() {
        let mut facts = parse_company_facts(FACTS).expect("fixture parses");
        let submissions = r#"{"filings":{"recent":{
          "accessionNumber":["0000320193-23-000064"],
          "acceptanceDateTime":["2023-01-15T18:03:41Z"]
        }}}"#;
        let acceptance = parse_acceptance_times(submissions).expect("fixture parses");
        join_acceptance(&mut facts, &acceptance);
        let fact = facts
            .iter()
            .find(|f| f.accession == "0000320193-23-000064")
            .expect("the fact");
        assert!(
            fact.knowledge.is_derived(),
            "the impossible stamp was refused"
        );
        assert_eq!(fact.accepted_at(), t("2023-06-30T23:59:59Z"));
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

    /// P-3.2 acceptance: the limiter REFUSES above 10 req/s. The budget is the
    /// SEC's published number, quoted on [`VenueLimit::sec_edgar`], and the
    /// refusal itself is proven in `ratelimit`'s own tests against a manual clock.
    #[test]
    fn the_client_budget_is_the_published_ten_per_second() {
        let c = EdgarClient::new(UA).expect("valid user agent");
        let limit = c.limiter().limit();
        assert_eq!(limit.venue, "sec_edgar");
        assert!((limit.sustained_rps() - MAX_REQUESTS_PER_SEC as f64).abs() < 1e-9);
        assert_eq!(
            limit.source_quote,
            "Current max request rate: 10 requests/second."
        );
        for i in 0..MAX_REQUESTS_PER_SEC {
            assert!(
                c.limiter().try_acquire().is_ok(),
                "request {i} is inside the rate"
            );
        }
        assert!(
            c.limiter().try_acquire().is_err(),
            "the eleventh request in one second must be refused"
        );
    }

    #[test]
    fn urls_are_zero_padded_to_ten_digits() {
        let c = EdgarClient::new(UA).expect("valid user agent");
        assert!(c.companyfacts_url(320_193).ends_with("CIK0000320193.json"));
        assert!(c.submissions_url(320_193).ends_with("CIK0000320193.json"));
    }
}
