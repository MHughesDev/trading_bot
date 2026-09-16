//! `0122_semantic_memory.sql` is a mirror of `harness::memory`, and stays one.
//!
//! The migration's own header calls itself a mirror rather than a second
//! opinion: every CHECK in it restates a rule `Entry::validate` already
//! enforces, because the acceptance for S-4.3 is that an entry without
//! provenance *cannot be inserted* — a claim about the database, not about one
//! caller. A rule enforced only in the writer holds until the second writer.
//!
//! That argument is worth exactly nothing if the two drift, and drift here is
//! silent in the worst direction: add a `Kind` in Rust, forget the CHECK, and
//! every write of that kind fails at runtime against a constraint nobody was
//! looking at. Or widen the CHECK and forget the Rust, and the database accepts
//! a kind nothing can read back.
//!
//! So this compares them. It needs no database — it reads the migration as text
//! and the enums as code — which matters, because the acceptance's own
//! `db_invariants` run is `--ignored` and needs Postgres, and a guard that only
//! runs where Postgres does is a guard that does not run.

use std::path::{Path, PathBuf};

fn migration() -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0122_semantic_memory.sql");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The quoted strings inside the `IN (...)` list that follows `column IN (`.
fn check_list(sql: &str, column: &str) -> Vec<String> {
    let at = sql
        .find(&format!("{column} IN ("))
        .unwrap_or_else(|| panic!("no `{column} IN (` CHECK in the migration"));
    let rest = &sql[at..];
    let close = rest.find(')').expect("the IN list closes");
    rest[..close]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The migration with its `--` comments removed and whitespace collapsed.
///
/// Both halves are lessons already paid for in this repository. **Comments must
/// go** because a scan that reads its own prose finds whatever it forbids: this
/// file's first run failed on the word `winner`, which appears only in the
/// migration's comment explaining why there is no such column. **Whitespace must
/// collapse** because a fragment like `prov_run_id       TEXT NOT NULL` pins the
/// column alignment, so re-indenting the schema would fail a test about
/// provenance.
fn statements(sql: &str) -> String {
    let stripped: String = sql
        .lines()
        .map(|l| match l.find("--") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join(" ");
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_mirrors(sql: &str, column: &str, variants: &[&str]) {
    let mut in_sql = check_list(sql, column);
    in_sql.sort();
    let mut in_rust: Vec<String> = variants.iter().map(|s| (*s).to_string()).collect();
    in_rust.sort();
    assert_eq!(
        in_sql, in_rust,
        "`{column}`'s CHECK list and the Rust enum disagree. Left is the migration, right is \
         `harness::memory`. A variant in Rust and not in the CHECK fails every insert of that \
         kind; one in the CHECK and not in Rust is a row nothing can read back."
    );
}

#[test]
fn the_kind_check_lists_exactly_the_rust_kinds() {
    use harness::memory::Kind;
    let all = [
        Kind::Finding,
        Kind::NegativeResult,
        Kind::InstrumentProfile,
        Kind::SignalRegistry,
        Kind::PlatformDefect,
        Kind::UserPreference,
        Kind::OpenQuestion,
    ];
    let names: Vec<&str> = all.iter().map(|k| k.as_str()).collect();
    assert_mirrors(&migration(), "kind", &names);
}

#[test]
fn the_confidence_check_lists_exactly_the_rust_levels() {
    use harness::memory::Confidence;
    let names: Vec<&str> = [
        Confidence::Established,
        Confidence::Suggestive,
        Confidence::Refuted,
    ]
    .iter()
    .map(|c| c.as_str())
    .collect();
    assert_mirrors(&migration(), "confidence", &names);
}

/// The constraints `Entry::validate` raises, each present in the schema.
///
/// Matched on the SQL rather than on a comment, because a comment saying
/// "enforced by a CHECK" beside no CHECK is the exact failure this repository
/// keeps finding.
#[test]
fn every_validate_rule_has_a_constraint_behind_it() {
    let sql = statements(&migration());
    let required: [(&str, &str); 7] = [
        ("EntryError::NoProvenance", "prov_run_id TEXT NOT NULL"),
        ("EntryError::NoProvenance", "prov_tool TEXT NOT NULL"),
        ("EntryError::NoProvenance", "prov_tool_version TEXT NOT NULL"),
        ("EntryError::NoProvenance", "prov_template_hash TEXT NOT NULL"),
        ("EntryError::ClaimTooLong", "char_length(claim) <= 300"),
        ("EntryError::EmptySubject", "btrim(subject) <> ''"),
        ("EntryError::NoWindow", "window_start DATE NOT NULL"),
    ];
    for (rule, fragment) in required {
        assert!(
            sql.contains(fragment),
            "{rule} is enforced in Rust and the migration has no `{fragment}`, so the database \
             would accept what `Entry::validate` refuses"
        );
    }
}

/// §2.3's rule, which is the one a second writer would most plausibly break: a
/// model's summary of something it read is never `established`.
#[test]
fn a_model_summary_can_never_be_established_in_the_database() {
    let sql = statements(&migration());
    assert!(
        sql.contains("a_model_summary_is_never_established"),
        "the named CHECK is gone. It is the only thing stopping a writer from promoting a \
         model's own summary to `established`, which is the whole distinction the evidence \
         classes exist to draw."
    );
}

/// The store must not grow a way to resolve a conflict.
///
/// §2.2.1 says conflicts are surfaced and never merged. A `winner` or
/// `resolution` column would be an invitation, and the absence is the design.
#[test]
fn the_schema_offers_no_way_to_resolve_a_conflict() {
    let sql = statements(&migration()).to_lowercase();
    for forbidden in ["winner", "resolved_by", "resolution"] {
        assert!(
            !sql.contains(forbidden),
            "the schema has a `{forbidden}`. AGENT-007 §2.2.1 says conflicts are surfaced and \
             never merged — a column that records which side won is a merge with a record of \
             the loser."
        );
    }
}
