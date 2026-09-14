//! ClickHouse client + batched insert helpers.
pub mod backfill;
pub mod canonical;
pub mod features;
pub mod migrate;
pub mod model_traces;
pub mod trades;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChError {
    #[error("clickhouse: {0}")]
    Client(String),
    #[error("insert: {0}")]
    Insert(String),
}

pub type ChClient = clickhouse::Client;

/// Builds a client from a full ClickHouse URL, credentials and database included:
/// `http://user:pass@host:8123/dbname`.
///
/// The `clickhouse` crate does **not** parse user, password or database out of the
/// URL — it wants them set through dedicated builder methods, and it rejects a path
/// component on the HTTP route. Passing the raw URL therefore connects as `default`
/// to the `default` database, which looks like it works right up until the writes
/// land somewhere nobody is reading. So the parts are split out here.
///
/// Mirrors the parsing in `backtest::store::BarStore::connect`.
pub fn connect(url: &str) -> ChClient {
    let mut client = clickhouse::Client::default();

    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let scheme = if url.starts_with("https") {
        "https"
    } else {
        "http"
    };

    // "user:pass@host:port/db" → (credentials, host_and_path)
    let (creds, host_path) = match after_scheme.rfind('@') {
        Some(at) => (Some(&after_scheme[..at]), &after_scheme[at + 1..]),
        None => (None, after_scheme),
    };

    // host[:port] and an optional /database
    let (host_port, db) = match host_path.find('/') {
        Some(slash) => (&host_path[..slash], Some(&host_path[slash + 1..])),
        None => (host_path, None),
    };

    client = client.with_url(format!("{scheme}://{host_port}"));

    if let Some(cred_str) = creds {
        match cred_str.split_once(':') {
            Some((user, pass)) => client = client.with_user(user).with_password(pass),
            None => client = client.with_user(cred_str),
        }
    }

    if let Some(database) = db.filter(|d| !d.is_empty()) {
        client = client.with_database(database);
    }

    client
}
