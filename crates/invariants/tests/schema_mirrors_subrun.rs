//! `0129_campaign_subrun.sql` is a mirror of `harness::subrun` and
//! `harness::context::projection`, and stays one (Set S task S-10.11).
//!
//! Same argument as `schema_mirrors_lifecycle.rs`, with one difference that
//! makes it matter more here: both of these columns record *what a sub-run was
//! allowed to see*. A role added in Rust with no CHECK to match is a sub-run
//! rejected at the boundary the first time it runs; a CHECK that permits a
//! projection name Rust no longer emits is a column that quietly accepts a value
//! nothing produces, which is the shape a stale guard takes.
//!
//! No database needed. The migration is read as text and the enums as code, so
//! this runs everywhere `cargo test` does.

use std::path::{Path, PathBuf};

use harness::context::projection::Projection;
use harness::subrun::Role;

fn migration() -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0129_campaign_subrun.sql");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The SQL with its `--` comments removed.
///
/// The lesson `schema_mirrors_lifecycle.rs` paid for: a scan that reads its own
/// prose finds whatever it forbids, and this file's prose names every value.
fn statements(sql: &str) -> String {
    sql.lines()
        .map(|l| match l.find("--") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `column IN (...)` list in the file, not merely the first.
///
/// `projection` is constrained on both tables. A helper that stopped at the
/// first would let the second drift, and the second is the one on the sub-run —
/// the row that records what the sub-run saw.
fn check_lists(sql: &str, column: &str) -> Vec<Vec<String>> {
    let needle = format!("{column} IN (");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(at) = sql[from..].find(&needle) {
        let start = from + at + needle.len();
        let close = sql[start..]
            .find(')')
            .unwrap_or_else(|| panic!("the `{column} IN (` list never closes"));
        out.push(
            sql[start..start + close]
                .split('\'')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect(),
        );
        from = start + close;
    }
    assert!(!out.is_empty(), "no `{column} IN (` CHECK in 0129");
    out
}

#[test]
fn the_role_check_is_the_rust_enum() {
    let sql = statements(&migration());
    let mut rust: Vec<String> = Role::ALL.iter().map(|r| r.as_str().to_string()).collect();
    rust.sort();
    for mut listed in check_lists(&sql, "role") {
        listed.sort();
        assert_eq!(
            listed, rust,
            "the sub-run role CHECK and `harness::subrun::Role` have drifted"
        );
    }
}

/// Both `projection` CHECKs, on both tables.
#[test]
fn every_projection_check_is_the_rust_enum() {
    let sql = statements(&migration());
    let mut rust = vec![
        Projection::Critic.as_str().to_string(),
        Projection::worker([]).as_str().to_string(),
        Projection::Full.as_str().to_string(),
    ];
    rust.sort();
    let lists = check_lists(&sql, "projection");
    assert_eq!(
        lists.len(),
        2,
        "0129 constrains `projection` on the campaign and on the sub-run; a table that \
         stopped constraining it would record what a sub-run saw as free text"
    );
    for mut listed in lists {
        listed.sort();
        assert_eq!(listed, rust);
    }
}

/// The projection a role requires is a value its column can hold.
///
/// The two enums are mirrored separately above, which leaves one way for them to
/// be individually correct and jointly wrong: `Role::requires_projection` naming
/// something the `projection` CHECK does not permit. Every worker sub-run would
/// then be refused by the database at dispatch.
#[test]
fn the_projection_each_role_requires_is_one_the_column_accepts() {
    let sql = statements(&migration());
    let permitted = check_lists(&sql, "projection")
        .into_iter()
        .next()
        .expect("at least one list");
    for role in Role::ALL {
        if let Some(required) = role.requires_projection() {
            assert!(
                permitted.iter().any(|p| p == required),
                "a `{}` sub-run must record projection `{required}`, which the column refuses",
                role.as_str()
            );
        }
    }
}

/// AT-59's exactly-once property, extended to the worker sub-run.
///
/// A campaign's state is the fold of its log, so a driver killed mid-phase
/// recomputes the same `seq` and therefore keys the same child job. The sub-run
/// must inherit that rather than invent its own: its unique index is
/// `(campaign_id, phase, seq)` — the same three values `child_key` hashes — so a
/// restarted driver claims the position it already holds instead of putting a
/// second proposer on one generation.
///
/// If the index gained or lost a column, two drivers could both claim, and the
/// campaign would record one `propose` decision for two proposals with no way to
/// tell which move it was about.
#[test]
fn the_sub_run_position_is_the_child_keys_position() {
    let sql = statements(&migration());
    let at = sql
        .find("sub_run_one_per_campaign_position")
        .expect("the position index exists");
    let open = sql[at..].find('(').expect("the index names its columns") + at;
    let close = sql[open..].find(')').expect("the column list closes") + open;
    let columns: Vec<String> = sql[open + 1..close]
        .split(',')
        .map(|c| c.trim().to_string())
        .collect();
    assert_eq!(
        columns,
        vec!["campaign_id", "phase", "seq"],
        "the sub-run's position must be the three values `CampaignState::child_key` hashes"
    );

    // And the behaviour those three give the key, so this is not only a spelling
    // check: same triple, same key; any one of them different, different key.
    use ledger::phase::{fold, CampaignEvent, CampaignPhase};
    let log = vec![
        CampaignEvent::new(CampaignPhase::Define, serde_json::json!({})),
        CampaignEvent::new(CampaignPhase::Baseline, serde_json::json!({})),
        CampaignEvent::new(CampaignPhase::Diagnose, serde_json::json!({})),
    ];
    let a = fold(&log).expect("folds");
    let b = fold(&log).expect("folds again");
    let campaign = uuid_from(1);
    assert_eq!(
        a.child_key(campaign, CampaignPhase::Hypothesize),
        b.child_key(campaign, CampaignPhase::Hypothesize),
        "two folds of one log must key the same sub-run"
    );
    assert_ne!(
        a.child_key(campaign, CampaignPhase::Hypothesize),
        a.child_key(uuid_from(2), CampaignPhase::Hypothesize)
    );
    assert_ne!(
        a.child_key(campaign, CampaignPhase::Hypothesize),
        a.child_key(campaign, CampaignPhase::Experiment)
    );
    let advanced = fold(
        &[
            log,
            vec![CampaignEvent::new(
                CampaignPhase::Hypothesize,
                serde_json::json!({}),
            )],
        ]
        .concat(),
    )
    .expect("folds");
    assert_ne!(
        a.child_key(campaign, CampaignPhase::Hypothesize),
        advanced.child_key(campaign, CampaignPhase::Hypothesize),
        "a later position in the fold is a different sub-run"
    );
}

fn uuid_from(n: u8) -> uuid::Uuid {
    let mut bytes = [0u8; 16];
    bytes[15] = n;
    uuid::Uuid::from_bytes(bytes)
}
