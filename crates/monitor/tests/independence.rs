//! COMP-007 §1 P1 — the monitor is a separate failure domain, structurally.
//!
//! Every other test in this crate could pass while the monitor quietly became a
//! module of the API, at which point it would die with the thing it is watching and
//! all of those tests would still be green. So the separation is asserted over the
//! manifest and the source, not over behaviour.

use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn manifest() -> String {
    fs::read_to_string(crate_root().join("Cargo.toml")).expect("crates/monitor/Cargo.toml")
}

fn sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    let src = crate_root().join("src");
    let mut paths = Vec::new();
    walk(&src, &mut paths);
    paths
        .into_iter()
        .filter_map(|p| {
            let name = p
                .strip_prefix(&src)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some((name, fs::read_to_string(&p).ok()?))
        })
        .collect()
}

#[test]
fn the_monitor_does_not_depend_on_the_api() {
    // The direction of the dependency *is* the design. The API may read the
    // monitor's tables; the monitor must be able to run, and keep recording, with
    // the API stopped.
    let toml = manifest();
    for forbidden in ["api = ", "api-lib", "ui-gateway", "axum"] {
        assert!(
            !toml.contains(forbidden),
            "crates/monitor must not depend on {forbidden:?}: a monitor that needs the \
             web layer to start is a monitor that stops when the web layer does"
        );
    }
    for (name, src) in sources() {
        assert!(
            !src.contains("use api::") && !src.contains("api::routes"),
            "{name} reaches into the API process"
        );
    }
}

#[test]
fn the_monitor_reads_one_database_and_it_is_postgres() {
    // The other half of the failure domain, and until 2026-09-21 it was a
    // sentence in a doc comment rather than a check. `FactKind::HardDataStaleness`
    // is unwired because the age of an instrument's last bar lives in ClickHouse
    // and this crate has no client for it; the obvious way to "fix" that blocker
    // is to add one, and the obvious way is the wrong one.
    //
    // A monitor holding a ClickHouse handle has to answer a question it cannot
    // answer well: what does a stale feed look like when the store you would
    // measure its staleness *from* is the thing that is down? Either it reports
    // every feed absent and halts the whole book on its own outage, or it reports
    // nothing and goes quiet in exactly the failure it exists to catch. Neither is
    // a safety property. The answer is a producer writing a Postgres gauge — see
    // `FactKind::HardDataStaleness`'s `needs` — so the monitor keeps measuring
    // from the one database whose loss already closes the order gate by itself.
    //
    // Asserted over the manifest, because this is a dependency decision and a
    // behavioural test cannot see it: a `clickhouse` line added here would break
    // nothing and pass everything.
    let toml = manifest();
    for forbidden in [
        // The client itself.
        "clickhouse",
        "reqwest",
        // And the two crates that own one, which would hand this crate a bar
        // reader by the back door while the manifest still read as Postgres-only.
        "storage = ",
        "backtest = ",
    ] {
        assert!(
            !toml.contains(forbidden),
            "crates/monitor must not depend on {forbidden:?}: a monitor that measures \
             staleness from the store it is measuring cannot distinguish a dead feed \
             from its own outage, and one of those two mistakes halts the book while \
             the other goes quiet. The bar age reaches this crate as a Postgres gauge \
             written by whoever observed the bar — see FactKind::HardDataStaleness"
        );
    }
    // Anti-vacuity: the manifest must still declare the one database it does read,
    // or the loop above is asserting the absence of things from an empty file.
    assert!(
        toml.contains("sqlx = "),
        "crates/monitor must still depend on sqlx; this guard is broken, not satisfied"
    );
}

#[test]
fn the_monitor_ships_its_own_binary() {
    // COMP-007 §1 P1: "its own binary (`crates/monitor`, service `tbot-monitor`)".
    // A library alone would satisfy every other test here and would only ever run
    // inside somebody else's process.
    let toml = manifest();
    assert!(
        toml.contains("[[bin]]"),
        "crates/monitor must build a binary"
    );
    assert!(
        toml.contains("name = \"tbot-monitor\""),
        "the service is named tbot-monitor in the spec, the compose file and the drill"
    );
    assert!(
        crate_root().join("src/bin/tbot-monitor.rs").exists(),
        "the binary's entry point must exist"
    );
}

#[test]
fn the_monitor_binary_opens_its_own_database_connection() {
    // Not "is given one by the platform". Its own pool, from its own environment,
    // so that nothing about the API's lifecycle can take it away.
    let main = fs::read_to_string(crate_root().join("src/bin/tbot-monitor.rs")).expect("main");
    assert!(
        main.contains("PgPoolOptions"),
        "the monitor builds its own pool"
    );
    assert!(main.contains("DATABASE_URL"), "from its own configuration");
}

#[test]
fn nothing_in_this_crate_writes_an_agent_actor() {
    // COMP-007 §5: agents can never trip or reset. Every place that could author a
    // record refuses an `agent:` prefix, and this asserts the refusals have not
    // been quietly removed — a `starts_with("agent:")` that got deleted would leave
    // every other test in this crate passing.
    let guards: usize = sources()
        .iter()
        .map(|(_, src)| src.matches("starts_with(\"agent:\")").count())
        .sum();
    assert!(
        guards >= 6,
        "expected every authoring path to refuse an agent actor; found {guards} guards"
    );
}
