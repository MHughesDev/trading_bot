//! A digest kind is declared unreadable only when its source really is.
//!
//! # The failure this exists to catch
//!
//! The away digest carries a `kinds_not_covered` array whose whole purpose is
//! honesty: "no findings" and "findings are unreadable here" look identical in
//! an empty list, and only one of them means the operator can stop looking.
//!
//! The array was a **constant**, and two of its three rows had become false.
//!
//! * `finding` said findings "are not persisted in this deployment — the memory
//!   engine exists and its migrations do not", while `0122_semantic_memory.sql`,
//!   `0124_memory_retrievals.sql` and `0125_negative_result_class.sql` were all
//!   in `migrations/` and applied, and `routes/memory.rs` had been serving seven
//!   buckets over `agent_memory.entry`.
//! * `monitor` said the monitor tables "record current state, not state
//!   changes", while `0075_kill_switch.sql` creates `monitor.events` — one row
//!   per change, carrying `from_state`, `to_state`, `actor` and `reason`.
//!
//! Both were true when they were typed. Neither had any way to stop being
//! shipped once they stopped being true, because nothing in the build could
//! contradict a sentence in a string literal. So the operator was told, by the
//! system, in the one field designed to tell them when to stop looking, that a
//! store they could open in another tab did not exist.
//!
//! # The two halves
//!
//! **Structural.** For every digest kind, name the table its lines come from.
//! If a migration in the tree creates that table, the route must contain a
//! `FROM` against it. A source that exists and is not read is a source the route
//! is about to declare uncovered.
//!
//! **Behavioural.** [`kinds_not_covered`] is called directly: if every source
//! answered, the array must be empty, and a row that does appear must quote what
//! the source actually said. A reason derived from a failed read expires by
//! itself; a reason typed into a literal does not.
//!
//! # Not a database test
//!
//! Deliberately. The claim is about the *code's* relationship to the schema, and
//! it has to fail in CI on a machine with no Postgres — which is exactly where a
//! stale claim about "this deployment" would otherwise survive.
//!
//! One consequence worth stating: this proves the route reads the table, not
//! that the deployment has rows in it. Both stores were empty when this was
//! written, and an empty section with an empty `kinds_not_covered` is the route
//! saying "I looked and there was nothing", which is the answer that was wanted.

use std::path::{Path, PathBuf};

use api::routes::desk_digest::{budget_line, kinds_not_covered, Budget};

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `crates/api`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/api sits two levels under the repo root")
        .to_path_buf()
}

/// Every `DigestKind` in `frontend/src/api/desk.ts`, with the table its lines
/// are read from and the migration that creates it.
///
/// A row here is a falsifiable claim: if the migration no longer creates the
/// table, the first assertion below stops applying and says so rather than
/// silently passing.
const SOURCES: &[(&str, &str)] = &[
    ("concluded", "agent_runs"),
    ("finding", "agent_memory.entry"),
    ("refuted", "agent_memory.entry"),
    ("approval_defaulted", "approval_requests"),
    ("monitor", "monitor.events"),
    ("budget", "llm_usage"),
    ("waiting", "approval_requests"),
];

fn migrations_create(table: &str) -> bool {
    let dir = repo_root().join("migrations");
    let needle = format!("create table if not exists {}", table.to_lowercase());
    let bare = format!("create table {}", table.to_lowercase());
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "sql"))
        .any(|e| {
            let sql = std::fs::read_to_string(e.path()).unwrap_or_default().to_lowercase();
            sql.contains(&needle) || sql.contains(&bare)
        })
}

fn route_source() -> String {
    let p = repo_root()
        .join("crates/api/src/routes/desk_digest.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The structural half: a source that exists is a source the route reads.
#[test]
fn a_kind_whose_table_a_migration_creates_is_read_and_not_declared_missing() {
    let route = route_source();
    for (kind, table) in SOURCES {
        assert!(
            migrations_create(table),
            "`{kind}` is documented as reading `{table}` and no migration creates it — either \
             the table was renamed and this row is stale, or the kind has no source at all"
        );
        assert!(
            route.contains(&format!("FROM {table}")),
            "`{table}` exists in `migrations/` and `desk_digest.rs` never selects from it, so \
             the `{kind}` line has a source and does not use it. That is the shape of the defect \
             this test was written for: the route told the operator findings were not persisted \
             while `0122_semantic_memory.sql` sat in the tree, applied."
        );
    }
}

/// The digest's own read of the memory store carries the store's filter.
///
/// `routes/memory.rs` argues against a second query over `agent_memory.entry`:
/// "two routes over one table is two places for the archived-and-superseded
/// filter to be forgotten, and that filter is what stops a retracted entry being
/// read as live."
///
/// The digest needs one anyway, because `Entry` carries no `created_at` and a
/// digest is a question about a window, so `MemoryStore` cannot answer it. This
/// is the price of that: the second copy exists and a test holds it to the
/// first. A digest that surfaced a superseded claim as news would be worse than
/// one that surfaced nothing.
#[test]
fn the_digests_memory_read_filters_what_the_store_filters() {
    let route = route_source();
    let store = std::fs::read_to_string(repo_root().join("crates/api/src/memory_store.rs"))
        .expect("the store is where the filter is defined");
    for half in ["NOT archived", "superseded_by IS NULL"] {
        assert!(
            store.contains(half),
            "`{half}` is no longer in `MemoryStore`, so this test is comparing against a rule \
             that moved — find where the live-entry filter lives now and update both"
        );
        assert!(
            route.contains(half),
            "`desk_digest.rs` reads `agent_memory.entry` without `{half}`. An archived or \
             superseded entry would then appear in the digest as something newly learned, which \
             is a retracted claim presented as a finding."
        );
    }
}

/// The behavioural half: nothing is declared uncovered when everything answered.
///
/// This is the assertion the pre-fix route could not satisfy under any input —
/// its array was a constant of three rows.
#[test]
fn every_source_answering_leaves_no_kind_declared_uncovered() {
    for budget in [
        Budget::Idle,
        Budget::Priced {
            usd: 3.25,
            calls: 41,
        },
        Budget::TokensOnly {
            tokens_in: 2_526_695,
            tokens_out: 27_512,
            runs: 27,
            runs_without_counters: 3,
        },
    ] {
        let gaps = kinds_not_covered(&Ok(()), &Ok(()), &budget);
        assert!(
            gaps.is_empty(),
            "every source answered and the digest still claims a blind spot ({budget:?}): \
             {gaps:?}. A kind named here tells the operator to keep looking somewhere the \
             platform can in fact see."
        );
    }
}

/// A declared gap names the failure it observed, in the words the source used.
#[test]
fn a_declared_gap_quotes_the_read_that_failed() {
    let gaps = kinds_not_covered(
        &Err("relation \"agent_memory.entry\" does not exist".into()),
        &Err("permission denied for schema monitor".into()),
        &Budget::Unreadable("pool timed out".into()),
    );
    let kinds: Vec<&str> = gaps.iter().filter_map(|g| g["kind"].as_str()).collect();
    assert_eq!(
        kinds,
        ["finding", "refuted", "monitor", "budget"],
        "a source that failed must produce a row for every kind that reads it"
    );
    let joined = gaps
        .iter()
        .filter_map(|g| g["why"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for quoted in [
        "does not exist",
        "permission denied for schema monitor",
        "pool timed out",
    ] {
        assert!(
            joined.contains(quoted),
            "a reason that does not quote the failure is a standing claim about the deployment, \
             and a standing claim cannot expire: {joined}"
        );
    }
    assert!(
        !joined.contains("migrations"),
        "a reason must not rest on what migrations do or do not exist — that is precisely the \
         sentence that outlived its truth by three of them: {joined}"
    );
}

/// An unpriced window with runs in it produces no figure and one stated gap.
///
/// `llm_usage` is written only by the LLM proxy. A driver that does not route
/// through it leaves the table empty, `SUM` over no rows is `0`, and a `0`
/// printed beside "spent while you were away" is the platform telling the
/// operator their research was free.
#[test]
fn an_unmeasured_spend_is_a_stated_gap_and_never_a_dollar_zero() {
    let b = Budget::Unmeasured { runs: 55 };
    assert!(
        budget_line(&b, chrono::Utc::now()).is_none(),
        "an unmeasured spend must not become a budget line; the only figure available is 0 and \
         an absent measurement is never a zero"
    );
    let gaps = kinds_not_covered(&Ok(()), &Ok(()), &b);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0]["kind"], "budget");
    assert!(gaps[0]["why"].as_str().unwrap().contains("55"));
}
