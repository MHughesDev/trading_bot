//! Fitting a selected set into a budget (AGENT-005 §16, §25.6; Set S tasks
//! S-9.3 and S-9.16).
//!
//! # Degrade-or-drop is a choice, not a sequence
//!
//! The obvious implementation walks each record's ladder greedily: take the
//! largest, shrink it a rung, check, repeat. That is wrong for a reason worth
//! stating, because it is not obvious — **degrading one large valuable record may
//! free less than dropping two small worthless ones, and it loses more**. Greedy
//! never considers the second option, so it pays for the cheap seats with the
//! expensive one.
//!
//! So each step considers every available cut — one rung down any record, or
//! dropping any record — and takes the one with the best *value lost per token
//! recovered*. That is bounded work over tens of records and it is the difference
//! between "the budget fit" and "the budget fit well".
//!
//! # The one modelled quantity, stated rather than buried
//!
//! Dropping a record loses all of its value; that is not a model, it is
//! arithmetic. Degrading a record loses *some* of it, and how much is not
//! measurable from the token counts alone. [`DEGRADE_LOSS_FACTOR`] is the
//! assumption: a rung that removes half the tokens loses half of that share of
//! the value, because the ladders are built so the kept half carries the schema,
//! the ids, the n and the summary — the parts a reader needs to know what they
//! are looking at and whether to ask for more.
//!
//! It lives in the policy (§22) so it is fitted with everything else rather than
//! argued about, and `ADR-S9-02` records why it is a parameter rather than a
//! constant: a value of 1.0 makes degradation and dropping identical per token
//! and the fitter collapses into "drop the cheapest", which is exactly the
//! behaviour Part I already had.
//!
//! # No record is ever cut at a byte offset
//!
//! A record that reaches the bottom of its ladder and still does not fit is
//! **dropped**, and it keeps its manifest line and its handle (§17). There is no
//! call to `truncate_middle` anywhere in this path, and there is a test that
//! greps for one.

use super::body::{Body, Rung};
use super::store::{Record, RecordId};

/// Share of a rung's token saving that is treated as lost value.
///
/// See the module header: 1.0 would make the fitter equivalent to "drop the
/// cheapest", and 0.0 would make degradation free and it is not.
pub const DEGRADE_LOSS_FACTOR: f32 = 0.5;

/// What a shrink step did, for the marker the model reads (§16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Degraded {
    pub from_tokens: u32,
    pub to_tokens: u32,
    pub how: Rung,
}

/// A record offered to the fitter, with the value selection put on it.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub record: &'a Record,
    /// Relative worth, from selection. Only ratios matter.
    pub value: f32,
    /// Records the fitter may not drop or degrade: the pinned set (§15) and
    /// §24.7's corrections for the upcoming tool.
    pub pinned: bool,
}

impl<'a> Candidate<'a> {
    #[must_use]
    pub fn new(record: &'a Record, value: f32) -> Self {
        Self {
            record,
            value,
            pinned: false,
        }
    }

    #[must_use]
    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }
}

/// One record as it will be rendered.
#[derive(Debug, Clone)]
pub struct Fitted {
    pub id: RecordId,
    pub body: Body,
    pub tokens: u32,
    /// `None` at full fidelity.
    pub degraded: Option<Degraded>,
}

/// The outcome of fitting a selected set.
#[derive(Debug, Clone, Default)]
pub struct Fit {
    pub kept: Vec<Fitted>,
    /// Dropped whole, in the order they were cut. They stay in the store and in
    /// the manifest; this is what the `SelectionReport` reports (§28.3).
    pub dropped: Vec<RecordId>,
    /// Value the cuts cost, in the same units as `Candidate::value`.
    pub value_lost: f32,
    /// True when the pinned set alone exceeds the budget. The caller must not
    /// silently ship an over-budget prompt; Part I's assembler still enforces
    /// the hard cap, and this says the fitter could not help.
    pub over_budget: bool,
}

impl Fit {
    #[must_use]
    pub fn tokens(&self) -> u32 {
        self.kept.iter().map(|f| f.tokens).sum()
    }

    /// Value retained, for comparing two fitters over one fixture.
    #[must_use]
    pub fn value_kept(&self, candidates: &[Candidate<'_>]) -> f32 {
        let total: f32 = candidates.iter().map(|c| c.value).sum();
        total - self.value_lost
    }
}

/// One cut the fitter could make.
struct Cut {
    index: usize,
    /// `None` means drop the record entirely.
    to: Option<(Body, u32, Rung)>,
    loss: f32,
    recovered: u32,
}

impl Cut {
    /// Value lost per token recovered — the quantity the fitter minimises.
    fn rate(&self) -> f32 {
        if self.recovered == 0 {
            f32::INFINITY
        } else {
            self.loss / self.recovered as f32
        }
    }
}

/// Fit a selected set into `budget` tokens, choosing cuts by value lost per
/// token recovered (§25.6).
#[must_use]
pub fn fit(candidates: &[Candidate<'_>], budget: u32, loss_factor: f32) -> Fit {
    fit_with(candidates, budget, loss_factor, false)
}

/// The greedy fitter Part II replaces, kept so the choice can be *measured*
/// rather than asserted.
///
/// It is not dead code and it is not a fallback: `the_fitter_beats_greedy`
/// compares retained value over a fixture built to make the difference visible,
/// which is S-9.16's acceptance. A comparison against a baseline that is not
/// implemented is not a comparison.
#[must_use]
pub fn fit_greedy(candidates: &[Candidate<'_>], budget: u32, loss_factor: f32) -> Fit {
    fit_with(candidates, budget, loss_factor, true)
}

fn fit_with(candidates: &[Candidate<'_>], budget: u32, loss_factor: f32, greedy: bool) -> Fit {
    let mut kept: Vec<Option<Fitted>> = candidates
        .iter()
        .map(|c| {
            Some(Fitted {
                id: c.record.id,
                body: c.record.body.clone(),
                tokens: c.record.body.tokens(),
                degraded: None,
            })
        })
        .collect();
    let mut out = Fit::default();

    let total = |kept: &[Option<Fitted>]| -> u32 { kept.iter().flatten().map(|f| f.tokens).sum() };

    // Bounded: every iteration either drops a record or walks one rung, and
    // both are finite per record.
    let mut guard = candidates.len() * 16 + 16;
    while total(&kept) > budget && guard > 0 {
        guard -= 1;
        // Greedy takes the largest record and walks its ladder, never
        // considering the drop-two-small alternative. That is the behaviour
        // being measured against, so it is implemented as written rather than
        // as a worse scorer, and it does not pay for the full scan it ignores.
        if greedy {
            let Some(cut) = greedy_cut(&kept, candidates) else {
                break;
            };
            apply(&mut kept, &mut out, candidates, cut);
            continue;
        }
        let mut best: Option<Cut> = None;
        for (i, slot) in kept.iter().enumerate() {
            let Some(current) = slot else { continue };
            if candidates[i].pinned {
                continue;
            }
            let value = candidates[i].value.max(0.0);
            // Cut A: one rung down.
            if let Some((next, how)) = current.body.shrink() {
                let to_tokens = next.tokens();
                if to_tokens < current.tokens {
                    let recovered = current.tokens - to_tokens;
                    let share = recovered as f32 / current.tokens.max(1) as f32;
                    best = better(
                        best,
                        Cut {
                            index: i,
                            to: Some((next, to_tokens, how)),
                            loss: value * share.min(1.0) * loss_factor,
                            recovered,
                        },
                    );
                }
            }
            // Cut B: drop it whole.
            best = better(
                best,
                Cut {
                    index: i,
                    to: None,
                    loss: value,
                    recovered: current.tokens,
                },
            );
        }
        let Some(cut) = best else { break };
        apply(&mut kept, &mut out, candidates, cut);
    }

    out.over_budget = total(&kept) > budget;
    out.kept = kept.into_iter().flatten().collect();
    out
}

/// Apply one chosen cut.
///
/// `from_tokens` in the marker stays the *original* size across repeated rungs:
/// a record told it shrank by a little when it shrank by a lot makes `expand`
/// look cheaper than it is.
fn apply(kept: &mut [Option<Fitted>], out: &mut Fit, candidates: &[Candidate<'_>], cut: Cut) {
    out.value_lost += cut.loss;
    match cut.to {
        Some((body, tokens, how)) => {
            let slot = kept[cut.index].as_mut().expect("chosen from a live slot");
            let original = slot.degraded.map_or(slot.tokens, |d| d.from_tokens);
            slot.body = body;
            slot.tokens = tokens;
            slot.degraded = Some(Degraded {
                from_tokens: original,
                to_tokens: tokens,
                how,
            });
        }
        None => {
            out.dropped.push(candidates[cut.index].record.id);
            kept[cut.index] = None;
        }
    }
}

fn better(best: Option<Cut>, c: Cut) -> Option<Cut> {
    match best {
        // Ties break toward the earlier record so the fitter is deterministic:
        // §24.6's rule, and a map-order tie-break would make the same store
        // render differently between runs.
        Some(b) if b.rate() <= c.rate() => Some(b),
        _ => Some(c),
    }
}

/// The baseline: shrink the largest unpinned record a rung, and drop it only
/// when its ladder is exhausted.
fn greedy_cut(kept: &[Option<Fitted>], candidates: &[Candidate<'_>]) -> Option<Cut> {
    let (i, current) = kept
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.as_ref().map(|f| (i, f)))
        .filter(|(i, _)| !candidates[*i].pinned)
        .max_by_key(|(i, f)| (f.tokens, std::cmp::Reverse(*i)))?;
    let value = candidates[i].value.max(0.0);
    match current.body.shrink() {
        Some((next, how)) if next.tokens() < current.tokens => {
            let to_tokens = next.tokens();
            let recovered = current.tokens - to_tokens;
            let share = recovered as f32 / current.tokens.max(1) as f32;
            Some(Cut {
                index: i,
                to: Some((next, to_tokens, how)),
                loss: value * share.min(1.0) * DEGRADE_LOSS_FACTOR,
                recovered,
            })
        }
        _ => Some(Cut {
            index: i,
            to: None,
            loss: value,
            recovered: current.tokens,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Column;
    use crate::context::store::{Draft, RecordKind, RunStore};

    fn table_record(store: &mut RunStore, rows: usize) -> RecordId {
        store.insert(Draft::new(
            1,
            RecordKind::ToolResult,
            "get_bars",
            Body::Table {
                schema: vec![
                    Column::new("ts", 200),
                    Column::new("close", 200),
                    Column::new("note", 10),
                ],
                rows: (0..rows)
                    .map(|i| {
                        vec![
                            format!("2024-01-{:02}", i % 28 + 1),
                            format!("{}", 40000 + i),
                            "a note of some length".into(),
                        ]
                    })
                    .collect(),
                total_rows: rows,
            },
        ))
    }

    fn text_record(store: &mut RunStore, bytes: usize) -> RecordId {
        store.insert(Draft::new(
            1,
            RecordKind::Note,
            "harness",
            Body::text("x".repeat(bytes)),
        ))
    }

    /// Everything fitting is a no-op: no degradation, no drops.
    #[test]
    fn under_budget_nothing_is_touched() {
        let mut s = RunStore::new();
        let a = table_record(&mut s, 4);
        let cands = vec![Candidate::new(s.get(a).unwrap(), 1.0)];
        let f = fit(&cands, 100_000, DEGRADE_LOSS_FACTOR);
        assert!(f.dropped.is_empty());
        assert!(f.kept[0].degraded.is_none());
        assert!(!f.over_budget);
    }

    /// Every shrink is a declared ladder step, and the marker says which.
    #[test]
    fn every_shrink_is_a_declared_rung() {
        let mut s = RunStore::new();
        let a = table_record(&mut s, 400);
        let cands = vec![Candidate::new(s.get(a).unwrap(), 1.0)];
        let f = fit(&cands, 200, DEGRADE_LOSS_FACTOR);
        let d = f.kept[0].degraded.expect("it had to shrink");
        assert!(d.to_tokens < d.from_tokens);
        assert!(
            matches!(
                d.how,
                Rung::DroppedColumns | Rung::SampledRows | Rung::Aggregated | Rung::SchemaOnly
            ),
            "{d:?}"
        );
        assert!(
            f.kept[0].body.render().contains("ts"),
            "the schema survives, which a byte cut would not have"
        );
    }

    /// A record that bottoms out and still does not fit is dropped, not cut.
    #[test]
    fn a_record_that_cannot_shrink_enough_is_dropped_whole() {
        let mut s = RunStore::new();
        let a = text_record(&mut s, 8000);
        let cands = vec![Candidate::new(s.get(a).unwrap(), 1.0)];
        let f = fit(&cands, 10, DEGRADE_LOSS_FACTOR);
        assert_eq!(f.dropped, vec![a]);
        assert!(f.kept.is_empty(), "dropped, never cut to fit");
        assert!(
            s.get(a).is_some(),
            "and it is still in the store, which is what makes it recoverable"
        );
    }

    /// Pinned records are neither dropped nor degraded.
    #[test]
    fn a_pinned_record_survives_a_squeeze() {
        let mut s = RunStore::new();
        let pin = text_record(&mut s, 400);
        let big = table_record(&mut s, 400);
        let cands = vec![
            Candidate::new(s.get(pin).unwrap(), 0.01).pinned(),
            Candidate::new(s.get(big).unwrap(), 10.0),
        ];
        let f = fit(&cands, 150, DEGRADE_LOSS_FACTOR);
        let kept_pin = f.kept.iter().find(|k| k.id == pin).expect("pinned kept");
        assert!(
            kept_pin.degraded.is_none(),
            "a pinned record is not degraded either, however cheap it is"
        );
    }

    /// The case §25.6 exists for.
    ///
    /// One large valuable table, and several small worthless notes. Greedy
    /// attacks the largest thing — the valuable table — while the fitter buys
    /// the same tokens by dropping the notes, which cost almost nothing.
    #[test]
    fn the_fitter_beats_greedy_where_it_matters() {
        let mut s = RunStore::new();
        let valuable = table_record(&mut s, 300);
        let junk: Vec<RecordId> = (0..6).map(|_| text_record(&mut s, 1200)).collect();

        let mut cands = vec![Candidate::new(s.get(valuable).unwrap(), 100.0)];
        cands.extend(
            junk.iter()
                .map(|id| Candidate::new(s.get(*id).unwrap(), 0.2)),
        );

        let full: u32 = cands.iter().map(|c| c.record.body.tokens()).sum();
        let budget = full * 2 / 5;

        let smart = fit(&cands, budget, DEGRADE_LOSS_FACTOR);
        let greedy = fit_greedy(&cands, budget, DEGRADE_LOSS_FACTOR);

        assert!(smart.tokens() <= budget, "the fitter fits");
        assert!(greedy.tokens() <= budget, "so does greedy");
        assert!(
            smart.value_kept(&cands) > greedy.value_kept(&cands),
            "fitter kept {:.2}, greedy kept {:.2} — the point of §25.6 is that these differ",
            smart.value_kept(&cands),
            greedy.value_kept(&cands)
        );
        assert!(
            smart.dropped.iter().all(|id| *id != valuable),
            "and the fitter did not pay for the cheap seats with the expensive one"
        );
    }

    /// Deterministic: the same candidates and budget give the same cuts.
    #[test]
    fn fitting_is_deterministic() {
        let mut s = RunStore::new();
        let ids: Vec<RecordId> = (0..8)
            .map(|i| {
                if i % 2 == 0 {
                    table_record(&mut s, 60)
                } else {
                    text_record(&mut s, 900)
                }
            })
            .collect();
        let cands: Vec<Candidate<'_>> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Candidate::new(s.get(*id).unwrap(), 1.0 + i as f32 * 0.1))
            .collect();
        let a = fit(&cands, 400, DEGRADE_LOSS_FACTOR);
        let b = fit(&cands, 400, DEGRADE_LOSS_FACTOR);
        assert_eq!(a.dropped, b.dropped);
        assert_eq!(
            a.kept.iter().map(|k| (k.id, k.tokens)).collect::<Vec<_>>(),
            b.kept.iter().map(|k| (k.id, k.tokens)).collect::<Vec<_>>()
        );
    }

    /// A budget the pinned set alone cannot meet is reported, never hidden.
    #[test]
    fn an_unmeetable_budget_is_reported() {
        let mut s = RunStore::new();
        let pin = text_record(&mut s, 8000);
        let cands = vec![Candidate::new(s.get(pin).unwrap(), 1.0).pinned()];
        let f = fit(&cands, 10, DEGRADE_LOSS_FACTOR);
        assert!(
            f.over_budget,
            "the caller must not ship an over-budget prompt believing it fit"
        );
    }

    /// The marker names the *original* size, not the previous rung's.
    ///
    /// Two rungs down, `from_tokens` must still be what the record cost whole —
    /// otherwise the model is told a record shrank by a little when it shrank by
    /// a lot, and `expand` looks cheaper than it is.
    #[test]
    fn the_marker_reports_the_full_original_size() {
        let mut s = RunStore::new();
        let a = table_record(&mut s, 400);
        let full = s.get(a).unwrap().body.tokens();
        let cands = vec![Candidate::new(s.get(a).unwrap(), 1.0)];
        let f = fit(&cands, 60, DEGRADE_LOSS_FACTOR);
        let d = f.kept[0].degraded.expect("shrank");
        assert_eq!(d.from_tokens, full);
        assert!(d.to_tokens < full / 2, "and it shrank a lot");
    }
}
