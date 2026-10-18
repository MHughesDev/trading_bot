//! Where EDGAR facts land, and the one way they are read back (P-3.2; DATA-005
//! §10.4, §12).
//!
//! The read is `accepted_at <= as_of`, always, and there is no function here that
//! takes a fact without an `as_of`. Within that, the value for a period is the one
//! from the **latest filing accepted by `as_of`** — so a 10-K/A restating a 10-K is
//! invisible before it was accepted and wins after, and the original is still in
//! the table (migration 0156 refuses UPDATE and DELETE).

use chrono::{DateTime, NaiveDate, Utc};
use dataplane::fundamentals::FundamentalFact;
use rust_decimal::Decimal;
use sqlx::PgPool;

/// The source row these facts are attributed to (migration 0156).
pub const SOURCE: &str = "sec_edgar";

/// One fact as read back.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow, serde::Serialize)]
pub struct StoredFact {
    pub cik: i64,
    pub tag: String,
    pub unit: String,
    pub period_start: Option<NaiveDate>,
    pub period_end: NaiveDate,
    pub form: String,
    pub accession: String,
    pub value: Decimal,
    pub accepted_at: DateTime<Utc>,
    pub knowledge: String,
}

/// Insert facts. Idempotent on `(cik, tag, unit, period_end, accession)`; a fact
/// that fails its own validation is refused rather than written.
///
/// # Errors
/// A fact that violates `FundamentalFact::validate`, or the database.
pub async fn insert_facts(pg: &PgPool, facts: &[FundamentalFact]) -> Result<u64, sqlx::Error> {
    let mut n = 0;
    for f in facts {
        if let Err(e) = f.validate() {
            return Err(sqlx::Error::Protocol(format!(
                "refusing fundamentals fact {} {}: {e}",
                f.cik, f.tag
            )));
        }
        let r = sqlx::query(
            "INSERT INTO dataplane.fundamentals_fact
               (cik, tag, unit, period_start, period_end, fiscal_year, fiscal_period, form,
                accession, value, accepted_at, knowledge, source_id)
             SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12, s.source_id
               FROM dataplane.source s WHERE s.name = $13
             ON CONFLICT (cik, tag, unit, period_end, accession) DO NOTHING",
        )
        .bind(i64::from(f.cik))
        .bind(&f.tag)
        .bind(&f.unit)
        .bind(f.period_start)
        .bind(f.period_end)
        .bind(f.fiscal_year)
        .bind(&f.fiscal_period)
        .bind(&f.form)
        .bind(&f.accession)
        .bind(f.value)
        .bind(f.knowledge.time())
        .bind(if f.knowledge.is_derived() {
            "derived"
        } else {
            "observed"
        })
        .bind(SOURCE)
        .execute(pg)
        .await?;
        n += r.rows_affected();
    }
    Ok(n)
}

/// The facts for one filer and tag **as they stood at `as_of`**: per
/// `(unit, period_end)`, the value from the latest filing accepted on or before
/// `as_of`. Nothing accepted after `as_of` can appear — DATA-005 §10.4.
///
/// # Errors
/// The database.
pub async fn facts_as_of(
    pg: &PgPool,
    cik: i64,
    tag: &str,
    as_of: DateTime<Utc>,
) -> Result<Vec<StoredFact>, sqlx::Error> {
    sqlx::query_as(
        "SELECT DISTINCT ON (unit, period_end)
                cik, tag, unit, period_start, period_end, form, accession, value,
                accepted_at, knowledge
           FROM dataplane.fundamentals_fact
          WHERE cik = $1 AND tag = $2 AND accepted_at <= $3
          ORDER BY unit, period_end, accepted_at DESC, accession DESC",
    )
    .bind(cik)
    .bind(tag)
    .bind(as_of)
    .fetch_all(pg)
    .await
}
