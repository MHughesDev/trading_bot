//! Compile errors that teach (ADR-0045; the `teach.rs` approach carried forward).
//!
//! A diagnostic names *where* (the node or rule), *what* (a stable code a tool can
//! branch on), *how to fix it*, and — where one helps — an example. The same
//! mistake produces the same diagnostic whether it came from the visual editor,
//! SLv3 text, a builder tool-call or raw JSON, because all four reach this
//! compiler through the same canonical graph.

use std::fmt;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Stable machine-readable code, e.g. `type_mismatch`.
    pub code: &'static str,
    /// Where: a node id, `rule[2]`, `select.score`, `params`.
    pub at: String,
    pub message: String,
    pub fix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

impl Diagnostic {
    pub(crate) fn new(
        code: &'static str,
        at: impl Into<String>,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            code,
            at: at.into(),
            message: message.into(),
            fix: fix.into(),
            example: None,
        }
    }

    #[must_use]
    pub(crate) fn example(mut self, e: impl Into<String>) -> Self {
        self.example = Some(e.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] at {}: {} — fix: {}",
            self.code, self.at, self.message, self.fix
        )?;
        if let Some(e) = &self.example {
            write!(f, " (e.g. {e})")?;
        }
        Ok(())
    }
}

/// The closest of `candidates` to `word`, if any is within a small edit distance
/// — for "did you mean" fixes.
pub(crate) fn nearest<'a>(
    word: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let cap = (word.len() / 3).max(2);
    candidates
        .into_iter()
        .map(|c| (levenshtein(word, c), c))
        .filter(|(d, _)| *d <= cap)
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)))
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_suggests_a_close_name_and_nothing_for_a_distant_one() {
        let names = ["crossed_above", "crossed_below", "rolling_mean"];
        assert_eq!(nearest("crosed_above", names), Some("crossed_above"));
        assert_eq!(nearest("zzzz", names), None);
    }
}
