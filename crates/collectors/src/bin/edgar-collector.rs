//! Collect SEC EDGAR fundamentals for named filers into
//! `dataplane.fundamentals_fact` (Set P P-3.2).
//!
//! ```text
//! DATABASE_URL=postgres://… \
//! EDGAR_USER_AGENT="Operator Name operator@example.org" \
//!   cargo run -p collectors --bin edgar-collector -- 320193 789019
//! ```
//!
//! The User-Agent is the operator's (`EDGAR_USER_AGENT`, or `config/edgar.toml`'s
//! `user_agent` field), required, and validated by
//! `EdgarClient::new`: the SEC's fair-access rules require one that identifies
//! who is asking, so this binary refuses to start without it rather than
//! inventing one. The client's limiter holds requests to the published 10/s.

use collectors::fundamentals::{edgar, store};

/// The marker `config/edgar.toml` ships its stand-in value with.
const PLACEHOLDER: &str = "PLACEHOLDER";

/// The operator's User-Agent: `EDGAR_USER_AGENT`, else the `user_agent` field of
/// `EDGAR_CONFIG` (default `config/edgar.toml`).
fn user_agent() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(ua) = std::env::var("EDGAR_USER_AGENT") {
        return Ok(ua);
    }
    let path = std::env::var("EDGAR_CONFIG").unwrap_or_else(|_| "config/edgar.toml".into());
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    #[derive(serde::Deserialize)]
    struct Cfg {
        user_agent: String,
    }
    let cfg: Cfg = toml::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    Ok(cfg.user_agent)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pg_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let ua = user_agent()?;
    if ua.contains(PLACEHOLDER) && std::env::var("EDGAR_ALLOW_PLACEHOLDER").as_deref() != Ok("1") {
        return Err(
            "the SEC User-Agent is still the PLACEHOLDER in config/edgar.toml: set              EDGAR_USER_AGENT or edit the file with a real operator and contact address              (EDGAR_ALLOW_PLACEHOLDER=1 permits a short smoke run)"
                .into(),
        );
    }
    let ciks: Vec<u32> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    if ciks.is_empty() {
        return Err("name at least one CIK".into());
    }
    let client = edgar::EdgarClient::new(&ua)?;
    let pg = storage::postgres::connect(&pg_url).await?;
    for cik in ciks {
        let mut facts = client.fetch_company_facts(cik).await?;
        let acceptance = client.fetch_acceptance_times(cik).await?;
        edgar::join_acceptance(&mut facts, &acceptance);
        let observed = facts.iter().filter(|f| !f.knowledge.is_derived()).count();
        let written = store::insert_facts(&pg, &facts).await?;
        println!(
            "cik={cik} facts={} observed_acceptance={observed} written={written}",
            facts.len()
        );
    }
    Ok(())
}
