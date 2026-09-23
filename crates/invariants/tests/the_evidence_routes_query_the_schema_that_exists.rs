//! Every SQL statement the evidence routes send names a table and columns the
//! migrated database actually has.
//!
//! # The defect this is written against
//!
//! `crates/api/src/routes/evidence.rs` shipped with
//!
//! ```sql
//! SELECT result FROM backtest_runs WHERE id = $1 AND user_id = $2
//! ```
//!
//! `migrations/0026_backtest_runs.sql` had already done
//! `ALTER TABLE IF EXISTS backtest_runs RENAME TO backtest_jobs` and created a
//! *different* `backtest_runs` — `run_id TEXT PRIMARY KEY`, `result_json`,
//! `created_by`. No `id`, no `user_id`, no `result`. So the query could not be
//! prepared, let alone run, and the handler's
//! `let Ok(Some(row)) = row else { … no_such_run … }` turned the error into a
//! 404. The route reported "that run does not exist" for every run that did.
//!
//! Nothing caught it. `crates/api/tests/the_desk_calls_routes_that_exist.rs`
//! checks that a **path** is served; a path served by a handler whose query
//! cannot execute passes it. The compiler cannot help either: `sqlx::query` with
//! a runtime string is a `&str`, and a `&str` is always well-typed.
//!
//! # The mechanism, and why `PREPARE` rather than a catalog diff
//!
//! Postgres resolves every relation, column and operator at Parse/Describe time
//! and fails on the first one that does not exist — **without executing the
//! statement or reading a row**. That is a complete check of the thing that
//! broke, for the price of one round trip per query, and it needs no list of
//! expected columns that somebody would have to keep in step.
//!
//! [`every_evidence_query_prepares_against_the_migrated_schema`] does exactly
//! that against a database built by running the whole migration chain.
//!
//! # Why there is a second, database-free test
//!
//! The live test follows the house convention for database tests — `#[ignore]`,
//! run by CI with `--ignored`. That is the right default (a developer with no
//! Postgres should not get a red suite), but it means the check that would have
//! caught this only runs where somebody wired it up. At the time of writing,
//! `.github/workflows/ci.yml` names its database tests one at a time and does
//! not name this one, so on its own the `PREPARE` test is a guard nothing calls.
//!
//! So [`every_table_and_column_the_evidence_routes_name_is_one_the_migrations_leave`]
//! runs always, with no services. It replays the migration chain's DDL as text —
//! `CREATE TABLE`, `RENAME TO`, `ADD`/`DROP`/`RENAME COLUMN`, `DROP TABLE`, in
//! offset order within each file — to work out which relations exist at the end
//! and what columns each has, then checks the routes' SQL against that.
//!
//! Checking the table name alone would **not** have caught this defect, and the
//! first version of this test did not: 0026 frees the name and immediately
//! creates a different table under it, so `backtest_runs` still resolves. It is
//! the columns that are gone. Hence the column check, and hence its shape — it
//! reads columns only from a `SELECT` over exactly one table whose select-list
//! is bare column names, and reports how many statements it therefore could not
//! judge. It may fail to catch a wrong column; it must never invent one.
//!
//! The two are deliberately redundant, and neither subsumes the other. The text
//! test runs everywhere and reads only the SQL it can parse; the `PREPARE` test
//! reads all of it and needs a database.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sqlx::{Connection, Executor, PgConnection, PgPool};

/// The files whose SQL is held to the schema.
///
/// Scoped rather than "all of `crates/api/src`" on purpose: a wider scan pulls
/// in `information_schema` and `pg_catalog` probes, `format!`-built SQL this
/// scanner cannot read, and statements that reference temporary relations, and
/// each of those needs an exemption. A list of exemptions is a list somebody
/// maintains by looking, which is the thing this test exists to replace.
///
/// Widen it by adding a file here and fixing whatever it reports.
const SCANNED: &[&str] = &[
    "crates/api/src/routes/evidence.rs",
    "crates/api/src/research/mod.rs",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .to_path_buf()
}

// ── Reading the SQL out of the Rust ──────────────────────────────────────────

/// One SQL statement found in a source file, with where it was found.
#[derive(Clone, Debug)]
struct FoundSql {
    file: String,
    line: usize,
    sql: String,
}

impl std::fmt::Display for FoundSql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let one_line = self.sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let shown = if one_line.len() > 160 {
            format!("{}…", &one_line[..160])
        } else {
            one_line
        };
        write!(f, "{}:{} — {shown}", self.file, self.line)
    }
}

/// The `sqlx` entry points that take a SQL string, as bare function names.
///
/// Matched against the **whole** identifier. Matching a prefix is how the first
/// version of this scanner read `sqlx::query_as(…)` as `sqlx::query` followed by
/// the stray token `_as`, decided that was not a call, and skipped the statement
/// without recording that it had — finding two statements where there were
/// three and passing on both.
const QUERY_FNS: &[&str] = &["query", "query_as", "query_scalar"];

/// Pull every string literal that is passed to a `sqlx::query*` call.
///
/// Deliberately a scanner and not a parser. It handles the two forms this
/// codebase writes — a plain `"…"` literal (which may span lines) and a raw
/// `r"…"` / `r#"…"#` literal — and **skips** a call whose argument is not a
/// literal, such as `&format!(…)`. Skipping is recorded, not silent: see
/// [`FoundSql`] counts in the assertions.
fn sql_literals(file: &str, src: &str) -> (Vec<FoundSql>, Vec<usize>) {
    let mut found = Vec::new();
    let mut skipped = Vec::new();

    let line_of = |idx: usize| src[..idx.min(src.len())].matches('\n').count() + 1;

    // Byte offsets of each `sqlx::query…(` call.
    let mut i = 0usize;
    while i < src.len() {
        let Some(rel) = src[i..].find("query") else {
            break;
        };
        let at = i + rel;
        i = at + 5;

        // The identifier `query` starts — take all of it, not a prefix of it.
        let end = at
            + src[at..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(src.len() - at);
        if !QUERY_FNS.contains(&&src[at..end]) {
            continue;
        }
        // Not this identifier if it started mid-word (`subquery`, `n_query`).
        let before = src[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }

        // Skip a turbofish, then find the opening paren.
        let mut j = end;
        let rest: Vec<char> = src[j..].chars().collect();
        let mut k = 0usize;
        if rest.first() == Some(&':') && rest.get(1) == Some(&':') && rest.get(2) == Some(&'<') {
            let mut depth = 0i32;
            for (n, c) in rest.iter().enumerate().skip(2) {
                match c {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            k = n + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        j += rest[..k].iter().map(|c| c.len_utf8()).sum::<usize>();

        // Whitespace, then `(`, then whitespace, then the literal.
        let tail = &src[j..];
        let Some(open) = tail.find('(') else { continue };
        if !tail[..open].trim().is_empty() {
            continue;
        }
        let mut p: Vec<char> = tail[open + 1..].chars().collect();
        let mut off = 0usize;
        while p.first().is_some_and(|c| c.is_whitespace()) {
            off += p[0].len_utf8();
            p.remove(0);
        }
        let start = j + open + 1 + off;

        match read_literal(src, start) {
            Some(sql) => found.push(FoundSql {
                file: file.to_string(),
                line: line_of(start),
                sql,
            }),
            None => skipped.push(line_of(start)),
        }
    }
    (found, skipped)
}

/// Read a Rust string literal beginning at byte offset `start`, or `None` when
/// what is there is not one.
fn read_literal(src: &str, start: usize) -> Option<String> {
    let s = &src[start..];
    if let Some(after_r) = s.strip_prefix('r') {
        let hashes = after_r.chars().take_while(|c| *c == '#').count();
        let after = &after_r[hashes..];
        let body = after.strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&close)?;
        return Some(body[..end].to_string());
    }
    let body = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                // A `\` at end of line continues a Rust literal, eating the
                // newline and the following indentation. Re-insert a space so
                // two SQL words do not fuse into one.
                '\n' => {
                    out.push(' ');
                    while let Some(&n) = chars.as_str().as_bytes().first() {
                        if n == b' ' || n == b'\t' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            other => out.push(other),
        }
    }
    None
}

/// Every SQL statement the scanned files hand to `sqlx`.
fn scanned_sql() -> (Vec<FoundSql>, Vec<(String, usize)>) {
    let root = repo_root();
    let mut all = Vec::new();
    let mut skipped = Vec::new();
    for rel in SCANNED {
        let path = root.join(rel);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let (found, skip) = sql_literals(rel, &src);
        // Per file, not in total. A scanner that reads one file and silently
        // reads nothing in the next is green for the wrong reason, and that is
        // not hypothetical: the first version of this one skipped every
        // `query_as` call in the repo, which is the whole of `research/mod.rs`.
        assert!(
            !found.is_empty() || !skip.is_empty(),
            "the scanner found no SQL at all in {rel}. Either that file stopped querying — in \
             which case take it out of SCANNED — or the scanner stopped reading it, in which \
             case this test is passing by finding nothing."
        );
        all.extend(found);
        skipped.extend(skip.into_iter().map(|l| ((*rel).to_string(), l)));
    }
    (all, skipped)
}

// ── The relation names the migration chain leaves behind ─────────────────────

/// Normalise a relation name to `schema.table`; unqualified means `public`.
fn qualify(name: &str) -> String {
    let n = name.trim().trim_matches('"').to_ascii_lowercase();
    if n.contains('.') {
        n
    } else {
        format!("public.{n}")
    }
}

/// What the migration chain says a relation is: its columns, or `None` when its
/// DDL is a shape this reader will not claim to understand (a view, a
/// `CREATE TABLE … AS SELECT`).
///
/// `None` is not "no columns". Every consumer treats it as "do not judge this
/// table's columns", because a reader that guessed would produce failures
/// nobody could act on, and a test nobody can act on gets deleted.
type Schema = std::collections::BTreeMap<String, Option<BTreeSet<String>>>;

/// Replay the migration chain's DDL, in filename order, and return the relations
/// that exist at the end with the columns each one has.
///
/// `CREATE TABLE`, `CREATE [MATERIALIZED] VIEW`, `ALTER TABLE … RENAME TO …`,
/// `ALTER TABLE … ADD/DROP/RENAME COLUMN` and `DROP TABLE` — the whole
/// vocabulary this chain uses.
fn live_schema() -> Schema {
    let dir = repo_root().join("migrations");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no migrations found in {}",
        dir.display()
    );

    let name = r"[a-z_][a-z0-9_]*(?:\s*\.\s*[a-z_][a-z0-9_]*)?";
    let create_table = regex::Regex::new(&format!(
        r"(?is)\bcreate\s+(?:unlogged\s+)?table\s+(?:if\s+not\s+exists\s+)?({name})"
    ))
    .unwrap();
    let create_view = regex::Regex::new(&format!(
        r"(?is)\bcreate\s+(?:or\s+replace\s+)?(?:materialized\s+)?view\s+(?:if\s+not\s+exists\s+)?({name})"
    ))
    .unwrap();
    let rename_table = regex::Regex::new(&format!(
        r"(?is)\balter\s+table\s+(?:if\s+exists\s+)?({name})\s+rename\s+to\s+([a-z_][a-z0-9_]*)"
    ))
    .unwrap();
    let add_col = regex::Regex::new(&format!(
        r"(?is)\balter\s+table\s+(?:if\s+exists\s+)?({name})\s+add\s+column\s+(?:if\s+not\s+exists\s+)?([a-z_][a-z0-9_]*)"
    ))
    .unwrap();
    let drop_col = regex::Regex::new(&format!(
        r"(?is)\balter\s+table\s+(?:if\s+exists\s+)?({name})\s+drop\s+column\s+(?:if\s+exists\s+)?([a-z_][a-z0-9_]*)"
    ))
    .unwrap();
    let rename_col = regex::Regex::new(&format!(
        r"(?is)\balter\s+table\s+(?:if\s+exists\s+)?({name})\s+rename\s+(?:column\s+)?([a-z_][a-z0-9_]*)\s+to\s+([a-z_][a-z0-9_]*)"
    ))
    .unwrap();
    let drop_rel = regex::Regex::new(&format!(
        r"(?is)\bdrop\s+(?:table|materialized\s+view|view)\s+(?:if\s+exists\s+)?({name})"
    ))
    .unwrap();

    let mut live: Schema = Schema::new();
    for f in &files {
        let sql = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        // Offset-ordered, because statement order matters *within* a file:
        // 0026 renames `backtest_runs` away and then creates a different table
        // under the freed name, in that order, eleven lines apart.
        let mut events: Vec<(usize, Event)> = Vec::new();
        for c in create_table.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            let cols = column_list(&sql, c.get(1).unwrap().end());
            events.push((
                m.start(),
                Event::Create(qualify(&c[1].replace(' ', "")), cols),
            ));
        }
        for c in create_view.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            events.push((
                m.start(),
                Event::Create(qualify(&c[1].replace(' ', "")), None),
            ));
        }
        for c in rename_table.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            let from = qualify(&c[1].replace(' ', ""));
            let schema = from.split('.').next().unwrap_or("public").to_string();
            let to = format!("{schema}.{}", c[2].to_ascii_lowercase());
            events.push((m.start(), Event::RenameTable(from, to)));
        }
        for c in add_col.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            events.push((
                m.start(),
                Event::AddCol(qualify(&c[1].replace(' ', "")), c[2].to_ascii_lowercase()),
            ));
        }
        for c in drop_col.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            events.push((
                m.start(),
                Event::DropCol(qualify(&c[1].replace(' ', "")), c[2].to_ascii_lowercase()),
            ));
        }
        for c in rename_col.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            events.push((
                m.start(),
                Event::RenameCol(
                    qualify(&c[1].replace(' ', "")),
                    c[2].to_ascii_lowercase(),
                    c[3].to_ascii_lowercase(),
                ),
            ));
        }
        for c in drop_rel.captures_iter(&sql) {
            let m = c.get(0).unwrap();
            events.push((m.start(), Event::Drop(qualify(&c[1].replace(' ', "")))));
        }
        events.sort_by_key(|e| e.0);
        for (_, e) in events {
            match e {
                Event::Create(t, cols) => {
                    live.insert(t, cols);
                }
                Event::RenameTable(from, to) => {
                    let cols = live.remove(&from).unwrap_or(None);
                    live.insert(to, cols);
                }
                // `ALTER TABLE … RENAME TO` also matches the `rename_col`
                // pattern's shape in some spellings, so an ALTER against a
                // table this reader has lost track of degrades that table to
                // "do not judge" rather than inventing a column set for it.
                Event::AddCol(t, c) => match live.get_mut(&t) {
                    Some(Some(cols)) => {
                        cols.insert(c);
                    }
                    Some(None) => {}
                    None => {
                        live.insert(t, None);
                    }
                },
                Event::DropCol(t, c) => {
                    if let Some(Some(cols)) = live.get_mut(&t) {
                        cols.remove(&c);
                    }
                }
                Event::RenameCol(t, from, to) => {
                    if let Some(Some(cols)) = live.get_mut(&t) {
                        if cols.remove(&from) {
                            cols.insert(to);
                        } else {
                            // The reader did not have the old name, so it does
                            // not know what this table's columns are now.
                            live.insert(t, None);
                        }
                    }
                }
                Event::Drop(t) => {
                    live.remove(&t);
                }
            }
        }
    }
    live
}

enum Event {
    Create(String, Option<BTreeSet<String>>),
    RenameTable(String, String),
    AddCol(String, String),
    DropCol(String, String),
    RenameCol(String, String, String),
    Drop(String),
}

/// The column names declared by the parenthesised list that follows a
/// `CREATE TABLE <name>`, or `None` when there is no such list (`… AS SELECT`,
/// `… PARTITION OF`) or it cannot be read.
fn column_list(sql: &str, after_name: usize) -> Option<BTreeSet<String>> {
    let rest = &sql[after_name..];
    let open_rel = rest.find(|c: char| !c.is_whitespace())?;
    if rest.as_bytes()[open_rel] != b'(' {
        return None;
    }
    let body_start = after_name + open_rel + 1;
    let mut depth = 1i32;
    let mut end = None;
    for (i, ch) in sql[body_start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(body_start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &sql[body_start..end?];

    // Split on commas at depth 0; the first word of each item is a column name
    // unless the item is a table constraint.
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                items.push(&body[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    items.push(&body[start..]);

    const NOT_A_COLUMN: &[&str] = &[
        "constraint",
        "primary",
        "unique",
        "foreign",
        "check",
        "exclude",
        "like",
        "inherits",
    ];
    let mut cols = BTreeSet::new();
    for item in items {
        // Strip line comments, which sit inside these bodies throughout.
        let cleaned: String = item
            .lines()
            .map(|l| l.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        let Some(word) = cleaned.split_whitespace().next() else {
            continue;
        };
        let word = word.trim_matches('"').to_ascii_lowercase();
        if word.is_empty() || NOT_A_COLUMN.contains(&word.as_str()) {
            continue;
        }
        if !word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return None;
        }
        cols.insert(word);
    }
    (!cols.is_empty()).then_some(cols)
}

/// Relation names read by a statement, minus its own CTEs.
fn tables_referenced(sql: &str) -> BTreeSet<String> {
    let cte = regex::Regex::new(r"(?is)(?:\bwith\b|,)\s+([a-z_][a-z0-9_]*)\s+as\s*\(").unwrap();
    let ctes: BTreeSet<String> = cte
        .captures_iter(sql)
        .map(|c| c[1].to_ascii_lowercase())
        .collect();

    let from = regex::Regex::new(
        r"(?is)\b(?:from|join|into|update)\s+(?:only\s+)?([a-z_][a-z0-9_]*(?:\.[a-z_][a-z0-9_]*)?)",
    )
    .unwrap();
    from.captures_iter(sql)
        .map(|c| c[1].to_ascii_lowercase())
        .filter(|n| !ctes.contains(n))
        // System catalogs are not in the migration chain and never will be.
        .filter(|n| !n.starts_with("pg_") && !n.starts_with("information_schema"))
        .map(|n| qualify(&n))
        .collect()
}

/// The columns a statement plainly names, or `None` when its shape is not one
/// this reader will claim to understand.
///
/// Only a `SELECT` whose select-list is a comma-separated list of bare column
/// names, plus the left-hand identifiers of comparisons in its `WHERE`. Anything
/// else — a function call, an alias, an expression, an `INSERT` — returns `None`
/// and is left to `PREPARE`. The rule is conservative on purpose: this test may
/// fail to catch a wrong column, and must never invent one.
fn columns_referenced(sql: &str) -> Option<BTreeSet<String>> {
    let select = regex::Regex::new(r"(?is)^\s*select\s+(.*?)\s+from\s").unwrap();
    let caps = select.captures(sql)?;
    let list = caps.get(1)?.as_str();
    if list.contains('(') || list.trim().eq_ignore_ascii_case("*") {
        return None;
    }

    let mut cols = BTreeSet::new();
    for item in list.split(',') {
        let item = item.trim();
        if item.is_empty()
            || !item.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || item.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return None;
        }
        cols.insert(item.to_ascii_lowercase());
    }

    let cmp = regex::Regex::new(
        r"(?is)\b(?:where|and|or)\s+([a-z_][a-z0-9_]*)\s*(?:=|<>|!=|>=|<=|>|<|\bis\b|\blike\b)",
    )
    .unwrap();
    for c in cmp.captures_iter(sql) {
        cols.insert(c[1].to_ascii_lowercase());
    }
    Some(cols)
}

// ── The two tests ────────────────────────────────────────────────────────────

/// No services. Runs in every `cargo test`.
#[test]
fn every_table_and_column_the_evidence_routes_name_is_one_the_migrations_leave() {
    let live = live_schema();

    // Anchors. A reader that silently stopped working would know about no
    // tables at all and pass everything; one that guessed would know about
    // tables the chain never creates. Both are checked before its verdict on
    // anything else is used — including the exact pair `0026` produced, since
    // the whole defect is that one of these names survived with a different
    // shape behind it.
    let cols = |t: &str| live.get(t).and_then(Clone::clone);
    let jobs = cols("public.backtest_jobs").expect("0011+0012 declare backtest_jobs' columns");
    let runs = cols("public.backtest_runs").expect("0026 declares backtest_runs' columns");
    assert!(
        jobs.contains("id") && jobs.contains("user_id") && jobs.contains("result"),
        "backtest_jobs lost the columns 0011/0012 give it: {jobs:?}"
    );
    assert!(
        runs.contains("run_id") && runs.contains("result_json") && !runs.contains("user_id"),
        "backtest_runs is not the Set-J table 0026 creates: {runs:?}"
    );
    assert!(
        live.contains_key("mlops.trial") && !live.contains_key("public.no_such_table"),
        "the migration reader is not reading schema-qualified names, or is inventing relations"
    );

    let (sql, skipped) = scanned_sql();
    let mut wrong: Vec<String> = Vec::new();
    let mut unjudged = 0usize;
    for found in &sql {
        let tables = tables_referenced(&found.sql);
        for t in &tables {
            if !live.contains_key(t) {
                wrong.push(format!(
                    "{found}\n      names `{t}`, which no migration leaves in existence"
                ));
            }
        }
        // Columns only when there is exactly one table to attribute them to and
        // the chain says what that table's columns are. Otherwise this says
        // nothing, and says that it said nothing.
        let one = match tables.iter().next() {
            Some(t) if tables.len() == 1 => t.clone(),
            _ => {
                unjudged += 1;
                continue;
            }
        };
        let (Some(Some(have)), Some(want)) = (live.get(&one), columns_referenced(&found.sql))
        else {
            unjudged += 1;
            continue;
        };
        for c in want.difference(have) {
            wrong.push(format!(
                "{found}\n      names column `{c}`, which `{one}` does not have. It has: {}",
                have.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "SQL that does not match the schema the migrations build:\n  - {}\n\n\
         `migrations/0026_backtest_runs.sql` renames `backtest_runs` to `backtest_jobs` and then \
         creates a *different* table under the freed name. A query left pointing at the old \
         shape still names a table that exists, so it fails on a column — and a handler that \
         reads a query error as \"not found\" then reports a clean 404 for every run forever.",
        wrong.join("\n  - ")
    );

    // Said out loud rather than left to be discovered. These are the statements
    // this database-free test could not judge; they are covered only by
    // `every_evidence_query_prepares_against_the_migrated_schema`, which needs
    // Postgres and is `#[ignore]`d.
    eprintln!(
        "checked {} statement(s); {unjudged} not judged for columns here; \
         {} built at runtime and not checked at all: {skipped:?}",
        sql.len(),
        skipped.len()
    );
}

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// A database with the whole migration chain applied — the deployed schema.
async fn migrated() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS evidence_sql_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE evidence_sql_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "evidence_sql_test"))
        .await
        .unwrap();
    storage::postgres::run_migrations(&pool)
        .await
        .expect("every migration applies to an empty database");
    pool
}

/// Live Postgres; ignored by default:
///
/// ```bash
/// cargo test -p invariants --test the_evidence_routes_query_the_schema_that_exists \
///     -- --ignored --test-threads=1
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn every_evidence_query_prepares_against_the_migrated_schema() {
    let pool = migrated().await;
    let mut conn = pool.acquire().await.expect("a connection");

    // Prove the mechanism can fail before trusting that it passed. A test that
    // prepares nothing, or whose prepare always succeeds, is green for the same
    // reason a broken query was.
    let control = conn
        .prepare("SELECT result FROM backtest_runs WHERE id = $1 AND user_id = $2")
        .await;
    assert!(
        control.is_err(),
        "the exact query that shipped broken prepared cleanly against the migrated schema. \
         Either the schema changed and this test's premise is stale, or PREPARE is not \
         resolving names — either way its passes mean nothing."
    );

    let (sql, skipped) = scanned_sql();
    let mut failures: Vec<String> = Vec::new();
    for found in &sql {
        if let Err(e) = conn.prepare(&found.sql).await {
            failures.push(format!("{found}\n      {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "SQL that Postgres cannot prepare against the migrated schema — every one of these \
         resolves to a runtime error the handler must not mistake for an empty result:\n  - {}",
        failures.join("\n  - ")
    );
    assert!(skipped.is_empty() || !sql.is_empty(), "nothing was checked");
    eprintln!(
        "prepared {} statement(s) from {} file(s); {} runtime-built statement(s) not checked",
        sql.len(),
        SCANNED.len(),
        skipped.len()
    );
}
