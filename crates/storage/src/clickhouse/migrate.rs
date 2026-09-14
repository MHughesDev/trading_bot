//! ClickHouse schema migrations, applied on platform boot (Set L, L-0.2).
//!
//! The `clickhouse/` directory is mounted at `/docker-entrypoint-initdb.d` in
//! `docker-compose.yml`, so those files run **only when the data volume is first
//! created**. Every developer box with an existing volume — which is all of them —
//! never sees a new DDL file. That is how `market_bars_v2` (DATA-005 §3) would
//! silently fail to exist on the very machines that hold the data it is meant to
//! protect.
//!
//! This module closes that gap the way Postgres already does it
//! (`storage::postgres::run_migrations`): the DDL is embedded at compile time and
//! replayed on every boot. Every statement is `CREATE TABLE IF NOT EXISTS`, so
//! replay is a no-op once the table exists.
//!
//! **Adding a DDL file:** write it in `clickhouse/`, then add it to [`MIGRATIONS`].
//! Both are required — the directory mount serves fresh volumes, this list serves
//! existing ones.

use tracing::{debug, info};

use super::ChError;

/// The ClickHouse DDL files, in apply order.
///
/// Embedded with `include_str!` so a binary carries its own schema and cannot drift
/// from the checked-out tree.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "01_trades.sql",
        include_str!("../../../../clickhouse/01_trades.sql"),
    ),
    (
        "02_bars.sql",
        include_str!("../../../../clickhouse/02_bars.sql"),
    ),
    (
        "03_model_predictions.sql",
        include_str!("../../../../clickhouse/03_model_predictions.sql"),
    ),
    (
        "04_model_traces.sql",
        include_str!("../../../../clickhouse/04_model_traces.sql"),
    ),
    (
        "05_backtest_run_series.sql",
        include_str!("../../../../clickhouse/05_backtest_run_series.sql"),
    ),
    (
        "06_market_bars_v2.sql",
        include_str!("../../../../clickhouse/06_market_bars_v2.sql"),
    ),
    (
        "07_canonical_market.sql",
        include_str!("../../../../clickhouse/07_canonical_market.sql"),
    ),
];

/// Applies every DDL file in [`MIGRATIONS`], in order.
///
/// Idempotent: all statements are `CREATE TABLE IF NOT EXISTS`.
///
/// `url` is the full ClickHouse URL, credentials and database included, e.g.
/// `http://trading:trading@clickhouse:8123/trading`.
pub async fn run_migrations(url: &str) -> Result<(), ChError> {
    let client = super::connect(url);
    let mut applied = 0usize;

    for (name, sql) in MIGRATIONS {
        for statement in split_statements(sql) {
            client
                .query(&statement)
                .execute()
                .await
                .map_err(|e| ChError::Client(format!("{name}: {e}")))?;
            applied += 1;
        }
        debug!(file = name, "clickhouse ddl applied");
    }

    // Schema corrections that a CREATE TABLE IF NOT EXISTS cannot express, because
    // the table already exists with the wrong shape.
    ensure_market_bars_v2_engine(url).await?;

    info!(
        files = MIGRATIONS.len(),
        statements = applied,
        "clickhouse schema migrations applied"
    );
    Ok(())
}

/// Sorting key and engine the v2 bar table must have (`clickhouse/06_market_bars_v2.sql`).
const V2_ENGINE: &str = "ReplacingMergeTree";
const V2_ORDER_BY: &str = "(instrument_id, timeframe, venue_id, source, event_time, revision)";

/// Rebuilds `market_bars_v2` when it still has the first engine it shipped with.
///
/// The table was created as a plain `MergeTree` so that no sorting-key mistake could
/// ever destroy a row again. That reasoning was sound but incomplete: re-collecting a
/// bar is routine — gap fill re-reads ranges on every boot — and an append-only table
/// keeps every repeat forever. Hours after the cutover the live table already held
/// 1,688 exact re-collections: same `event_id`, same revision, differing only in
/// `ingested_time`.
///
/// The fix is not to go back to v1's mistake. v1 collapsed rows that differed in
/// `timeframe`, because `timeframe` was not in its key. Here the key is complete —
/// instrument, timeframe, venue, source, close time, revision — so the only rows that
/// can merge are genuine repeats of the same observation.
///
/// A ClickHouse table's engine cannot be altered in place, so this builds the correct
/// table alongside, copies the deduplicated rows, and swaps the two names in one
/// atomic `EXCHANGE TABLES`. The old table is kept as `market_bars_v2_premerge_<ts>`
/// rather than dropped: it is the only copy of any row this step gets wrong.
pub async fn ensure_market_bars_v2_engine(url: &str) -> Result<bool, ChError> {
    let client = super::connect(url);

    let current: Option<(String, String)> = client
        .query(
            "SELECT engine, sorting_key FROM system.tables \
             WHERE database = currentDatabase() AND name = 'market_bars_v2'",
        )
        .fetch_optional()
        .await
        .map_err(|e| ChError::Client(format!("inspect market_bars_v2: {e}")))?;

    let Some((engine, sorting_key)) = current else {
        // Nothing to migrate: a fresh install gets the right shape from the DDL.
        return Ok(false);
    };

    let key_matches = normalise_key(&sorting_key) == normalise_key(V2_ORDER_BY);
    if engine == V2_ENGINE && key_matches {
        return Ok(false);
    }

    info!(
        current_engine = %engine,
        current_key = %sorting_key,
        "rebuilding market_bars_v2 with the corrected engine and sorting key"
    );

    let stamp = chrono_stamp();
    let staging = format!("market_bars_v2_rebuild_{stamp}");
    let archive = format!("market_bars_v2_premerge_{stamp}");

    client
        .query(&format!(
            "CREATE TABLE {staging} AS market_bars_v2 \
             ENGINE = ReplacingMergeTree(ingested_time) \
             ORDER BY {V2_ORDER_BY} \
             PARTITION BY (timeframe, toYYYYMM(event_time))"
        ))
        .execute()
        .await
        .map_err(|e| ChError::Client(format!("create {staging}: {e}")))?;

    // Copy every distinct row. `DISTINCT` here is belt and braces — the new engine
    // would collapse the repeats on merge anyway — but doing it at copy time means
    // the row counts can be compared immediately rather than after a merge that may
    // not have happened yet.
    client
        .query(&format!(
            "INSERT INTO {staging} SELECT DISTINCT * FROM market_bars_v2"
        ))
        .execute()
        .await
        .map_err(|e| ChError::Client(format!("copy into {staging}: {e}")))?;

    // Every distinct bar must have survived. Comparing distinct (instrument,
    // timeframe, venue, source, event_time, revision) tuples is the right check:
    // total row counts are expected to shrink, which is the point.
    let before: u64 = distinct_observations(&client, "market_bars_v2").await?;
    let after: u64 = distinct_observations(&client, &staging).await?;
    if before != after {
        return Err(ChError::Client(format!(
            "refusing to swap market_bars_v2: {before} distinct observations before, \
             {after} after. The rebuilt table is in {staging}; nothing was swapped."
        )));
    }

    client
        .query(&format!("EXCHANGE TABLES market_bars_v2 AND {staging}"))
        .execute()
        .await
        .map_err(|e| ChError::Client(format!("exchange market_bars_v2 with {staging}: {e}")))?;

    client
        .query(&format!("RENAME TABLE {staging} TO {archive}"))
        .execute()
        .await
        .map_err(|e| ChError::Client(format!("archive old table as {archive}: {e}")))?;

    info!(
        distinct_observations = after,
        archive = %archive,
        "market_bars_v2 rebuilt; previous table kept for inspection"
    );
    Ok(true)
}

async fn distinct_observations(client: &super::ChClient, table: &str) -> Result<u64, ChError> {
    client
        .query(&format!(
            "SELECT uniqExact((instrument_id, timeframe, venue_id, source, event_time, revision)) \
             FROM {table}"
        ))
        .fetch_one()
        .await
        .map_err(|e| ChError::Client(format!("count distinct in {table}: {e}")))
}

/// Compares sorting keys ignoring whitespace, since ClickHouse reports them without
/// the parentheses and spacing the DDL was written with.
fn normalise_key(key: &str) -> String {
    key.chars()
        .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')')
        .collect()
}

fn chrono_stamp() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M%S").to_string()
}

/// Splits a DDL file into executable statements.
///
/// ClickHouse's HTTP interface takes one statement per request, so the file has to
/// be split on `;`. Naively splitting is wrong: `06_market_bars_v2.sql` documents
/// the canonical `argMax` read shape in a comment, and that comment ends in a
/// semicolon. Splitting there produces a fragment of SQL-looking prose that fails
/// to parse.
///
/// So line comments are stripped first, and the `--` that starts one is only
/// honoured outside a single-quoted string — `DateTime64(9, 'UTC')` must survive
/// intact.
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;

    for line in sql.lines() {
        let mut code = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\'' {
                in_string = !in_string;
                code.push(c);
                continue;
            }
            if !in_string && c == '-' && chars.peek() == Some(&'-') {
                break; // rest of the line is a comment
            }
            code.push(c);
        }

        current.push_str(&code);
        current.push('\n');

        // A statement can only end outside a string literal.
        if !in_string {
            while let Some(idx) = current.find(';') {
                let statement = current[..idx].trim().to_string();
                if !statement.is_empty() {
                    out.push(statement);
                }
                current = current[idx + 1..].to_string();
            }
        }
    }

    let tail = current.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_semicolons() {
        let sql = "CREATE TABLE a (x Int64) ENGINE = MergeTree ORDER BY x;\n\
                   CREATE TABLE b (y Int64) ENGINE = MergeTree ORDER BY y;";
        let statements = split_statements(sql);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("CREATE TABLE a"));
        assert!(statements[1].starts_with("CREATE TABLE b"));
    }

    #[test]
    fn strips_line_comments() {
        let sql = "-- a leading comment\nCREATE TABLE a (x Int64);\n-- trailing\n";
        let statements = split_statements(sql);
        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "CREATE TABLE a (x Int64)");
    }

    /// The regression this splitter exists for: a semicolon inside a comment must
    /// not end a statement. `06_market_bars_v2.sql` documents the canonical read
    /// shape in a trailing comment that ends with `;`.
    #[test]
    fn semicolon_inside_a_comment_is_not_a_statement_boundary() {
        let sql = "CREATE TABLE a (x Int64);\n\
                   -- SELECT argMax(open, (revision, ingested_time)) FROM t GROUP BY x;\n\
                   -- NOTE: another sentence.\n";
        let statements = split_statements(sql);
        assert_eq!(
            statements.len(),
            1,
            "comment semicolons must not split statements, got {statements:?}"
        );
    }

    /// `--` inside a string literal is data, not a comment.
    #[test]
    fn double_dash_inside_a_string_literal_survives() {
        let sql = "CREATE TABLE a (t DateTime64(9, 'UTC'), s String DEFAULT 'a--b');";
        let statements = split_statements(sql);
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("'UTC'"), "got {}", statements[0]);
        assert!(statements[0].contains("'a--b'"), "got {}", statements[0]);
    }

    /// Every embedded file must parse into at least one statement, and none of them
    /// may produce an empty fragment. This catches a malformed DDL file at test
    /// time rather than at boot.
    #[test]
    fn every_embedded_migration_splits_cleanly() {
        for (name, sql) in MIGRATIONS {
            let statements = split_statements(sql);
            assert!(!statements.is_empty(), "{name} produced no statements");
            for statement in &statements {
                assert!(!statement.trim().is_empty(), "{name} produced an empty one");
                assert!(
                    statement
                        .to_uppercase()
                        .contains("CREATE TABLE IF NOT EXISTS"),
                    "{name}: every statement must be idempotent, got: {statement}"
                );
            }
        }
    }

    /// The v2 bar table is the reason this module exists; assert its shape is what
    /// DATA-005 §3 requires, so a careless edit to the DDL trips a test.
    #[test]
    fn market_bars_v2_key_covers_every_dimension_that_distinguishes_a_bar() {
        let (_, sql) = MIGRATIONS
            .iter()
            .find(|(name, _)| *name == "06_market_bars_v2.sql")
            .expect("06_market_bars_v2.sql must be registered");
        let statements = split_statements(sql);
        let ddl = statements
            .iter()
            .find(|s| s.contains("market_bars_v2"))
            .expect("market_bars_v2 DDL");

        // v1's bug was an INCOMPLETE key, not the engine: it collapsed rows that
        // differed in timeframe because timeframe was not in the key. Every
        // dimension that makes two bars different observations must appear here, or
        // that bug comes back wearing different clothes.
        for dimension in [
            "instrument_id",
            "timeframe",
            "venue_id",
            "source",
            "event_time",
            "revision",
        ] {
            let order_by = &ddl[ddl.find("ORDER BY").expect("a sorting key")..];
            assert!(
                order_by.contains(dimension),
                "{dimension} must be in the sorting key or bars differing only in it                  will be destroyed on merge: {ddl}"
            );
        }
        assert!(
            ddl.contains("ORDER BY (instrument_id, timeframe,"),
            "instrument and timeframe lead the key: {ddl}"
        );
    }
}
