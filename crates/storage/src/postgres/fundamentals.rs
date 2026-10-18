//! EDGAR facts as a decision may read them (Set P P-3.6).

use sqlx::Row;

/// Every fact of `(cik, tag)` accepted on or before `as_of`, **all filings
/// kept**: which one a decision sees is the feature runtime's choice by
/// acceptance time (`latest_as_of`), so a restatement filed between two
/// decisions is seen by the second only.
///
/// # Errors
/// A database error.
pub async fn facts(
    pool: &super::PgPool,
    cik: u32,
    tag: &str,
    as_of: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<dataplane::fundamentals::FundamentalFact>, super::PgError> {
    let rows = sqlx::query(
        "SELECT unit, period_start, period_end, fiscal_year, fiscal_period, form, accession,
                value, accepted_at, knowledge
           FROM dataplane.fundamentals_fact
          WHERE cik = $1 AND tag = $2 AND accepted_at <= $3
          ORDER BY period_end, accepted_at, accession",
    )
    .bind(i64::from(cik))
    .bind(tag)
    .bind(as_of)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let at: chrono::DateTime<chrono::Utc> = r.get("accepted_at");
            let derived = r.get::<String, _>("knowledge") == "derived";
            dataplane::fundamentals::FundamentalFact {
                cik,
                tag: tag.to_owned(),
                unit: r.get("unit"),
                period_start: r.get("period_start"),
                period_end: r.get("period_end"),
                fiscal_year: r.get("fiscal_year"),
                fiscal_period: r.get("fiscal_period"),
                form: r.get("form"),
                accession: r.get("accession"),
                value: r.get("value"),
                knowledge: if derived {
                    dataplane::source::Knowledge::Derived {
                        at,
                        from_event_time: at,
                        declared_lag: std::time::Duration::ZERO,
                    }
                } else {
                    dataplane::source::Knowledge::Observed { at }
                },
            }
        })
        .collect())
}
