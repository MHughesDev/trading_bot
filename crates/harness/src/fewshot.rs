//! Worked select→fill examples, chosen by the step's namespace (AGENT-008 §3.5,
//! Set S task S-1.17).
//!
//! `orchestration.few_shot_examples` has been declared in both local profiles since
//! the profiles existed and read by nothing but `appendix_a`'s band check — a value
//! that validated and never reached a prompt. The harness crate's own rule is that a
//! profile value nothing enforces is a bug, so this is either wired or deleted.
//!
//! **Why examples at all, and only here.** A flat-tier model is asked to do two
//! things it is bad at: pick one name from a menu, then emit an object that
//! satisfies a schema. Both are format problems rather than reasoning problems, and
//! a worked example is the cheapest correction for a format problem that exists —
//! cheaper than a longer instruction, and much cheaper than a retry.
//!
//! Frontier tiers get none. They do not need the format help, and two examples in
//! the prefix is two examples paid for on every turn. `few_shot_examples: 0` is the
//! frontier profiles' setting and this module honours it by returning nothing.
//!
//! **The examples are canned, not retrieved.** A retrieved example is a second thing
//! that can be wrong, and an example drawn from this run's own transcript teaches the
//! model to repeat what it just did — which is the failure mode when it is stuck.
//! These are hand-written, generic over the namespace, and contain no instrument, no
//! date and no number a model could copy into an answer.

use crate::context::{Block, Section};
use crate::provenance::Tagged;

/// A worked pair: the menu line the model saw, and the two replies that were right.
struct Example {
    /// Namespace this teaches. `None` is the fallback, used when the step's
    /// namespace has no example of its own.
    namespace: Option<&'static str>,
    select: &'static str,
    fill: &'static str,
}

/// Deliberately few. Two are rendered at most, and a library of thirty would be a
/// library nobody audits — each of these is read on every step of every local run.
const EXAMPLES: &[Example] = &[
    Example {
        namespace: Some("data"),
        select: r#"{"name": "get_bars"}"#,
        fill: r#"{"instrument_id": "<the instrument the step names>", "timeframe": "1h", "limit": 200}"#,
    },
    Example {
        namespace: Some("discovery"),
        select: r#"{"name": "list_instruments"}"#,
        fill: r#"{}"#,
    },
    Example {
        namespace: Some("research"),
        select: r#"{"name": "get_funnel"}"#,
        fill: r#"{"experiment_id": "<the id a previous result returned>"}"#,
    },
    Example {
        namespace: Some("falsification"),
        select: r#"{"name": "check_tradeable_frontier"}"#,
        fill: r#"{"instrument_id": "<the instrument the step names>", "holding_bars": 60}"#,
    },
    // The fallback, and the most important one: it teaches the *shape* rather than
    // any particular tool, and it is what a step in an unfamiliar namespace gets.
    Example {
        namespace: None,
        select: r#"{"name": "<one name from the list above, copied exactly>"}"#,
        fill: r#"{"<the fields the schema marks required>": "<their values>"}"#,
    },
];

/// The few-shot block for this step, or `None` when the profile asks for none.
///
/// `namespaces` are the step's, in routing order, so the first example is the one
/// most likely to match what the model is about to do. The fallback is appended when
/// there is room and nothing else filled it, because a model with one example and a
/// shape is better off than one with a single example it cannot generalise from.
#[must_use]
pub fn block(namespaces: &[String], want: u32) -> Option<Block> {
    if want == 0 {
        return None;
    }
    let want = want as usize;
    let mut chosen: Vec<&Example> = Vec::with_capacity(want);
    for ns in namespaces {
        if chosen.len() >= want {
            break;
        }
        if let Some(e) = EXAMPLES.iter().find(|e| e.namespace == Some(ns.as_str())) {
            chosen.push(e);
        }
    }
    // The shape example, when there is room. It is last because it is the weakest
    // of the pair: a concrete tool teaches more than a placeholder.
    if chosen.len() < want {
        if let Some(fallback) = EXAMPLES.iter().find(|e| e.namespace.is_none()) {
            chosen.push(fallback);
        }
    }
    if chosen.is_empty() {
        return None;
    }

    let mut text = String::with_capacity(512);
    text.push_str(
        "WORKED EXAMPLES — this is the SHAPE of a correct reply, not the answer to \
         your step:\n",
    );
    for e in chosen {
        text.push_str("\n  you are asked to choose:  ");
        text.push_str(e.select);
        text.push_str("\n  then for its arguments:   ");
        text.push_str(e.fill);
        text.push('\n');
    }
    text.push_str("\nReply with the object only. No prose before it, no code fence around it.\n");

    Some(Block {
        section: Section::FewShot,
        tagged: Tagged::system("examples", text),
        // Below the charter and the state card, above raw observations: an example
        // is worth more than an old tool result and less than the task.
        priority: 240,
        reference: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(namespaces: &[&str], want: u32) -> Option<String> {
        let ns: Vec<String> = namespaces.iter().map(|s| (*s).to_string()).collect();
        block(&ns, want).map(|b| b.tagged.render())
    }

    /// A frontier profile declares zero, and zero means none — not "one anyway".
    #[test]
    fn a_profile_asking_for_none_gets_none() {
        assert!(rendered(&["data"], 0).is_none());
    }

    /// The step's namespace picks the example, so the worked pair is the one the
    /// model is about to need.
    #[test]
    fn the_example_matches_the_step_namespace() {
        let d = rendered(&["data"], 1).expect("data has an example");
        assert!(d.contains("get_bars"), "{d}");
        assert!(!d.contains("list_instruments"));

        let x = rendered(&["discovery"], 1).expect("discovery has one");
        assert!(x.contains("list_instruments"), "{x}");
    }

    /// An unfamiliar namespace still gets the shape. A model with no example at all
    /// is the case this field exists for.
    #[test]
    fn an_unknown_namespace_falls_back_to_the_shape() {
        let out = rendered(&["nothing_like_this"], 2).expect("the fallback always exists");
        assert!(out.contains("copied exactly"), "{out}");
        assert!(out.contains("the fields the schema marks required"));
    }

    /// Two means two. A block that quietly rendered five would be five examples
    /// charged to every step of every local run.
    #[test]
    fn the_count_is_the_count() {
        let out = rendered(&["data", "discovery", "research"], 2).expect("some");
        assert_eq!(out.matches("you are asked to choose").count(), 2);
    }

    /// No instrument, no date, no number a model could lift into an answer. An
    /// example that looks like data is an example that gets cited as data.
    #[test]
    fn no_example_carries_a_value_that_could_be_mistaken_for_a_result() {
        let out = rendered(&["data", "research", "falsification"], 8).expect("some");
        for forbidden in ["BTC-USD", "ETH", "2026", "2025", "sharpe", "0."] {
            assert!(
                !out.contains(forbidden),
                "an example contains {forbidden}, which a model can copy into a finding: {out}"
            );
        }
    }
}
