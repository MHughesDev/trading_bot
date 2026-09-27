//! Every `pg_notify` the monitor's migrations publish has something listening
//! (COMP-007 §5, §6; S-7.10, and the last of Phase 7's exit criteria).
//!
//! # The defect this file exists to refuse
//!
//! Migration `0074_monitor_lease.sql` installs a trigger that publishes on
//! `monitor_lease`, and its comment states the consequence as fact:
//!
//! > The mirror is refreshed on every lease renewal and on this NOTIFY, so a trip
//! > propagates without waiting a cadence.
//!
//! `0075_kill_switch.sql` does the same for `monitor_kill_switch`, which is the
//! channel that matters — it fires on every insert or update of
//! `monitor.kill_switch`, which is every halt and every reset.
//!
//! On 2026-09-23 **no code in the workspace had ever issued a `LISTEN`.** Both
//! triggers were firing into an empty room, `LeaseWatcher::run` was a plain
//! `sleep(cadence)` loop, and a halt reached the order gate when the timer next
//! came round. The exit criterion — "the kill-switch state is visible on every
//! route in the desk within one event of a trip" — was resting on a sentence in a
//! SQL comment.
//!
//! This is the repository's dominant defect shape, in the one place where the
//! producer is the database rather than a Rust caller: a complete, working,
//! tested mechanism with nothing on the other end. A comment cannot notice that
//! about itself. This scan can, and it fails when a future migration adds a
//! channel nobody subscribes to.

use std::fs;
use std::path::{Path, PathBuf};

use monitor::store::NOTIFY_CHANNELS;

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn repo_root() -> PathBuf {
    crate_root()
        .parent()
        .and_then(Path::parent)
        .expect("crates/monitor sits two levels below the repository root")
        .to_path_buf()
}

/// A source file with its test module removed.
///
/// The same reason as `facts_are_wired.rs`: a scan that counted the crate's own
/// tests would report the wiring that is missing as present, which is the
/// mistake being guarded against.
fn production_source(relative: &str) -> String {
    let src = fs::read_to_string(crate_root().join(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"));
    match src.find("#[cfg(test)]") {
        Some(i) => src[..i].to_owned(),
        None => src,
    }
}

/// The body of one function inside one `impl` block.
///
/// The `impl` anchor is not decoration. `src/service.rs` declares **two** `pub
/// async fn run(self)` — `MonitorService::run`, the monitor process's own loop,
/// and `LeaseWatcher::run`, which runs in the API process and is the only thing
/// that refreshes the order gate's mirror. A scan that took the first match
/// would assert about the monitor's loop and report the watcher as wired, which
/// is the wrong half of the process boundary and the exact confusion this phase
/// has already been burned by twice.
fn function_body(source: &str, impl_anchor: &str, signature: &str) -> String {
    let block = source
        .find(impl_anchor)
        .unwrap_or_else(|| panic!("{impl_anchor:?} is not in this source any more"));
    let source = &source[block..];
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature:?} is not in {impl_anchor:?} any more"));
    let rest = &source[start + signature.len()..];
    let end = rest.find("\n    pub ").unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// `LeaseWatcher::run`, the loop in the *watched* process.
fn watcher_run_body() -> String {
    let service = production_source("src/service.rs");
    let body = function_body(&service, "impl LeaseWatcher {", "pub async fn run(self) {");
    assert!(
        body.len() > 200,
        "LeaseWatcher::run did not parse; the guard is broken, not satisfied"
    );
    assert!(
        body.contains("self.refresh()"),
        "the body the guard found does not call refresh(), so it is not the watcher loop \
         that feeds the order gate; the guard is broken, not satisfied"
    );
    body
}

/// Every channel any migration publishes on, read out of the SQL.
///
/// Read rather than listed, because the whole point is to catch a channel
/// somebody adds later. A second hand-written list here would go stale in
/// exactly the way the one in `0074`'s comment did.
fn channels_published_by_migrations() -> Vec<(String, String)> {
    let dir = repo_root().join("migrations");
    let mut found = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    entries.sort();
    for path in entries {
        let sql = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let file = path
            .file_name()
            .expect("a read migration has a name")
            .to_string_lossy()
            .into_owned();
        for rest in sql.split("pg_notify(").skip(1) {
            // `pg_notify('channel', payload)` — the literal between the first
            // pair of single quotes. A computed channel name is not something
            // this guard can follow, and it fails below rather than passing.
            let Some(after) = rest.strip_prefix('\'') else {
                panic!(
                    "{file} calls pg_notify with a channel this guard cannot read \
                     statically; a channel nobody can enumerate is a channel nobody \
                     can prove has a listener"
                );
            };
            let channel: String = after.chars().take_while(|c| *c != '\'').collect();
            found.push((file.clone(), channel));
        }
    }
    found
}

#[test]
fn the_scan_still_finds_the_two_channels_it_was_written_for() {
    // The anti-vacuity assertion, first. Every claim below is about channels the
    // scan found; a scan that found none would make all of them pass while the
    // notifications went nowhere — which is precisely the original defect, with a
    // green test on top of it.
    let published = channels_published_by_migrations();
    for expected in ["monitor_lease", "monitor_kill_switch"] {
        assert!(
            published.iter().any(|(_, c)| c == expected),
            "no migration publishes on {expected:?} any more; this guard is broken, \
             not satisfied (found {published:?})"
        );
    }
}

#[test]
fn a_channel_a_migration_publishes_on_is_never_left_without_a_subscriber() {
    for (file, channel) in channels_published_by_migrations() {
        // Only the monitor's own channels. Another schema's notifications are
        // another crate's to subscribe to, and claiming them here would be this
        // crate asserting about wiring it cannot see.
        if !channel.starts_with("monitor_") {
            continue;
        }
        assert!(
            NOTIFY_CHANNELS.contains(&channel.as_str()),
            "{file} publishes on {channel:?} and monitor::store::NOTIFY_CHANNELS does \
             not list it, so nothing LISTENs on it. Every write that trigger fires for \
             reaches the order gate's mirror no sooner than the next cadence — which is \
             the state `monitor_lease` and `monitor_kill_switch` were both in until \
             2026-09-23, with a comment in {file} asserting the opposite."
        );
    }
}

#[test]
fn the_watcher_that_feeds_the_order_gate_does_more_than_sleep() {
    let run = watcher_run_body();
    assert!(
        run.contains("listener"),
        "LeaseWatcher::run never obtains a listener, so the pg_notify triggers in \
         migrations 0074 and 0075 publish to nobody and a trip reaches the order gate \
         only when the cadence timer comes round"
    );
    // And the cadence must remain the floor. A refresh driven *only* by
    // notifications is the dangerous shape: a listener whose connection died
    // silently stops refreshing, and the mirror goes on answering the order gate
    // with the last thing it happened to see.
    assert!(
        run.contains("self.cadence"),
        "LeaseWatcher::run no longer bounds its wait by the cadence. A notification-only \
         refresh stops refreshing the moment the listener breaks, and an order gate that \
         has quietly stopped asking is the one direction this crate must not fail in."
    );
}

#[test]
fn a_listener_that_cannot_be_built_does_not_stop_the_refresh() {
    let run = watcher_run_body();
    // `store.listener()` returns a `Result`, and the only acceptable handling of
    // the error is to carry on without it. An `unwrap`, an `expect` or a `?` here
    // would mean a process whose database was briefly unreachable at startup ran
    // on with **no mirror refresh at all** — the lease would look valid for ever
    // from the gate's point of view, because nothing would ever observe it
    // lapsing.
    for forbidden in [".listener().await.unwrap()", ".listener().await.expect("] {
        assert!(
            !run.contains(forbidden),
            "LeaseWatcher::run uses {forbidden} — a failure to LISTEN must degrade to the \
             cadence poll, never take down the only thing that refreshes the order gate"
        );
    }
    assert!(
        run.contains("None"),
        "LeaseWatcher::run has no path for having no listener, so it cannot be degrading \
         to the cadence poll when one cannot be built"
    );
}
