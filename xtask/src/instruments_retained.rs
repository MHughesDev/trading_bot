//! `cargo xtask check-instruments-retained` (Set P P-4.4; DATA-005 §8; DA-08).
//!
//! Delisted instruments stay in `instruments` with `status = 'delisted'`, and
//! `dataplane.instrument` rows are never removed. Migration 0155 enforces that in
//! the database — a row trigger refuses DELETE, a statement trigger refuses
//! TRUNCATE, and `app_role` no longer holds either privilege. This is the static
//! half: it fails the build when **source** acquires a path that deletes an
//! instrument row, or a migration that would switch the database half off.
//!
//! Why both halves: the trigger stops the statement at runtime, which is after a
//! deploy and after somebody's cleanup script has been written around it. The scan
//! stops it at review time, which is when the decision is still cheap.

use std::path::Path;
use std::process::Command;

/// The migrations that create the guards, and so may drop-and-recreate them to
/// stay re-runnable: 0155 for `instruments`, 0046 for `dataplane.instrument`.
const OWNING_MIGRATION: &str = "migrations/0155_instrument_delisting.sql";
const OWNING_MIGRATIONS: &[&str] = &[OWNING_MIGRATION, "migrations/0046_dataplane.sql"];

/// A finding: `file:line: text`.
pub type Finding = String;

/// Normalise a line for matching: lowercase, every run of whitespace one space,
/// and quote characters dropped, so `DELETE  FROM "instruments"` matches too.
fn normalise(line: &str) -> String {
    let lower = line.to_ascii_lowercase().replace(['"', '`'], "");
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `hay` contains `needle` as a whole identifier (the next character is
/// not part of an identifier), so `instruments_audit` and
/// `dataplane.instrument_symbol` do not match.
fn contains_ident(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let end = from + i + needle.len();
        let next = hay[end..].chars().next();
        if !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            return true;
        }
        from = end;
    }
    false
}

/// The removal statements this check refuses, on a normalised line.
#[must_use]
pub fn line_removes_an_instrument(line: &str) -> bool {
    let l = normalise(line);
    const TABLES: &[&str] = &["instruments", "public.instruments", "dataplane.instrument"];
    TABLES.iter().any(|t| {
        contains_ident(&l, &format!("delete from {t}"))
            || contains_ident(&l, &format!("truncate {t}"))
            || contains_ident(&l, &format!("truncate table {t}"))
            || contains_ident(&l, &format!("drop table {t}"))
            || contains_ident(&l, &format!("drop table if exists {t}"))
    })
}

/// A statement that would switch the database half of the rule off: dropping or
/// disabling 0155's triggers or function, or handing DELETE back on `instruments`.
#[must_use]
pub fn line_disables_the_guard(line: &str) -> bool {
    let l = normalise(line);
    let names_guard = l.contains("trg_instruments_no_delete")
        || l.contains("trg_instruments_no_truncate")
        || l.contains("trg_instrument_no_delete")
        || l.contains("refuse_instrument_removal");
    let disables =
        l.contains("drop trigger") || l.contains("drop function") || l.contains("disable trigger");
    let grants_delete =
        l.contains("grant") && l.contains("delete") && contains_ident(&l, "on instruments");
    (names_guard && disables) || grants_delete
}

/// Scan one file's text.
#[must_use]
pub fn scan(path: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let norm = path.replace('\\', "/");
    let is_owner = OWNING_MIGRATIONS.iter().any(|m| norm.ends_with(m));
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("--") || t.starts_with('#') {
            continue;
        }
        let removes = line_removes_an_instrument(line);
        // The owning migration drops and re-creates its own triggers so it can be
        // re-run; that is the one legitimate DROP TRIGGER on them.
        let disables = !is_owner && line_disables_the_guard(line);
        if removes || disables {
            out.push(format!("{path}:{}: {}", i + 1, line.trim()));
        }
    }
    out
}

/// Every tracked source file that could carry SQL, relative to `root`.
fn tracked_files(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--", "*.rs", "*.sql", "*.py", "*.ts", "*.tsx"])
        .output()
        .expect("git ls-files failed");
    String::from_utf8(output.stdout)
        .expect("invalid utf8 from git ls-files")
        .lines()
        .map(str::to_string)
        .collect()
}

/// Scan the whole tree, including untracked-but-present migrations, so a new
/// migration is checked before it is committed.
#[must_use]
pub fn scan_tree(root: &Path) -> Vec<Finding> {
    let mut files = tracked_files(root);
    if let Ok(rd) = std::fs::read_dir(root.join("migrations")) {
        for e in rd.flatten() {
            let p = format!("migrations/{}", e.file_name().to_string_lossy());
            if p.ends_with(".sql") && !files.contains(&p) {
                files.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for f in files {
        if f.contains("target/") || f.ends_with("xtask/src/instruments_retained.rs") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(root.join(&f)) {
            out.extend(scan(&f, &text));
        }
    }
    out
}

pub fn run() {
    let found = scan_tree(Path::new("."));
    if found.is_empty() {
        println!(
            "check-instruments-retained: OK — no code path deletes an instrument row, and nothing disables 0155's guard"
        );
        return;
    }
    eprintln!(
        "check-instruments-retained: FAILED — {} path(s) that remove an instrument:",
        found.len()
    );
    for f in &found {
        eprintln!("  {f}");
    }
    eprintln!();
    eprintln!("Delisted instruments are retained with status = 'delisted' (DATA-005 §8,");
    eprintln!("Set P P-4.4). Call delist_instrument(id, at, reason) instead; a deleted");
    eprintln!("instrument makes every historical universe survivorship-biased for good.");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delete_of_an_instrument_row_is_found_in_any_spelling() {
        for l in [
            "DELETE FROM instruments WHERE instrument_id = $1",
            "  delete   from   \"instruments\" where true",
            "sqlx::query(\"DELETE FROM public.instruments\")",
            "TRUNCATE instruments CASCADE",
            "TRUNCATE TABLE instruments",
            "DELETE FROM dataplane.instrument WHERE instrument_id = 1",
            "DROP TABLE IF EXISTS instruments",
        ] {
            assert!(line_removes_an_instrument(l), "{l}");
        }
    }

    #[test]
    fn neighbouring_tables_are_not_instruments() {
        for l in [
            "DELETE FROM instruments_audit WHERE x",
            "DELETE FROM dataplane.instrument_symbol WHERE x",
            "DELETE FROM synthetic_instruments WHERE x",
            "UPDATE instruments SET status = 'delisted'",
            "SELECT * FROM instruments",
        ] {
            assert!(!line_removes_an_instrument(l), "{l}");
        }
    }

    #[test]
    fn a_later_migration_that_drops_the_guard_is_found() {
        let f = scan(
            "migrations/0199_cleanup.sql",
            "DROP TRIGGER IF EXISTS trg_instruments_no_delete ON instruments;\n\
             GRANT DELETE ON instruments TO app_role;\n",
        );
        assert_eq!(f.len(), 2, "{f:?}");
        // …but the owning migration may re-create its own triggers.
        let own = scan(
            OWNING_MIGRATION,
            "DROP TRIGGER IF EXISTS trg_instruments_no_delete ON instruments;\n",
        );
        assert!(own.is_empty(), "{own:?}");
    }

    #[test]
    fn comments_are_not_code() {
        assert!(scan("x.rs", "// DELETE FROM instruments\n").is_empty());
        assert!(scan("x.sql", "-- DELETE FROM instruments\n").is_empty());
    }

    /// The live tree is clean. This is the test that fails the build when a
    /// deletion path is added anywhere in the workspace.
    #[test]
    fn the_workspace_has_no_instrument_deletion_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let found = scan_tree(root);
        assert!(found.is_empty(), "instrument deletion paths: {found:#?}");
        // And the guard it protects exists, so the clean result is not vacuous.
        assert!(root.join(OWNING_MIGRATION).exists());
    }
}
