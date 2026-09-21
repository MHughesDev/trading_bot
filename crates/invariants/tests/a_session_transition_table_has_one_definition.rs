//! The session transition table is written twice and must mean one thing
//! (AGENT-014 §2.1, §7; Set S task S-11.1).
//!
//! # Why it is written twice at all
//!
//! Once in Rust, in `research_session::fold::transition_ok`, because the fold
//! needs it to decide whether a log it is replaying makes sense — and a fold
//! that had to ask the database what it thinks of each event would not be a fold.
//!
//! Once in SQL, as `session_transition_ok`, because the trigger has to hold the
//! rule against **every** writer. A rule the driver enforces is a rule one
//! `psql` session can walk around, and every fold taken after that point would
//! disagree with every fold taken before it. That is the 0052 argument, and it
//! is why the copy exists rather than being removed.
//!
//! Two copies of a table are a defect the moment they differ, which is what this
//! test is for: it walks every ordered pair of the six state words and compares
//! the two answers cell by cell. The SQL side is read out of the migration and
//! evaluated here rather than against a live database, so the check runs in
//! `cargo test` with no services and fails at the moment somebody edits one copy.
//!
//! # What it does not check
//!
//! That the trigger *calls* the function, and that the opening state is `draft`.
//! Both are asserted below by reading the migration text, which is weaker than
//! executing it — a database test could execute it, and there is one to write.
//! What is checked here is the part that actually drifts: the table.

use std::collections::BTreeSet;

use research_session::fold::{transition_ok, SessionState};

const MIGRATION: &str = include_str!("../../../migrations/0138_research_sessions.sql");

/// The SQL table, read out of the migration.
///
/// Deliberately a parser for the exact shapes the function uses rather than a
/// general SQL evaluator: a general one would be a third implementation of the
/// same table, with its own bugs, and this one fails loudly when the shape
/// changes instead of quietly matching nothing.
struct SqlTable {
    /// `prev` → the set of `next` values it allows, before the guards.
    rows: Vec<(String, BTreeSet<String>)>,
    /// `WHEN next = prev THEN FALSE`
    refuses_self: bool,
    /// `WHEN prev = 'stopped' THEN FALSE`
    stopped_is_terminal: bool,
}

impl SqlTable {
    fn allows(&self, prev: &str, next: &str) -> bool {
        if self.refuses_self && prev == next {
            return false;
        }
        if self.stopped_is_terminal && prev == "stopped" {
            return false;
        }
        self.rows
            .iter()
            .find(|(p, _)| p == prev)
            .is_some_and(|(_, allowed)| allowed.contains(next))
    }
}

fn parse_sql_table() -> SqlTable {
    let body = MIGRATION
        .split_once("CREATE OR REPLACE FUNCTION session_transition_ok")
        .expect("the migration still defines session_transition_ok")
        .1;
    let body = body
        .split_once("$$;")
        .expect("the function body is delimited")
        .0;

    let mut rows = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("WHEN prev = '") else {
            continue;
        };
        let Some((prev, rest)) = rest.split_once('\'') else {
            continue;
        };
        let Some(list) = rest.split_once("next IN (").and_then(|(_, r)| {
            let end = r.find(')')?;
            Some(&r[..end])
        }) else {
            continue;
        };
        let allowed: BTreeSet<String> = list
            .split(',')
            .map(|s| s.trim().trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        rows.push((prev.to_string(), allowed));
    }

    SqlTable {
        rows,
        refuses_self: body.contains("WHEN next = prev      THEN FALSE"),
        stopped_is_terminal: body.contains("WHEN prev = 'stopped' THEN FALSE"),
    }
}

#[test]
fn the_parser_still_reads_the_migration_it_was_written_for() {
    let sql = parse_sql_table();
    assert!(
        sql.refuses_self,
        "the SQL table no longer refuses a state moving to itself, or the guard was reworded and \
         this parser silently stopped seeing it"
    );
    assert!(
        sql.stopped_is_terminal,
        "the SQL table no longer makes `stopped` terminal, or the guard was reworded"
    );
    // Five `WHEN prev = '…' THEN next IN (…)` rows: every state except
    // `stopped`, which the guard above handles.
    assert_eq!(
        sql.rows.len(),
        5,
        "the parser found {} rows in session_transition_ok, not the five it was written for — the \
         function's shape changed and this comparison is no longer comparing anything: {:?}",
        sql.rows.len(),
        sql.rows.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
}

#[test]
fn every_cell_of_the_two_tables_agrees() {
    let sql = parse_sql_table();
    let mut disagreements = Vec::new();

    for prev in SessionState::WORDS {
        for next in SessionState::WORDS {
            let in_rust = transition_ok(prev, next);
            let in_sql = sql.allows(prev, next);
            if in_rust != in_sql {
                disagreements.push(format!(
                    "{prev} -> {next}: Rust says {in_rust}, the migration says {in_sql}"
                ));
            }
        }
    }

    assert!(
        disagreements.is_empty(),
        "the fold and the trigger disagree about what a session may do. Whichever is right, a \
         session's history now depends on which one looked at it: {disagreements:#?}"
    );
}

#[test]
fn the_state_words_the_column_accepts_are_the_states_that_exist() {
    let check = MIGRATION
        .split_once("CONSTRAINT a_state_is_one_of_six CHECK (")
        .expect("the state CHECK is still there")
        .1;
    let list = check
        .split_once("state IN (")
        .expect("the CHECK still lists the words")
        .1;
    let list = &list[..list.find(')').expect("a closed list")];
    let in_sql: BTreeSet<String> = list
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .collect();
    let in_rust: BTreeSet<String> = SessionState::WORDS.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        in_sql, in_rust,
        "the column accepts a different set of state words than the enum has. A state the fold \
         can produce but the column refuses makes the session unwritable at the moment it \
         happens; the other way round makes it unreadable."
    );
}

#[test]
fn the_trigger_is_installed_on_the_log_and_uses_the_table() {
    assert!(
        MIGRATION.contains("CREATE TRIGGER trg_session_transition BEFORE INSERT ON session_events"),
        "the transition table exists but nothing runs it on INSERT, which makes it documentation"
    );
    assert!(
        MIGRATION.contains("session_transition_ok(prev_state, NEW.state)"),
        "the trigger no longer calls the table it is supposed to enforce"
    );
    assert!(
        MIGRATION.contains("NEW.state <> 'draft'"),
        "the opening state is no longer held to `draft`, so a log can begin mid-life"
    );
}

/// The item status words are the same two-copy problem, one table over.
#[test]
fn the_agenda_status_words_match_the_enum() {
    let check = MIGRATION
        .split_once("CONSTRAINT an_item_status_is_one_of_nine CHECK (")
        .expect("the status CHECK is still there")
        .1;
    let list = check
        .split_once("status IN (")
        .expect("the CHECK still lists the words")
        .1;
    let list = &list[..list.find(')').expect("a closed list")];
    let in_sql: BTreeSet<String> = list
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let in_rust: BTreeSet<String> = research_session::ItemStatus::WORDS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(in_sql, in_rust);
}

/// Layer one is a partial unique index, and "partial" is where it can be wrong.
///
/// The index enforces one live item per dedupe key. Which statuses count as
/// "live" is in its `WHERE`, written as a list of the terminal words — and if
/// that list gains a word the Rust side does not call terminal, an item that is
/// still live stops blocking duplicates and the layer quietly stops working. The
/// scan below is the only thing that would notice.
#[test]
fn the_index_that_enforces_layer_one_excludes_exactly_the_terminal_statuses() {
    let idx = MIGRATION
        .split_once("CREATE UNIQUE INDEX IF NOT EXISTS agenda_items_live_key")
        .expect("layer one's index is gone, so two live items may share a dedupe key")
        .1;
    let idx = &idx[..idx.find(';').unwrap_or(idx.len())];
    let list = idx
        .split_once("status NOT IN (")
        .expect("the index still names the statuses it excludes")
        .1;
    let list = &list[..list.find(')').expect("a closed list")];
    let in_sql: BTreeSet<String> = list
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let in_rust: BTreeSet<String> = research_session::ItemStatus::TERMINAL_WORDS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        in_sql, in_rust,
        "the index's idea of a finished item and `ItemStatus::is_terminal`'s disagree"
    );
}

/// And the autonomy words.
#[test]
fn the_autonomy_words_match_the_enum() {
    let check = MIGRATION
        .split_once("CONSTRAINT autonomy_is_one_of_three CHECK (")
        .expect("the autonomy CHECK is still there")
        .1;
    let list = check
        .split_once("autonomy IN (")
        .expect("the CHECK still lists the words")
        .1;
    let list = &list[..list.find(')').expect("a closed list")];
    let in_sql: BTreeSet<String> = list
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let in_rust: BTreeSet<String> = research_session::Autonomy::ALL
        .iter()
        .map(|a| a.as_str().to_string())
        .collect();
    assert_eq!(in_sql, in_rust);
}
