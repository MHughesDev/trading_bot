//! A finding's data window comes from the evidence it cites (OQ-18; S-4.2).
//!
//! # The question this answers
//!
//! Every entry carries a `data_window` (AGENT-007 §2.2) and the loop tracks
//! none. That blocked `record_finding` from writing through, and the blockage
//! mattered more than it looked: §2.2.1's near-duplicate check is "same subject,
//! same measured quantity, **overlapping window**", so the window is one of the
//! three keys the conflict machinery joins on. A wrong one does not fail — it
//! quietly stops two findings about the same period from being compared, or
//! starts comparing two about different ones.
//!
//! # Derived from the cited results, and refused when it cannot be
//!
//! The claim rests on the results it cites and on nothing else, so its window is
//! theirs. Every producer stamps one in the same shape — the analysis templates,
//! `GET /api/data/bars`, `preview_signals` — as `window: {start, end}`, which is
//! what this reads.
//!
//! When no cited result carries a window, this returns `None` and the caller
//! **refuses the finding**. That is the whole point of deriving rather than
//! defaulting: a finding whose window cannot be established is a finding nobody
//! can later check for conflict, and storing it under a guessed window would
//! make it look checkable. The refusal is cheap and says what to cite instead;
//! the guess is undetectable afterwards.
//!
//! # The union, not the intersection
//!
//! A claim citing a January backtest and a June one is a claim about both, so
//! the stored window spans them. Two consequences, and the second is why:
//!
//! * The intersection of disjoint windows is empty, which would make a claim
//!   resting on two periods carry no window at all — the case most in need of
//!   one.
//! * The union is the wider bound, so it reports **more** overlaps, so more
//!   pairs reach the conflict check. P5 says conflicts are surfaced and never
//!   merged; erring toward surfacing is erring in the direction the spec wants.

use super::DataWindow;

/// The earliest start and latest end among the cited results.
///
/// `evidence` is the comma-separated id list `record_finding` receives;
/// `results_by_id` is the text each id arrived in.
#[must_use]
pub fn from_evidence(
    evidence: &str,
    results_by_id: &std::collections::BTreeMap<String, String>,
) -> Option<DataWindow> {
    let mut earliest: Option<String> = None;
    let mut latest: Option<String> = None;

    for id in evidence.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some(text) = results_by_id.get(id) else {
            continue;
        };
        let Some((start, end)) = window_in(text) else {
            continue;
        };
        // Lexical comparison, which is ordering for ISO-8601 and only for
        // ISO-8601 — hence `is_iso_date` below. A format that sorted differently
        // would produce a window that looked fine and bounded the wrong period.
        if earliest.as_ref().is_none_or(|e| start < *e) {
            earliest = Some(start);
        }
        if latest.as_ref().is_none_or(|l| end > *l) {
            latest = Some(end);
        }
    }

    match (earliest, latest) {
        (Some(s), Some(e)) if s <= e => Some(DataWindow::new(s, e)),
        _ => None,
    }
}

/// `YYYY-MM-DD`, possibly with a time after it.
///
/// Checked rather than assumed because the comparison above is lexical: a date
/// in any other format sorts wrongly and would bound the wrong period while
/// looking entirely valid.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// The `start` and `end` of one result, as dates.
///
/// Reads `window: {start, end}` wherever it appears in the document — nested,
/// because a template puts it under `detail.window` and the data route puts it
/// at the top level, and a reader that knew only one of those shapes would find
/// windows for some tools and not others.
fn window_in(text: &str) -> Option<(String, String)> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    find_window(&doc)
}

fn find_window(v: &serde_json::Value) -> Option<(String, String)> {
    if let Some(map) = v.as_object() {
        if let Some(w) = map.get("window").and_then(serde_json::Value::as_object) {
            let start = w.get("start").and_then(serde_json::Value::as_str);
            let end = w.get("end").and_then(serde_json::Value::as_str);
            if let (Some(s), Some(e)) = (start, end) {
                if is_iso_date(s) && is_iso_date(e) {
                    return Some((s[..10].to_string(), e[..10].to_string()));
                }
            }
        }
        for value in map.values() {
            if let Some(found) = find_window(value) {
                return Some(found);
            }
        }
    }
    if let Some(list) = v.as_array() {
        for value in list {
            if let Some(found) = find_window(value) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn results(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn one_cited_result_gives_its_own_window() {
        let r = results(&[(
            "exp_1",
            r#"{"detail":{"window":{"start":"2026-04-01","end":"2026-06-30"}}}"#,
        )]);
        assert_eq!(
            from_evidence("exp_1", &r),
            Some(DataWindow::new("2026-04-01", "2026-06-30"))
        );
    }

    /// Two windows become their union, not their overlap.
    ///
    /// The union reports more overlaps, so more pairs reach §2.2.1's conflict
    /// check — and P5 says conflicts are surfaced, never merged.
    #[test]
    fn two_cited_results_span_both() {
        let r = results(&[
            (
                "a",
                r#"{"window":{"start":"2026-01-01","end":"2026-03-31"}}"#,
            ),
            (
                "b",
                r#"{"window":{"start":"2026-06-01","end":"2026-08-31"}}"#,
            ),
        ]);
        assert_eq!(
            from_evidence("a, b", &r),
            Some(DataWindow::new("2026-01-01", "2026-08-31")),
            "the intersection of these is empty, which would leave the claim most in need of \
             a window carrying none"
        );
    }

    /// A window nested anywhere is found.
    #[test]
    fn the_window_is_found_wherever_the_producer_put_it() {
        for text in [
            r#"{"window":{"start":"2026-04-01","end":"2026-06-30"}}"#,
            r#"{"detail":{"window":{"start":"2026-04-01","end":"2026-06-30"}}}"#,
            r#"{"detail":{"a":{"b":{"window":{"start":"2026-04-01","end":"2026-06-30"}}}}}"#,
            r#"{"results":[{"window":{"start":"2026-04-01","end":"2026-06-30"}}]}"#,
        ] {
            let r = results(&[("x", text)]);
            assert_eq!(
                from_evidence("x", &r),
                Some(DataWindow::new("2026-04-01", "2026-06-30")),
                "not found in {text}"
            );
        }
    }

    /// No window in the evidence means no window, which means a refusal.
    #[test]
    fn evidence_with_no_window_yields_none_rather_than_a_guess() {
        let r = results(&[("x", r#"{"detail":{"sharpe":1.4}}"#)]);
        assert_eq!(
            from_evidence("x", &r),
            None,
            "a guessed window is undetectable afterwards; a refusal costs one turn"
        );
        assert_eq!(from_evidence("", &r), None);
        assert_eq!(from_evidence("not_an_id", &r), None);
    }

    /// A timestamp is truncated to its date, and a non-ISO date is ignored.
    ///
    /// The comparison is lexical, which is ordering for ISO-8601 and nothing
    /// else — a `04/01/2026` would sort before every ISO date and bound the
    /// wrong period while looking valid.
    #[test]
    fn only_iso_dates_are_read() {
        let r = results(&[(
            "x",
            r#"{"window":{"start":"2026-04-01T00:00:00Z","end":"2026-06-30T23:59:59Z"}}"#,
        )]);
        assert_eq!(
            from_evidence("x", &r),
            Some(DataWindow::new("2026-04-01", "2026-06-30"))
        );

        let bad = results(&[("y", r#"{"window":{"start":"04/01/2026","end":"06/30/2026"}}"#)]);
        assert_eq!(from_evidence("y", &bad), None);
    }

    /// A backwards window is refused rather than silently reordered.
    ///
    /// Reordering would hide a producer that is emitting them wrongly, and the
    /// database would refuse the row anyway — better to find out here, where the
    /// message can name the evidence.
    #[test]
    fn a_backwards_window_is_not_quietly_swapped() {
        // One result whose own start is after its end.
        let r = results(&[("x", r#"{"window":{"start":"2026-06-30","end":"2026-04-01"}}"#)]);
        // The union takes min(start) and max(end): 2026-06-30 and 2026-04-01,
        // which fails the ordering check rather than being swapped.
        assert_eq!(from_evidence("x", &r), None);
    }
}
