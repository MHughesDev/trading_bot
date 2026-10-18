//! Reading memory: the task brief (AGENT-007 §4, P4; Set S tasks S-4.5, S-4.6).
//!
//! # What a brief is
//!
//! At run start, before the first model call, the harness turns the store into
//! `Reference` blocks under that section's share (AGENT-005 §4.5). Every item is
//! one line, its ids, and **why it was retrieved** — the last of those being the
//! part P4 adds to an ordinary search result and the part that makes a surprising
//! retrieval debuggable after the fact.
//!
//! # The slots, and why they are shares rather than a single ranked list
//!
//! A single ranked list is simpler and is wrong in a way that is hard to see: the
//! findings slot wins it. Findings are numerous, recent and similar to the goal,
//! so a flat ranker returns eight findings, no negative result, and no
//! preference — and the run re-derives a dead idea while ignoring an instruction
//! the user already gave. §4's per-slot shares exist to stop that, and
//! `always ≥ 1 if any match` on negative results is the same rule stated for the
//! slot most likely to be crowded out.
//!
//! | Slot | Share | Rule |
//! |---|---|---|
//! | [`Slot::UserPreference`] | 15% | top rank in its scope; the user already decided |
//! | [`Slot::InstrumentProfile`] | 25% | most recent only |
//! | [`Slot::Finding`] | 35% | `stale` retrieved but flagged; `established` before `suggestive` |
//! | [`Slot::NegativeResult`] | 15% | ≥ 1 whenever anything matches; met retry conditions flagged |
//! | [`Slot::OpenQuestion`] | 10% | `blocking` first |
//! | [`Slot::Conflict`] | — | **always included, both sides**, outside the shares |
//!
//! # Conflicts are outside the budget on purpose
//!
//! §4's table gives conflicts no share and says "always included, both sides".
//! A conflict that loses its slot to a ranker is a conflict that was silently
//! resolved by ranking — which is precisely what P5 forbids. So both sides are
//! admitted before the shares are applied, and they are the one thing here that
//! can push the brief over a slot's allowance.
//!
//! # No embeddings, on purpose, for now
//!
//! §6: retrieval must never block a run on an embedding service. The scorer here
//! is the lexical one `registry::search` already uses, over `claim + subject`,
//! and it works with the sidecar stopped — which §8.8 requires and
//! `retrieval_works_with_the_embedding_sidecar_stopped` asserts.

use serde::{Deserialize, Serialize};

use crate::context::{estimate_tokens, Block, Section};
use crate::provenance::Tagged;

use super::{Entry, Kind};

/// Which slot an item was retrieved into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    UserPreference,
    InstrumentProfile,
    Finding,
    NegativeResult,
    OpenQuestion,
    Conflict,
}

impl Slot {
    /// Share of the `Reference` budget (AGENT-007 §4).
    #[must_use]
    pub fn share(self) -> f32 {
        match self {
            Slot::UserPreference => 0.15,
            Slot::InstrumentProfile => 0.25,
            Slot::Finding => 0.35,
            Slot::NegativeResult => 0.15,
            Slot::OpenQuestion => 0.10,
            // Outside the shares by design — see the module docs.
            Slot::Conflict => 0.0,
        }
    }

    /// Assembly order. Preferences first because the user's own decision
    /// outranks anything the platform inferred.
    pub const ORDER: &'static [Slot] = &[
        Slot::UserPreference,
        Slot::InstrumentProfile,
        Slot::Finding,
        Slot::NegativeResult,
        Slot::OpenQuestion,
    ];
}

/// How much of each entry to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    /// One line per item. Every local tier.
    Lines,
    /// One line plus the entry's structured detail. Frontier only.
    Full,
}

/// One retrieved item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BriefItem {
    pub entry_id: String,
    pub slot: Slot,
    pub rank: usize,
    /// The rendered line, ids included.
    pub line: String,
    /// P4: *why it was retrieved*.
    pub why: String,
}

/// One row of `memory_retrievals` (AGENT-007 §4's last paragraph).
///
/// `cited` starts false and is set when a later `record_finding` or
/// `finish_task` names the entry's id. It is the memory-recall signal
/// AGENT-009 §4 scores, which is the one place these two specs meet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retrieval {
    pub run_id: String,
    pub entry_id: String,
    pub rank: usize,
    pub slot: Slot,
    pub why: String,
    pub cited: bool,
}

/// The assembled brief.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Brief {
    pub items: Vec<BriefItem>,
    pub retrievals: Vec<Retrieval>,
    /// Entries that matched but did not fit. Reported rather than dropped
    /// silently — a brief that quietly omits half the store is the degradation
    /// §2.4 warns about, observed from the inside.
    pub elided: usize,
}

impl Brief {
    /// Turn the brief into `Reference` blocks for [`crate::context::assemble`].
    ///
    /// One block per slot rather than one per item: the assembler drops whole
    /// blocks under pressure, and dropping "the negative results" as a unit is
    /// more legible than losing three of five to a size cap.
    #[must_use]
    pub fn blocks(&self) -> Vec<Block> {
        let mut out = Vec::new();
        for slot in [
            Slot::Conflict,
            Slot::UserPreference,
            Slot::InstrumentProfile,
            Slot::Finding,
            Slot::NegativeResult,
            Slot::OpenQuestion,
        ] {
            let lines: Vec<String> = self
                .items
                .iter()
                .filter(|i| i.slot == slot)
                .map(|i| format!("{}  why: {}", i.line, i.why))
                .collect();
            if lines.is_empty() {
                continue;
            }
            out.push(Block {
                section: Section::Reference,
                tagged: Tagged::system(
                    slot_label(slot),
                    format!("{}:\n{}", slot_label(slot), lines.join("\n")),
                ),
                // Conflicts rank above the rest of Reference for the same reason
                // they are outside the shares: a silently dropped conflict is a
                // silently resolved one.
                priority: if slot == Slot::Conflict { 210 } else { 190 },
                reference: None,
            });
        }
        out
    }

    /// Mark a retrieval cited, when a later claim names the entry's id.
    ///
    /// Returns whether anything changed, so the caller can tell "cited" from
    /// "named an id that was never retrieved".
    pub fn mark_cited(&mut self, entry_id: &str) -> bool {
        let mut hit = false;
        for r in &mut self.retrievals {
            if r.entry_id == entry_id && !r.cited {
                r.cited = true;
                hit = true;
            }
        }
        hit
    }

    /// The memory-recall score AGENT-009's scorecard reads.
    #[must_use]
    pub fn recall(&self) -> f64 {
        if self.retrievals.is_empty() {
            return 0.0;
        }
        let cited = self.retrievals.iter().filter(|r| r.cited).count();
        cited as f64 / self.retrievals.len() as f64
    }
}

fn slot_label(s: Slot) -> &'static str {
    match s {
        Slot::UserPreference => "what you have already been told",
        Slot::InstrumentProfile => "instrument profile",
        Slot::Finding => "established elsewhere",
        Slot::NegativeResult => "already ruled out",
        Slot::OpenQuestion => "open questions",
        Slot::Conflict => "CONFLICTING EVIDENCE — both sides, unresolved",
    }
}

/// Words that discriminate between nothing. The same list `registry` keeps for
/// the same reason, reached through the same scorer's shape.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "does", "was", "are", "has", "have",
    "into", "over", "its", "any", "all", "can", "use", "using", "get", "run", "give", "tell",
    "what", "how", "why", "when", "which", "who", "data", "value", "values",
];

/// Lexical relevance of an entry to a query.
///
/// Deliberately the same dumb scorer as `registry::search`: subject terms weigh
/// double, claim terms weigh one. A retrieval miss must be predictable, and a
/// second model call to rank memory is a second thing that can be wrong.
fn relevance(entry: &Entry, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let subject = entry.subject.to_lowercase();
    let claim = entry.claim.to_lowercase();
    terms
        .iter()
        .map(|t| {
            usize::from(subject.contains(t.as_str())) * 2 + usize::from(claim.contains(t.as_str()))
        })
        .sum()
}

fn terms_of(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() > 2 && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect()
}

/// Rank: relevance × recency × confidence × corroboration (§4).
///
/// Corroboration is §2.5's payoff: a finding measured on three windows outranks
/// a singleton, which is the reason corroborating beats deduplicating.
fn rank_score(entry: &Entry, terms: &[String], today: &str) -> f64 {
    let rel = relevance(entry, terms) as f64;
    if rel == 0.0 && entry.kind != Kind::UserPreference {
        return 0.0;
    }
    let age_days = super::days_between(&entry.data_window.end, today)
        .unwrap_or(0)
        .max(0) as f64;
    // Half-life of a year. Gentle: a finding does not stop being true on a
    // schedule, and `valid_until` already carries the hard edge.
    let recency = 0.5f64.powf(age_days / 365.0);
    let corroboration = 1.0 + (f64::from(entry.corroborations.saturating_sub(1)) * 0.25);
    let stale_penalty = if entry.stale { 0.6 } else { 1.0 };
    rel.max(1.0) * recency * entry.confidence.weight() * corroboration * stale_penalty
}

/// Whether a negative result's retry condition is now satisfied.
///
/// The platform decides this, not the model: `met_conditions` is a set the
/// caller assembles from the world (an adapter that now exists, a window that
/// now extends). §4's "flagged: retry condition met" is exactly this.
fn retry_met(entry: &Entry, met_conditions: &[String]) -> bool {
    entry
        .extra
        .get("retry_condition")
        .and_then(serde_json::Value::as_str)
        .map(|c| met_conditions.iter().any(|m| c.contains(m.as_str())))
        .unwrap_or(false)
}

/// Assemble the brief (AGENT-007 §4).
///
/// * `reference_budget_tokens` — the `Reference` section's share of the window.
/// * `met_conditions` — retry conditions the platform can see are now satisfied.
/// * `scopes` — the user-preference scopes in play for this goal.
#[must_use]
// Eight, and every one is a separate fact the brief needs. Bundling them into a
// struct would move the argument list rather than shorten it.
#[allow(clippy::too_many_arguments)]
pub fn assemble_brief(
    run_id: &str,
    goal: &str,
    entries: &[Entry],
    reference_budget_tokens: u32,
    detail: Detail,
    today: &str,
    met_conditions: &[String],
    scopes: &[String],
) -> Brief {
    let terms = terms_of(goal);
    let mut brief = Brief::default();

    let live: Vec<&Entry> = entries.iter().filter(|e| !e.archived).collect();

    // ── Conflicts first, outside the shares, both sides ──────────────────────
    //
    // A conflicting pair is admitted whenever *either* side matches the goal.
    // Retrieving only the matching half is how a ranker resolves a conflict
    // without anyone deciding to.
    let mut conflicted: Vec<&Entry> = Vec::new();
    for e in &live {
        if e.conflicts_with.is_empty() {
            continue;
        }
        let self_matches = relevance(e, &terms) > 0;
        let peer_matches = e
            .conflicts_with
            .iter()
            .any(|id| live.iter().any(|o| &o.id == id && relevance(o, &terms) > 0));
        if self_matches || peer_matches {
            conflicted.push(e);
        }
    }
    conflicted.sort_by(|a, b| a.id.cmp(&b.id));
    for (rank, e) in conflicted.iter().enumerate() {
        push(
            &mut brief,
            run_id,
            e,
            Slot::Conflict,
            rank,
            format!(
                "conflicts with {} — both sides are retrieved and neither is downgraded",
                e.conflicts_with.join(", ")
            ),
            detail,
        );
    }
    let already: Vec<String> = brief.items.iter().map(|i| i.entry_id.clone()).collect();

    // ── The slots ────────────────────────────────────────────────────────────
    for slot in Slot::ORDER {
        let kind = match slot {
            Slot::UserPreference => Kind::UserPreference,
            Slot::InstrumentProfile => Kind::InstrumentProfile,
            Slot::Finding => Kind::Finding,
            Slot::NegativeResult => Kind::NegativeResult,
            Slot::OpenQuestion => Kind::OpenQuestion,
            Slot::Conflict => continue,
        };
        let mut candidates: Vec<(f64, &Entry)> = live
            .iter()
            .filter(|e| e.kind == kind && !already.contains(&e.id))
            .filter(|e| match slot {
                // A preference in scope is retrieved whether or not it shares a
                // word with the goal: "always use 1h bars" matches no query and
                // is the whole point of §3.4.
                Slot::UserPreference => {
                    let s = e
                        .extra
                        .get("scope")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("global");
                    s == "global" || scopes.iter().any(|sc| sc == s)
                }
                // §4: "plus any whose retry_condition is now satisfied".
                Slot::NegativeResult => relevance(e, &terms) > 0 || retry_met(e, met_conditions),
                Slot::OpenQuestion => {
                    relevance(e, &terms) > 0
                        && e.extra
                            .get("status")
                            .and_then(|s| s.get("closed"))
                            .is_none()
                }
                _ => true,
            })
            .map(|e| (rank_score(e, &terms, today), *e))
            .filter(|(s, e)| *s > 0.0 || e.kind == Kind::UserPreference)
            .collect();

        candidates.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                // `blocking` questions first (AGENT-013 §9), then by id so the
                // brief is byte-stable across runs — a brief that reorders is a
                // prompt-cache miss on every step.
                .then_with(|| blocking_first(a.1, b.1))
                .then_with(|| a.1.id.cmp(&b.1.id))
        });

        if *slot == Slot::InstrumentProfile {
            // "most recent only": one per subject.
            let mut seen: Vec<&str> = Vec::new();
            candidates.retain(|(_, e)| {
                if seen.contains(&e.subject.as_str()) {
                    false
                } else {
                    seen.push(&e.subject);
                    true
                }
            });
        }

        let slot_budget = (reference_budget_tokens as f32 * slot.share()) as u32;
        let mut used = 0u32;
        let mut any_admitted = false;
        for (rank, (_, e)) in candidates.iter().enumerate() {
            let why = why_for(*slot, e, &terms, met_conditions);
            let line = render(e, detail);
            let cost = estimate_tokens(&format!("{line}  why: {why}\n"));
            // §4's floor: a negative result that matched is ALWAYS admitted, even
            // when its slot has no room. Crowding out the entry most likely to
            // save the budget is the one failure this slot exists to prevent.
            let forced = *slot == Slot::NegativeResult && !any_admitted;
            if !forced && used + cost > slot_budget {
                brief.elided += candidates.len() - rank;
                break;
            }
            used += cost;
            any_admitted = true;
            push(&mut brief, run_id, e, *slot, rank, why, detail);
        }
    }
    brief
}

fn blocking_first(a: &Entry, b: &Entry) -> std::cmp::Ordering {
    let key = |e: &Entry| {
        u8::from(
            !e.extra
                .get("blocking")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        )
    };
    key(a).cmp(&key(b))
}

fn render(e: &Entry, detail: Detail) -> String {
    match detail {
        Detail::Lines => e.line(),
        Detail::Full => {
            let extra = if e.extra.is_null() {
                String::new()
            } else {
                format!("\n    {}", e.extra)
            };
            format!("{}{extra}", e.line())
        }
    }
}

fn why_for(slot: Slot, e: &Entry, terms: &[String], met: &[String]) -> String {
    let hit: Vec<&str> = terms
        .iter()
        .filter(|t| {
            e.subject.to_lowercase().contains(t.as_str())
                || e.claim.to_lowercase().contains(t.as_str())
        })
        .map(String::as_str)
        .collect();
    let base = if hit.is_empty() {
        match slot {
            Slot::UserPreference => "the user told you this; it applies to this scope".to_string(),
            _ => "in scope for this goal".to_string(),
        }
    } else {
        format!("matches {}", hit.join(", "))
    };
    let mut why = base;
    if slot == Slot::NegativeResult && retry_met(e, met) {
        why.push_str(" — RETRY CONDITION MET: this was ruled out, and the thing that ruled it out has changed");
    }
    if e.stale {
        why.push_str(" — STALE: past valid_until, not re-verified");
    }
    why
}

fn push(
    brief: &mut Brief,
    run_id: &str,
    e: &Entry,
    slot: Slot,
    rank: usize,
    why: String,
    detail: Detail,
) {
    brief.items.push(BriefItem {
        entry_id: e.id.clone(),
        slot,
        rank,
        line: render(e, detail),
        why: why.clone(),
    });
    brief.retrievals.push(Retrieval {
        run_id: run_id.to_string(),
        entry_id: e.id.clone(),
        rank,
        slot,
        why,
        cited: false,
    });
}

/// The ids among `retrieved` that `text` names (S-6.10).
///
/// What a run **cited** is what its `record_finding` / `finish_task` evidence
/// names, and the driver feeds that text here. An id counts only as a whole
/// token: `f_1` is not cited by a claim that mentions `f_12`, and an id the brief
/// never showed is not returned however confidently the model names it (the
/// counterpart of [`Brief::mark_cited`]'s "named an id that was never retrieved").
#[must_use]
pub fn cited_ids(retrieved: &[Retrieval], text: &str) -> Vec<String> {
    let tokens: std::collections::BTreeSet<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(|t| t.trim_matches(|c: char| c == '.' || c == '-'))
        .filter(|t| !t.is_empty())
        .collect();
    let mut out: Vec<String> = Vec::new();
    for r in retrieved {
        if tokens.contains(r.entry_id.as_str()) && !out.contains(&r.entry_id) {
            out.push(r.entry_id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Confidence, DataWindow, Direction, Entry, Kind, Measurement, Provenance};

    fn prov() -> Provenance {
        Provenance::new("run_1", "run_backtest", "v3", "tpl")
    }

    fn finding(id: &str, subject: &str, claim: &str, conf: Confidence) -> Entry {
        let mut e = Entry::finding(
            id,
            subject,
            claim,
            DataWindow::new("2026-04-01", "2026-06-30"),
            prov(),
        )
        .with_measurement(Measurement::new("sharpe", Direction::Positive, 0.4).with_n(812))
        .expiring();
        e.confidence = conf;
        e
    }

    fn negative(id: &str, subject: &str, claim: &str, retry: Option<&str>) -> Entry {
        let mut e = finding(id, subject, claim, Confidence::Established);
        e.id = id.to_string();
        e.kind = Kind::NegativeResult;
        e.valid_until = None;
        e.extra = serde_json::json!({
            "failure_class": "capacity_insufficient",
            "ticket_id": "tkt_00099911",
            "retry_condition": retry,
        });
        e
    }

    fn preference(id: &str, scope: &str, text: &str) -> Entry {
        let mut e = finding(id, "global", text, Confidence::Established);
        e.id = id.to_string();
        e.kind = Kind::UserPreference;
        e.extra = serde_json::json!({ "scope": scope, "statement": text });
        e
    }

    /// AGENT-007 §8.2 / S-4.5: a second conversation receives the first's
    /// established findings **with ids**, at a 7B share and a frontier share.
    #[test]
    fn a_second_conversation_receives_the_first_findings_with_ids_on_both_tiers() {
        let store = vec![
            finding(
                "f_31a",
                "BTC-USD 1h",
                "momentum survives costs",
                Confidence::Established,
            ),
            finding("f_9", "ETH-USD 1d", "unrelated", Confidence::Established),
        ];
        for (budget, detail) in [(400u32, Detail::Lines), (4000u32, Detail::Full)] {
            let b = assemble_brief(
                "run_2",
                "does BTC-USD 1h momentum survive costs",
                &store,
                budget,
                detail,
                "2026-07-01",
                &[],
                &[],
            );
            let ids: Vec<&str> = b.items.iter().map(|i| i.entry_id.as_str()).collect();
            assert!(ids.contains(&"f_31a"), "{ids:?} at budget {budget}");
            assert!(
                b.items.iter().any(|i| i.line.contains("f_31a")),
                "the id must be in the rendered line, not only in the row"
            );
            assert!(
                b.items.iter().all(|i| !i.why.is_empty()),
                "P4: every item says why it was retrieved"
            );
        }
    }

    /// S-4.5 and §8.4: two conflicting findings are both retrieved and marked.
    #[test]
    fn both_sides_of_a_conflict_are_retrieved_and_marked() {
        let mut a = finding(
            "f_a",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        );
        let mut b = finding(
            "f_b",
            "BTC-USD 1h",
            "momentum does not survive costs",
            Confidence::Established,
        );
        a.conflicts_with.push("f_b".into());
        b.conflicts_with.push("f_a".into());
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &[a, b],
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        let conflict: Vec<&BriefItem> = brief
            .items
            .iter()
            .filter(|i| i.slot == Slot::Conflict)
            .collect();
        assert_eq!(conflict.len(), 2, "both sides, always: {:?}", brief.items);
        assert!(conflict.iter().all(|i| i.why.contains("conflicts with")));
        assert!(brief
            .blocks()
            .iter()
            .any(|bl| bl.tagged.content.contains("CONFLICTING EVIDENCE")));
    }

    /// Even when only one side matches the query. Retrieving the matching half
    /// alone is a ranker resolving a conflict nobody decided to resolve.
    #[test]
    fn a_conflict_is_retrieved_whole_when_only_one_side_matches_the_goal() {
        let mut a = finding(
            "f_a",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        );
        let mut b = finding(
            "f_b",
            "BTC-USD 1h",
            "nothing here matches the words used",
            Confidence::Established,
        );
        a.conflicts_with.push("f_b".into());
        b.conflicts_with.push("f_a".into());
        let brief = assemble_brief(
            "run_2",
            "momentum survives costs",
            &[a, b],
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(
            brief
                .items
                .iter()
                .filter(|i| i.slot == Slot::Conflict)
                .count(),
            2
        );
    }

    /// §8.5 / S-4.5: a stale finding is flagged rather than hidden.
    #[test]
    fn a_stale_finding_is_retrieved_and_flagged() {
        let mut e = finding(
            "f_old",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        );
        e.stale = true;
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &[e],
            400,
            Detail::Lines,
            "2027-01-01",
            &[],
            &[],
        );
        let item = &brief.items[0];
        assert!(item.line.contains("STALE"), "{}", item.line);
        assert!(item.why.contains("STALE"), "{}", item.why);
    }

    /// §4's floor: the entry most likely to save a budget is never crowded out.
    #[test]
    fn a_matching_negative_result_is_admitted_even_at_a_budget_of_zero() {
        let store = vec![negative(
            "n_1",
            "BTC-USD 1h",
            "momentum ruled out at gate 2",
            None,
        )];
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &store,
            1,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(brief.items.len(), 1, "always >= 1 if any match");
        assert_eq!(brief.items[0].slot, Slot::NegativeResult);
    }

    #[test]
    fn a_met_retry_condition_is_retrieved_and_flagged() {
        let store = vec![negative(
            "n_1",
            "SOL-USD",
            "no execution adapter",
            Some("retry if an execution adapter for Kraken exists"),
        )];
        let brief = assemble_brief(
            "run_2",
            "trade BTC-USD momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &["execution adapter for Kraken".to_string()],
            &[],
        );
        assert_eq!(brief.items.len(), 1, "a met condition retrieves on its own");
        assert!(brief.items[0].why.contains("RETRY CONDITION MET"));
    }

    /// S-4.6: the user's own knowledge is retrieved at top rank, in scope, and
    /// without needing to share a word with the goal.
    #[test]
    fn a_user_preference_in_scope_is_first_and_needs_no_lexical_match() {
        let store = vec![
            finding(
                "f_1",
                "BTC-USD 1h",
                "momentum survives costs",
                Confidence::Established,
            ),
            preference("p_1", "global", "never use 1m bars before 2026-03"),
        ];
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(brief.items[0].entry_id, "p_1", "{:?}", brief.items);
        let out_of_scope = preference("p_2", "ETH-USD", "prefer 4h");
        let brief2 = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &[out_of_scope],
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &["BTC-USD".to_string()],
        );
        assert!(brief2.items.is_empty(), "out of scope, not retrieved");
    }

    /// S-4.6's second half: a deleted preference is gone from the next brief.
    #[test]
    fn a_deleted_preference_leaves_the_next_brief() {
        let mut p = preference("p_1", "global", "never use 1m bars");
        let with = assemble_brief(
            "r",
            "anything",
            std::slice::from_ref(&p),
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(with.items.len(), 1);
        p.archived = true; // what DELETE /api/memory/preferences does
        let without = assemble_brief(
            "r",
            "anything",
            &[p],
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert!(without.items.is_empty());
    }

    /// AGENT-007 §8.8 and §6: retrieval must never block on an embedding
    /// service. This scorer has no service to block on, and that is the test.
    #[test]
    fn retrieval_works_with_the_embedding_sidecar_stopped() {
        let store = vec![finding(
            "f_1",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        )];
        let brief = assemble_brief(
            "run_2",
            "BTC-USD momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(brief.items.len(), 1, "lexical retrieval needs no sidecar");
    }

    /// The findings slot cannot eat the whole brief. Without per-slot shares a
    /// flat ranker returns eight findings and no negative result.
    #[test]
    fn the_findings_slot_cannot_crowd_out_the_others() {
        let mut store: Vec<Entry> = (0..30)
            .map(|i| {
                finding(
                    &format!("f_{i}"),
                    "BTC-USD 1h",
                    "momentum survives costs",
                    Confidence::Established,
                )
            })
            .collect();
        store.push(negative(
            "n_1",
            "BTC-USD 1h",
            "momentum ruled out at gate 2",
            None,
        ));
        store.push(preference("p_1", "global", "never use 1m bars"));
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        let slots: std::collections::BTreeSet<Slot> = brief.items.iter().map(|i| i.slot).collect();
        assert!(slots.contains(&Slot::NegativeResult), "{slots:?}");
        assert!(slots.contains(&Slot::UserPreference), "{slots:?}");
        assert!(brief.elided > 0, "elision is reported, not silent");
    }

    /// §4's last paragraph, and AGENT-009's memory-recall metric.
    #[test]
    fn cited_is_set_when_a_later_claim_names_the_entry() {
        let store = vec![finding(
            "f_31a",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        )];
        let mut brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(brief.recall(), 0.0);
        assert!(brief.mark_cited("f_31a"));
        assert!((brief.recall() - 1.0).abs() < f64::EPSILON);
        assert!(
            !brief.mark_cited("f_never_retrieved"),
            "naming an id that was never retrieved is not recall"
        );
    }

    /// A corroborated finding outranks a singleton — §2.5's payoff, and the
    /// reason corroborating beats deduplicating.
    #[test]
    fn a_corroborated_finding_outranks_a_singleton() {
        let single = finding(
            "f_single",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        );
        let mut many = finding(
            "f_many",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        );
        many.corroborations = 4;
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            &[single, many],
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(brief.items[0].entry_id, "f_many", "{:?}", brief.items);
        assert!(brief.items[0].line.contains("corroborated×4"));
    }

    #[test]
    fn an_archived_entry_is_out_of_retrieval_but_the_id_still_exists() {
        let mut e = finding(
            "f_1",
            "BTC-USD 1h",
            "momentum survives costs",
            Confidence::Established,
        );
        e.archived = true;
        let brief = assemble_brief(
            "run_2",
            "BTC-USD 1h momentum",
            std::slice::from_ref(&e),
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert!(brief.items.is_empty());
        assert_eq!(e.id, "f_1");
    }

    #[test]
    fn the_brief_is_byte_stable_across_two_identical_assemblies() {
        let store = vec![
            finding(
                "f_1",
                "BTC-USD 1h",
                "momentum survives",
                Confidence::Established,
            ),
            finding(
                "f_2",
                "BTC-USD 1h",
                "momentum survives",
                Confidence::Established,
            ),
        ];
        let a = assemble_brief(
            "r",
            "BTC-USD momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        let b = assemble_brief(
            "r",
            "BTC-USD momentum",
            &store,
            400,
            Detail::Lines,
            "2026-07-01",
            &[],
            &[],
        );
        assert_eq!(a, b, "a reordering brief is a prompt-cache miss every step");
    }

    /// S-6.10: only whole ids the brief showed are cited.
    #[test]
    fn a_citation_is_a_whole_retrieved_id() {
        let mk = |id: &str| Retrieval {
            run_id: "r".into(),
            entry_id: id.into(),
            rank: 1,
            slot: Slot::Finding,
            why: String::new(),
            cited: false,
        };
        let shown = vec![mk("f_1"), mk("f_2"), mk("q-7")];
        assert_eq!(cited_ids(&shown, "f_1, q-7."), vec!["f_1", "q-7"]);
        assert!(cited_ids(&shown, "f_12 and xf_1").is_empty(), "prefix is not a cite");
        assert!(cited_ids(&shown, "f_99").is_empty(), "never shown, never cited");
        assert_eq!(cited_ids(&shown, "[f_2] f_2"), vec!["f_2"], "once");
    }
}
