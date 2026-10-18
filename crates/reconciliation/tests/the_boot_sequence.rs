//! The wire in T-0.1's title, exercised end to end, and the ratchet that keeps
//! it from rotting back out.
//!
//! T-0.1 was marked `DONE` while `ForwardRecorder` was constructed only in
//! `crates/reconciliation/tests/`, `execution::fills::register_observer` had
//! zero production call sites, and `ExecutionEngine::with_fill_observer` — the
//! method three plan files instructed tracks to call — did not exist anywhere
//! in the workspace. Nothing failed, because nothing checked.
//!
//! So this file checks three separate things:
//!
//! 1. The boot sequence written in `recorder.rs`'s module docs is real: it
//!    compiles, it runs, and a fill taken after it lands in a deployment's
//!    forward record.
//! 2. A recorder that did not come from the registry receives nothing, and
//!    there is no way to attach one that did not — `ForwardRecorder` does not
//!    implement `FillObserver`, so the half-wire that shipped last time is now
//!    a compile error rather than a silent no-op.
//! 3. The set of production binaries that can place orders and do not install
//!    forward recording is still exactly the set we know about — empty, since
//!    `apps/platform/src/main.rs` was wired on 2026-09-27 — and some production
//!    binary does both. Both halves are asserted, because an empty list alone is
//!    also what a workspace with no binaries at all looks like.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::money::{Price, Size};
use domain::numeraire::Numeraire;
use domain::order::{OrderIntent, OrderType, Side};
use execution::broker::{Broker, BrokerOrderState};
use execution::mock_broker::MockBroker;
use execution::ExecutionEngine;
use reconciliation::forward::{DeploymentSpec, DivergenceThresholds, PaperVenue};
use reconciliation::{ForwardRecorder, ForwardRecorders, ForwardRecordingVerdict};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate, StrategyHaltRegistry};
use rust_decimal::Decimal;
use uuid::Uuid;

const STRATEGY: &str = "boot-sequence-1";
const BAR: i64 = 60;

fn ensure_journal() {
    execution::journal::install(Arc::new(
        execution::journal::memory::MemoryOrderJournal::new(),
    ));
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn px(s: &str) -> Price {
    Price::from_str(s).unwrap()
}

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

fn spec(strategy_id: &str) -> DeploymentSpec {
    DeploymentSpec {
        deployment_id: Uuid::new_v4(),
        strategy_id: strategy_id.to_owned(),
        trial_id: Uuid::new_v4(),
        venue: PaperVenue::Platform,
        numeraire: Numeraire::Usd,
        bar_seconds: BAR,
        thresholds: DivergenceThresholds {
            max_abs_return_divergence: 1.0,
            max_cum_return_divergence: 1.0,
            max_position_divergence: dec("0.000001"),
            min_signal_match: 0.5,
        },
    }
}

fn buy_intent(strategy_id: &str) -> OrderIntent {
    OrderIntent::new(
        "AAPL",
        Side::Buy,
        OrderType::Market,
        Size::from_str("10").unwrap(),
        None,
        Some(strategy_id.to_owned()),
    )
}

fn filling_engine() -> ExecutionEngine {
    let mut mock = MockBroker::new();
    mock.filled_qty = dec("10");
    mock.fill_price = Some(px("101"));
    mock.query_state = BrokerOrderState::Filled;
    ExecutionEngine::new(Arc::new(mock) as Arc<dyn Broker>)
}

async fn submit(engine: &ExecutionEngine, intent: OrderIntent) {
    let gate = RiskGate::new(
        GlobalRiskLimits::default(),
        Arc::new(KillSwitch::new(false)),
    );
    let ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        Some(px("100")),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        true,
        0,
        0,
    );
    engine
        .submit(gate.check(intent, &ctx).expect("approved"))
        .await
        .expect("submitted");
}

// ── 1. The documented boot sequence ──────────────────────────────────────────

/// The literal contents of `recorder.rs`'s module doc, run.
///
/// If `install` is renamed, if `deploy` changes shape, or if the engine stops
/// publishing, this stops compiling or stops passing. That is the check nobody
/// had on `with_fill_observer`.
#[tokio::test]
async fn the_documented_boot_sequence_puts_a_fill_in_the_forward_record() {
    ensure_journal();

    // ── Startup, before the first order. ──
    let recorders = ForwardRecorders::install();
    execution::fills::require_recording()
        .expect("installing the registry is what makes the fill stream recorded");

    // ── A deployment, later. ──
    let halts = Arc::new(StrategyHaltRegistry::new());
    let recorder = recorders
        .deploy(spec(STRATEGY), dec("10000"), Arc::clone(&halts))
        .expect("a valid paper deployment");
    recorder.open(at(BAR), px("100")).unwrap();

    // ── An order. ──
    let engine = filling_engine();
    let intent = buy_intent(STRATEGY);
    recorder.on_submit(&intent, px("100"));
    assert_eq!(recorder.position(), Decimal::ZERO, "nothing has filled yet");

    submit(&engine, intent).await;

    assert_eq!(
        recorder.position(),
        dec("10"),
        "a fill taken after the boot sequence must reach the deployment's \
         forward record — this assertion is the whole of T-0.1"
    );

    let ev = recorders.evidence();
    assert!(ev.installed);
    assert!(ev.deployments >= 1);
    assert!(
        ev.fills_attributed >= 1,
        "the fill was claimed by a deployment: {ev:?}"
    );
    assert!(
        matches!(ev.verdict(), ForwardRecordingVerdict::Recording { .. }),
        "an empty forward record in this state would genuinely mean no trading: {:?}",
        ev.verdict()
    );
    ForwardRecorders::require_recording().expect("a deployed, attached process records");
}

/// A second `install` joins the first rather than subscribing again.
///
/// Two subscriptions would deliver every fill to every deployment twice, and a
/// double-counted forward observation inflates exactly the number Set T cannot
/// manufacture more of.
#[tokio::test]
async fn installing_twice_does_not_double_count_a_fill() {
    ensure_journal();
    let first = ForwardRecorders::install();
    let second = ForwardRecorders::install();
    assert!(
        Arc::ptr_eq(&first, &second),
        "install must be idempotent, not additive"
    );

    let halts = Arc::new(StrategyHaltRegistry::new());
    let recorder = second
        .deploy(spec("twice-1"), dec("10000"), Arc::clone(&halts))
        .expect("a valid paper deployment");
    recorder.open(at(BAR), px("100")).unwrap();

    let engine = filling_engine();
    let intent = buy_intent("twice-1");
    recorder.on_submit(&intent, px("100"));
    submit(&engine, intent).await;

    assert_eq!(
        recorder.position(),
        dec("10"),
        "one fill must move the position once, however many times install ran"
    );
}

// ── 2. A recorder off the registry is inert, and cannot be put on it ─────────

/// `ForwardRecorder::new` still exists — the unit tests in `forward.rs` need a
/// recorder they can drive by hand — but one built that way is not on the fill
/// stream and cannot be put there: it does not implement `FillObserver`, so
/// `execution::fills::register_observer(recorder)` does not compile.
///
/// The shape that used to ship was the opposite: a recorder that *could* be
/// registered, and a platform where nobody had.
#[tokio::test]
async fn a_recorder_built_outside_the_registry_never_sees_a_fill() {
    ensure_journal();
    let _registry = ForwardRecorders::install();

    let halts = Arc::new(StrategyHaltRegistry::new());
    let detached = ForwardRecorder::new(spec("detached-1"), dec("10000"), Arc::clone(&halts))
        .expect("a valid paper deployment");
    detached.open(at(BAR), px("100")).unwrap();

    let engine = filling_engine();
    let intent = buy_intent("detached-1");
    detached.on_submit(&intent, px("100"));
    submit(&engine, intent).await;

    assert_eq!(
        detached.position(),
        Decimal::ZERO,
        "a recorder nobody deployed receives nothing — and this is visible \
         rather than assumed, because `deploy` is the only way to attach one"
    );
}

// ── 3. The ratchet on the production call site ───────────────────────────────

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/reconciliation sits two levels below the repo root")
        .to_path_buf()
}

/// The code of a file: everything before the first top-level `#[cfg(test)]`
/// (this repo puts its test modules last, so cutting there is what separates a
/// production call site from a test one), with comment lines dropped.
///
/// Dropping comments is load-bearing in both directions. `execution/src/lib.rs`
/// *names* `fills::register_observer` in a doc link and calls it nowhere, and a
/// scan that counted that would report a subscriber that does not exist. In the
/// other direction, a file that only mentions `ForwardRecorders::install` in a
/// "TODO: wire this up" must not count as wired — which is the exact shape of
/// the failure this whole file exists because of.
fn production_source(path: &Path) -> String {
    let body = std::fs::read_to_string(path).unwrap_or_default();
    // The trailing test *module*, not the first `#[cfg(test)]`: a test-only
    // `mod` declaration near the top of a file (apps/platform/src/main.rs has
    // `#[cfg(test)] mod test_support;` on line 20) would otherwise cut the whole
    // file out and make a wired binary read as having no wire at all.
    let body = body.replace("\r\n", "\n");
    let code = match body.find("\n#[cfg(test)]\nmod tests") {
        Some(i) => &body[..i],
        None => &body[..],
    };
    code.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `.rs` file under `apps/*/src` and `crates/*/src`, excluding the shadow
/// worktrees under `.claude/`.
fn production_files() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    let root = repo_root();
    let mut files = Vec::new();
    for top in ["apps", "crates"] {
        let Ok(entries) = std::fs::read_dir(root.join(top)) else {
            continue;
        };
        for entry in entries.flatten() {
            walk(&entry.path().join("src"), &mut files);
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            let body = production_source(&p);
            (rel, body)
        })
        .collect()
}

/// The paste-ready wire, quoted here so the failure message carries it.
const CALL_SITE: &str = "\
    // Forward evidence (Set T): attach reconciliation to the fill stream before
    // the first order can be submitted, then refuse to serve if nothing is
    // listening.  Without this a paper credential buys elapsed calendar time
    // and an empty `mlops.forward_observation`.
    let forward_recorders = reconciliation::ForwardRecorders::install();
    execution::fills::require_recording()
        .expect(\"the fill stream must be recorded before any order is placed\");
";

/// Production binaries that construct an `ExecutionEngine` and do not install
/// forward recording.
///
/// **Empty since 2026-09-27**, when `apps/platform/src/main.rs` — the one entry
/// this list has ever held — installed the registry beside its engine. The list
/// stays, and the assertion stays exact equality rather than becoming
/// `is_empty()`, because the two directions are different failures and both
/// have happened: a new binary appearing here is a process that trades without
/// recording, and an entry reappearing is the wire rotting back out. Exact
/// equality is also the acknowledgement the last attempt never had to make —
/// closing the hole edits this file, so nobody can believe it is closed without
/// having said so here.
const UNWIRED_ENGINE_SITES: &[&str] = &[];

#[test]
fn every_production_binary_that_can_place_an_order_is_accounted_for() {
    let unwired: Vec<String> = production_files()
        .into_iter()
        .filter(|(_, body)| body.contains("ExecutionEngine::new"))
        .filter(|(_, body)| !body.contains("ForwardRecorders::install"))
        .map(|(rel, _)| rel)
        .collect();

    assert_eq!(
        unwired, UNWIRED_ENGINE_SITES,
        "\n\nThe set of production sources that build an ExecutionEngine without \
         installing forward recording has changed.\n\
         Fills taken by such a binary are written to `fills` and to no forward \
         record, and `mlops.forward_observation` stays empty in a way nobody can \
         tell apart from a platform that never traded.\n\n\
         If a file appeared: add this to it, immediately after the engine is \
         constructed, and delete the entry here.\n\n{CALL_SITE}\n\
         If a file disappeared: it is now wired — delete its entry from \
         UNWIRED_ENGINE_SITES.\n"
    );
}

/// The wire is in a binary, and it is in the *one* binary that can place orders.
///
/// [`every_production_binary_that_can_place_an_order_is_accounted_for`] is
/// satisfied by a workspace with no binaries at all, which is how a ratchet
/// quietly stops ratcheting: delete the call site and the engine together and
/// the list is still empty. This asserts the positive fact separately — some
/// production source both builds an engine and installs the registry — so the
/// two cannot fail silently in the same direction.
#[test]
fn the_binary_that_builds_an_engine_is_the_one_that_installs_the_registry() {
    let wired: Vec<String> = production_files()
        .into_iter()
        .filter(|(_, body)| {
            body.contains("ExecutionEngine::new") && body.contains("ForwardRecorders::install")
        })
        .map(|(rel, _)| rel)
        .collect();

    assert!(
        !wired.is_empty(),
        "no production source both constructs an ExecutionEngine and installs forward recording. Either the platform binary lost its engine, or it lost the wire in T-0.1's title — and an empty UNWIRED_ENGINE_SITES cannot tell those apart from a closed hole"
    );
}

/// One seam, not two.
///
/// The half-built `ExecutionEngine::fill_observer` field existed beside
/// `fills::register_observer` for months, set by nobody and read by nobody. A
/// second wire is worse than none: an observer on one of them records nothing
/// of the other's fills while looking, from outside, exactly like a process
/// that did not trade.
#[test]
fn reconciliations_registry_is_the_only_production_fill_subscriber() {
    let subscribers: Vec<String> = production_files()
        .into_iter()
        .filter(|(rel, body)| {
            body.contains("fills::register_observer") && rel != "crates/execution/src/fills.rs"
        })
        .map(|(rel, _)| rel)
        .collect();

    assert_eq!(
        subscribers,
        vec!["crates/reconciliation/src/recorder.rs".to_owned()],
        "the fill stream must have exactly one production subscriber, and it \
         must be ForwardRecorders::install"
    );
}
