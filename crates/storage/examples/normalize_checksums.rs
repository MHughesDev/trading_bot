//! Re-record migration checksums that differ **only** by line endings.
//!
//! ## The problem this exists for
//!
//! sqlx stores a SHA-384 of each migration's bytes when it applies it, and
//! refuses to run against a database whose recorded checksum disagrees with the
//! file on disk. That is the right behaviour: a migration edited after it ran is
//! a schema nobody can reconstruct.
//!
//! But with `core.autocrlf=true` and no `.gitattributes` rule, a Windows
//! checkout of a migration produces CRLF and a Linux one produces LF, so the
//! *same commit* yields two different checksums. A database migrated on one
//! machine then cannot be migrated on the other, and the error — "migration 36
//! was previously applied but has been modified" — describes a file that git
//! reports as unmodified. That happened on 2026-09-14 and cost an hour.
//!
//! `.gitattributes` now pins migrations to LF, which stops it recurring. This
//! repairs the databases that were migrated before that.
//!
//! ## What it will and will not do
//!
//! For each mismatched row it recomputes the checksum of the file with CRLF
//! normalised to LF, and with LF expanded to CRLF. **If either matches the
//! recorded value, the only difference is line endings** and the row is
//! re-recorded to the file's current bytes. If neither matches, the file's
//! content genuinely changed, and the row is left alone and reported — because
//! at that point the stored checksum is doing exactly the job it exists for, and
//! overwriting it would erase the only evidence that a migration was edited
//! after it ran.
//!
//! It never applies a migration and never touches schema. It is a repair tool
//! for one specific, provable, cosmetic discrepancy.
//!
//! ```text
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo run -p storage --example normalize_checksums
//! ```
//!
//! Pass `--dry-run` to see what it would change without changing it.

use sha2::{Digest, Sha384};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dry_run = std::env::args().any(|a| a == "--dry-run");

    let url = std::env::var("DATABASE_URL").map_err(|_| {
        anyhow::anyhow!(
            "DATABASE_URL is not set. There is no default here on purpose: a tool that rewrites \
             migration checksums and guesses its target is one that eventually guesses production."
        )
    })?;

    let files = read_migrations()?;
    if files.is_empty() {
        anyhow::bail!("no migrations found — run this from the workspace root");
    }

    let pool = PgPool::connect(&url).await?;
    let rows =
        sqlx::query("SELECT version, checksum FROM public._sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await?;

    let (mut ok, mut fixed, mut refused, mut missing) = (0usize, 0usize, 0usize, 0usize);

    for row in rows {
        let version: i64 = row.try_get("version")?;
        let stored: Vec<u8> = row.try_get("checksum")?;

        let Some(bytes) = files.get(&version) else {
            println!("  ?  {version}: applied, but no file with that version is on disk");
            missing += 1;
            continue;
        };

        let current = sha384(bytes);
        if current == stored {
            ok += 1;
            continue;
        }

        let as_lf = sha384(&to_lf(bytes));
        let as_crlf = sha384(&to_crlf(bytes));

        if stored != as_lf && stored != as_crlf {
            println!(
                "  !  {version}: the content differs by more than line endings. Left alone — the \
                 stored checksum is doing its job, and overwriting it would erase the only \
                 evidence that this migration was edited after it ran."
            );
            refused += 1;
            continue;
        }

        let was = if stored == as_lf { "LF" } else { "CRLF" };
        if dry_run {
            println!(
                "  ~  {version}: recorded under {was}, file is now the other. Would re-record."
            );
        } else {
            sqlx::query("UPDATE public._sqlx_migrations SET checksum = $1 WHERE version = $2")
                .bind(&current)
                .bind(version)
                .execute(&pool)
                .await?;
            println!("  ✓  {version}: recorded under {was}; re-recorded to the file's bytes.");
        }
        fixed += 1;
    }

    pool.close().await;

    println!(
        "\n{ok} already matched, {fixed} {} line-ending mismatches, {refused} refused, \
         {missing} applied with no file.",
        if dry_run { "would fix" } else { "fixed" }
    );
    if refused > 0 {
        println!(
            "\nThe refused rows are a real discrepancy, not a cosmetic one. Either the migration \
             was edited after it ran — in which case the database and the file describe different \
             schemas and somebody has to decide which is right — or it was replaced wholesale, in \
             which case it needed a new version number."
        );
        std::process::exit(1);
    }
    Ok(())
}

fn sha384(bytes: &[u8]) -> Vec<u8> {
    Sha384::digest(bytes).to_vec()
}

fn to_lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn to_crlf(bytes: &[u8]) -> Vec<u8> {
    let lf = to_lf(bytes);
    let mut out = Vec::with_capacity(lf.len() * 2);
    for b in lf {
        if b == b'\n' {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

/// Migration files keyed by the numeric prefix sqlx uses as `version`.
fn read_migrations() -> anyhow::Result<BTreeMap<i64, Vec<u8>>> {
    let dir = std::path::Path::new("migrations");
    if !dir.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(version) = digits.parse::<i64>() {
            out.insert(version, std::fs::read(&path)?);
        }
    }
    Ok(out)
}
