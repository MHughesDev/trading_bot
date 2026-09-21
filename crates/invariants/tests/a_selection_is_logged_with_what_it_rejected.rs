//! A selection that is not in the decision log is a selection nobody can
//! question (AGENT-014 P2, §5.3; SPEC §4.4, INV-21; Set S task S-11.3).
//!
//! # What is actually at risk
//!
//! The session's own log already records *which* item was chosen. That is enough
//! to render a board and useless for the thing the decision log exists for:
//! "why that one, out of what?". §5.3 requires the full candidate set with its
//! signals, the propensity under the mixture, and the exploration flag —
//! together they are what make the selection policy a learning-to-rank dataset
//! later rather than a trace, and what makes an off-policy estimate over these
//! rows possible at all.
//!
//! A driver that appended `selected` and skipped the decision row would look
//! completely normal: the desk would be right, the agenda would be right, and
//! the dataset would be silently missing every choice. Nothing in the type system
//! catches it, because both are `async fn`s returning `Result<()>`.
//!
//! So this reads the driver's source. That is weaker than a type and it is the
//! check that exists; the two things it holds are the two that would have to
//! change for the gap to open.
//!
//! # And the share
//!
//! §5.2 says the exploration share is the charter's and is not in any tool
//! schema. A constant in the driver would satisfy every test in `select.rs` —
//! they take the share as an argument — and would quietly pin every session to
//! one value, including sessions whose user raised it. The scan below requires
//! the driver to read it from the session.

const DRIVER: &str = include_str!("../../api/src/session_driver.rs");
const SELECT: &str = include_str!("../../research-session/src/select.rs");

/// The body of a named `fn` in the driver, comments stripped.
///
/// Comments are removed because several of them quote the very strings this
/// scans for — a guard that passed because its own explanation named the thing
/// it was looking for has happened in this repository before.
fn body_of(name: &str) -> String {
    let after = DRIVER
        .split_once(name)
        .unwrap_or_else(|| panic!("`{name}` is gone from the driver; this test is stale"))
        .1;
    let end = after.find("\n    /// ").unwrap_or(after.len());
    after[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The scanner reads a body, not an empty string.
///
/// Every source-scanning check in this repository has to guard this separately:
/// a `find` that stops matching returns nothing, and a test over nothing passes.
#[test]
fn the_scanner_still_reads_the_functions_it_was_written_for() {
    for name in [
        "async fn choose(",
        "async fn log_selection(",
        "async fn measure(",
    ] {
        let body = body_of(name);
        assert!(
            body.lines().count() > 10,
            "`{name}` came back as {} lines, which is not a function body — the driver's shape              changed and every assertion below is now checking an empty string",
            body.lines().count()
        );
    }
}

#[test]
fn choosing_writes_a_decision_row_as_well_as_an_event() {
    let choose = body_of("async fn choose(");
    assert!(
        choose.contains("self.log_selection("),
        "`choose` appends `Selected` without writing the `select` decision. The board would be \
         right and the one log every off-policy estimate reads would be silently missing every \
         choice the session made."
    );
    assert!(
        choose.contains("EventKind::Selected"),
        "`choose` no longer records the selection on the session's own log"
    );
}

#[test]
fn the_decision_carries_the_whole_candidate_set_and_the_propensity() {
    let log = body_of("async fn log_selection(");
    assert!(
        log.contains("candidate_set: select::candidate_set(scored)"),
        "the decision's candidate set is not built from the slate that was scored. §5.3 requires \
         every scored item with its signals — a subset is a dataset that silently omits the \
         options the policy rejected, which are the only rows that make it a dataset."
    );
    assert!(
        log.contains("propensity: Some(selection.propensity)"),
        "the decision carries no propensity, and `Decision::validate` only refuses that for a \
         legacy context — so it would be accepted and be unusable"
    );
    assert!(
        log.contains("exploration_flag: selection.exploration"),
        "the exploration flag is not the draw's own answer, so the achieved share cannot be \
         measured against the floor it is supposed to hold"
    );
    assert!(
        log.contains("kind: ledger::DecisionKind::Select"),
        "the decision is not recorded as a `select`"
    );
}

#[test]
fn the_share_comes_from_the_session_and_not_from_a_constant() {
    let choose = body_of("async fn choose(");
    assert!(
        choose.contains("session.exploration_share()"),
        "the driver does not read the share from the session. A constant here would pin every \
         session to one value — including one whose user raised it — and every test in \
         `select.rs` would still pass, because they take the share as an argument."
    );
    // And the seed is the platform's, derived from the session and its position,
    // not a clock: a clock-seeded draw cannot be replayed from the row, which
    // makes the recorded propensity a claim rather than a fact.
    assert!(
        choose.contains("select::seed_for(session.id, folded.event_seq)"),
        "the draw's seed is not derived from the session and its log position, so a decision row \
         cannot be replayed"
    );
}

#[test]
fn a_user_item_is_taken_without_being_drawn_or_logged_as_a_draw() {
    let choose = body_of("async fn choose(");
    let bypass = choose
        .split_once("bypasses_scoring()")
        .expect("the §4.2 bypass is gone from `choose`")
        .1;
    let bypass = &bypass[..bypass.find("return Ok(Some").unwrap_or(bypass.len())];
    assert!(
        !bypass.contains("log_selection"),
        "a `user` item is being recorded as a draw. It was not drawn — it was asked for — and a \
         propensity attached to it would put a fabricated probability into the decision log."
    );
    assert!(
        bypass.contains("bypassed_scoring"),
        "the bypass is not recorded on the session log, so the board cannot show why an item \
         jumped the ranking"
    );
}

/// The floor is INV-21 at the agenda level, and the same argument.
#[test]
fn the_exploration_floor_is_the_one_in_the_charter() {
    assert!(
        (research_session::EXPLORATION_FLOOR - 0.10).abs() < f64::EPSILON,
        "the floor moved; AGENT-014 §5.2 and INV-21 both name 0.10"
    );
    // A bounded session does not select, and still reports a share at or above
    // the floor — so "the share is never below the floor" is true everywhere,
    // not only where a charter exists.
    let bounded = research_session::ResearchSession::bounded(
        uuid::Uuid::nil(),
        uuid::Uuid::nil(),
        "q",
        "opus",
    );
    assert!(bounded.exploration_share() >= research_session::EXPLORATION_FLOOR);
}

/// Feasibility is `{0, 0.5, 1}` and the zero has to survive combination.
///
/// The one property of the scorer that a wrong reading of "percentile-normalise
/// every signal" would destroy, and it would destroy it silently: an impossible
/// item would come out of normalisation with a positive rank and could be
/// chosen. Asserted here as well as in `select.rs` because it is the difference
/// between a ranking and a refusal.
#[test]
fn normalisation_cannot_make_an_impossible_item_possible() {
    assert!(
        SELECT.contains("// A rank in (0, 1] as a divisor")
            || SELECT.contains("cost_pct[n]"),
        "the cost signal is no longer ranked, so one expensive outlier can flatten the slate"
    );
    let combine = SELECT
        .split_once("let normalised = Score::combine(")
        .expect("the scorer still combines a normalised score")
        .1;
    let combine = &combine[..combine.find(");").unwrap_or(combine.len())];
    let stripped: String = combine
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        stripped.contains("inputs.feasibility.raw"),
        "feasibility is being normalised rather than used as measured. A percentile rank turns \
         the lowest value in the slate into something greater than zero, and feasibility's whole \
         job is that zero means impossible: {stripped}"
    );
    assert!(
        stripped.contains("inputs.novelty.raw"),
        "novelty is being normalised, and its zero means `already answered`: {stripped}"
    );
}
