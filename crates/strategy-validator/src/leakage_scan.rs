//! `feature_leakage_scan` -- a static pass over feature and analysis **code** for the
//! constructs that read the future (AGENT-006 §5.4; Set S task S-3.11).
//!
//! # Where this can find anything, and where it cannot
//!
//! The first version of this task was deferred because the frozen expression grammar
//! (`expressions::check_call`) admits two calls, `feature(name)` and `bar(field)`, and
//! every feature a backtest evaluates is causal: a scan over *strategy definitions*
//! "has nothing expressible to find", and a scan that returns clean on every strategy
//! the platform can represent reads as evidence there is nothing to find. That reasoning
//! stands, and this module does not scan definitions.
//!
//! What it scans is the other place an agent writes a transform: **code it submits**
//! (a `run_code` cell, a skill, a feature script). There a full-window statistic, a
//! negative shift, a centred rolling window, a backward fill and a scaler fitted on the
//! whole frame are all one line away, none of the platform's structural causality
//! applies, and a strategy fitted on such a column looks wonderful in a backtest that
//! never could have traded it. That is the leak this finds. ADR-S3-14: scan submitted
//! source, not definitions (rejected: scanning definitions -- nothing is expressible, so
//! it could only ever say "clean").
//!
//! # What "no findings" means
//!
//! Not "no leak". A static pass finds the patterns it has a rule for; a leak built
//! another way passes it. The result says so in [`Scan::limits`], and the tool that
//! serves it points at the behavioural checks (`future_perturbation_test`,
//! `bar_boundary_check`) that do not depend on how the code was written.
//!
//! # How it reads source
//!
//! Comments and string literals are blanked before any rule looks at a line, so a leaky
//! snippet in a comment or a docstring is not a finding and a rule is never fooled by a
//! column called `"shift(-1)"`. The one place string content is read on purpose is the
//! label-in-features check, which looks for a label's *name* quoted in a features list.

use std::collections::BTreeSet;

/// How seriously to take a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The column cannot have been known at the decision time. Fix it or do not
    /// trust anything fitted on it.
    Blocking,
    /// Leaks under some uses and not others; a person has to look.
    Flag,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocking => "blocking",
            Self::Flag => "flag",
        }
    }
}

/// One finding: a rule that fired on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    /// 1-based.
    pub line: usize,
    /// The source line, trimmed and cut to 120 characters.
    pub snippet: String,
    pub why: &'static str,
}

/// The result of one scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub findings: Vec<Finding>,
    pub lines_scanned: usize,
    /// Names the scan recognised as labels (built from a forward shift) and so did
    /// not report: computing a label from the future is what a label is.
    pub labels: Vec<String>,
}

impl Scan {
    /// What a result with no findings does **not** establish.
    pub const LIMITS: &'static str =
        "a static pass finds the patterns it has a rule for; no findings is not no leak. A \
         leak built another way passes it, so follow it with future_perturbation_test (does \
         the strategy's behaviour move when a column is shifted) and bar_boundary_check \
         (is every aggregate built from completed bars).";

    #[must_use]
    pub fn blocking(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Blocking)
            .count()
    }
}

/// Blank comments and string contents, keeping line structure and the quotes.
///
/// Tracks triple-quoted strings across lines. Not a Python parser: an unterminated
/// quote ends at end of line, which is the failure direction that hides less.
fn mask(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut triple: Option<char> = None;
    for raw in source.lines() {
        let mut line = String::with_capacity(raw.len());
        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0;
        let mut quote: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if let Some(q) = triple {
                if c == q && chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                    triple = None;
                    line.push_str("   ");
                    i += 3;
                } else {
                    line.push(' ');
                    i += 1;
                }
                continue;
            }
            if let Some(q) = quote {
                if c == '\\' {
                    line.push_str("  ");
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                    line.push(c);
                } else {
                    line.push(' ');
                }
                i += 1;
                continue;
            }
            match c {
                '#' => break,
                '"' | '\'' => {
                    if chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c) {
                        triple = Some(c);
                        line.push_str("   ");
                        i += 3;
                        continue;
                    }
                    quote = Some(c);
                    line.push(c);
                }
                _ => line.push(c),
            }
            i += 1;
        }
        out.push(line);
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The text of the call whose `(` is at `open` in `s`, without the parentheses.
fn call_args(s: &str, open: usize) -> &str {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    for (k, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return &s[open + 1..k];
                }
            }
            _ => {}
        }
    }
    &s[open + 1..]
}

/// All byte offsets of `needle` in `hay`.
fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(p) = hay[from..].find(needle) {
        v.push(from + p);
        from += p + needle.len();
    }
    v
}

/// Whether the first positional or `periods=` argument of a shift-like call is a
/// negative number or a negated name -- a read of a later row.
fn negative_lookahead(args: &str) -> bool {
    let first = args.split(',').next().unwrap_or("").trim();
    let value = first
        .strip_prefix("periods")
        .map(|r| r.trim_start().trim_start_matches('=').trim())
        .unwrap_or(first);
    let mut cs = value.chars();
    cs.next() == Some('-') && cs.next().is_some_and(|c| c.is_ascii_digit() || is_ident(c) || c == ' ')
}

fn label_like(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "y"
        || n.starts_with("y_")
        || n.ends_with("_y")
        || ["label", "target", "fwd", "forward", "future", "ahead", "outcome"]
            .iter()
            .any(|k| n.contains(k))
}

/// `NAME` in `NAME = ...` or `NAME[...] = ...`, for a plain assignment statement.
fn assigned_name(masked: &str) -> Option<String> {
    let eq = masked.find('=')?;
    let after = masked.as_bytes().get(eq + 1).copied();
    let before = masked[..eq].chars().last();
    if after == Some(b'=') || matches!(before, Some('=' | '!' | '<' | '>')) {
        return None;
    }
    let lhs = masked[..eq].trim();
    // `df['fwd'] = ...` names the column; `fwd = ...` names the variable.
    if let (Some(a), Some(b)) = (lhs.find('['), lhs.rfind(']')) {
        let inner: String = lhs[a + 1..b]
            .chars()
            .filter(|c| !matches!(c, '\'' | '"') && !c.is_whitespace())
            .collect();
        // Quotes are kept by `mask` but their contents blanked, so recover the name
        // from the raw line in the caller; here an empty inner means "a string key".
        return if inner.is_empty() { Some(String::new()) } else { Some(inner) };
    }
    let name: String = lhs.chars().take_while(|c| is_ident(*c)).collect();
    (!name.is_empty() && name.len() == lhs.len()).then_some(name)
}

/// The column name inside `df['name'] = ...`, read from the raw (unmasked) line.
fn assigned_column(raw: &str) -> Option<String> {
    let eq = raw.find('=')?;
    let lhs = &raw[..eq];
    let a = lhs.find('[')?;
    let inner = lhs[a + 1..].trim().trim_end_matches(']').trim();
    let name = inner.trim_matches(|c| c == '\'' || c == '"');
    (!name.is_empty() && name.chars().all(is_ident)).then(|| name.to_string())
}

const STATS: [&str; 8] = [
    ".mean(", ".std(", ".var(", ".median(", ".min(", ".max(", ".quantile(", ".sum(",
];
const SCALERS: [&str; 5] = [
    "StandardScaler",
    "MinMaxScaler",
    "RobustScaler",
    "MaxAbsScaler",
    "QuantileTransformer",
];

/// The chain `a.b[c].d(...)` immediately to the left of a method call at `dot`.
fn receiver_before(masked: &str, dot: usize) -> &str {
    let bytes = masked.as_bytes();
    let mut depth = 0i32;
    let mut start = dot;
    while start > 0 {
        let b = bytes[start - 1];
        match b {
            b')' | b']' => depth += 1,
            b'(' | b'[' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ if depth > 0 => {}
            b if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'\'' || b == b'"' => {}
            _ => break,
        }
        start -= 1;
    }
    &masked[start..dot]
}

/// The first identifier of a receiver chain.
fn root(chain: &str) -> &str {
    let end = chain.find(|c: char| !is_ident(c)).unwrap_or(chain.len());
    &chain[..end]
}

fn trimmed(raw: &str) -> String {
    let t = raw.trim();
    if t.chars().count() > 120 {
        let cut: String = t.chars().take(117).collect();
        format!("{cut}...")
    } else {
        t.to_string()
    }
}

/// Scan `source` (Python) for the rules above.
#[must_use]
pub fn scan(source: &str) -> Scan {
    let masked = mask(source);
    let raw: Vec<&str> = source.lines().collect();
    let mut findings: Vec<Finding> = Vec::new();
    let mut labels: BTreeSet<String> = BTreeSet::new();
    // Names derived from a label, propagated through assignments.
    let mut tainted: BTreeSet<String> = BTreeSet::new();

    let mut push = |rule: &'static str, severity: Severity, i: usize, why: &'static str| {
        findings.push(Finding {
            rule,
            severity,
            line: i + 1,
            snippet: trimmed(raw.get(i).copied().unwrap_or("")),
            why,
        });
    };

    for (i, m) in masked.iter().enumerate() {
        let lhs = assigned_name(m);
        let lhs_name = match lhs.as_deref() {
            Some("") => assigned_column(raw.get(i).copied().unwrap_or("")),
            other => other.map(str::to_string),
        };

        // 1. A read of a later row.
        let mut forward = false;
        for call in [".shift(", ".pct_change(", ".diff(", "np.roll("] {
            for p in find_all(m, call) {
                let open = p + call.len() - 1;
                let args = call_args(m, open);
                let negative = if call == "np.roll(" {
                    args.split(',').nth(1).is_some_and(|a| a.trim_start().starts_with('-'))
                } else {
                    negative_lookahead(args)
                };
                if negative {
                    forward = true;
                }
            }
        }
        if forward {
            match &lhs_name {
                Some(n) if label_like(n) => {
                    labels.insert(n.clone());
                    tainted.insert(n.clone());
                }
                _ => {
                    push(
                        "future_shift",
                        Severity::Blocking,
                        i,
                        "a negative shift reads a later row; as a feature it is a value that \
                         did not exist at the decision time. If this is a label, name it like \
                         one (`y`, `label`, `target`, `fwd_ret`) so it is kept out of the features",
                    );
                    if let Some(n) = &lhs_name {
                        tainted.insert(n.clone());
                    }
                }
            }
        }

        // 2. A centred window uses rows after the decision bar.
        if find_all(m, ".rolling(").into_iter().any(|p| {
            let args = call_args(m, p + ".rolling(".len() - 1);
            args.replace(' ', "").contains("center=True")
        }) {
            push(
                "centered_window",
                Severity::Blocking,
                i,
                "`rolling(..., center=True)` averages rows on both sides of the bar, so half of \
                 every value is the future; use the default trailing window",
            );
        }

        // 3. Filling from the next observation.
        let compact = m.replace(' ', "");
        if m.contains(".bfill(")
            || m.contains(".backfill(")
            || compact.contains("method='bfill'")
            || compact.contains("method=\"bfill\"")
            || compact.contains("method='backfill'")
            || raw.get(i).is_some_and(|r| {
                let r = r.replace(' ', "");
                r.contains("method='bfill'") || r.contains("method=\"bfill\"")
            })
        {
            push(
                "backward_fill",
                Severity::Blocking,
                i,
                "a backward fill carries a later observation onto an earlier bar",
            );
        }
        if m.contains(".interpolate(") {
            push(
                "interpolation",
                Severity::Flag,
                i,
                "interpolation fills a gap from the observation after it; fine for display, a \
                 leak in a feature",
            );
        }

        // 4. A full-sample statistic used as a normaliser.
        for s in STATS {
            for p in find_all(m, s) {
                let recv = receiver_before(m, p);
                if recv.is_empty()
                    || recv.contains(".rolling(")
                    || recv.contains(".expanding(")
                    || recv.contains(".ewm(")
                    || recv.ends_with("np")
                    || recv.ends_with("math")
                {
                    continue;
                }
                let before = m[..p - recv.len()].trim_end();
                let operand = before.ends_with('-')
                    || before.ends_with('/')
                    || before.ends_with(".sub(")
                    || before.ends_with(".div(")
                    || before.ends_with(".subtract(")
                    || before.ends_with(".divide(");
                let r = root(recv);
                // `x - x.mean()` and `(x - x.mean()) / x.std()`: the statistic is the
                // operand of a subtraction or division, over the same variable.
                if operand && !r.is_empty() && m.contains(r) {
                    push(
                        "full_window_statistic",
                        Severity::Blocking,
                        i,
                        "a mean/std/min/max over the whole sample used as a normaliser: every \
                         bar is scaled by numbers that include the bars after it. Use \
                         `.rolling(n)` or `.expanding()`",
                    );
                    break;
                }
            }
        }

        // 5. A scaler or ranker fitted on the whole frame.
        if SCALERS.iter().any(|s| m.contains(s)) || m.contains("zscore(") || m.contains(".fit_transform(") {
            for p in find_all(m, ".fit_transform(").into_iter().chain(find_all(m, ".fit(")) {
                let open = m[p..].find('(').map(|k| p + k).unwrap_or(p);
                let a = call_args(m, open).trim_start().to_ascii_lowercase();
                let on_train = a.starts_with("x_train")
                    || a.starts_with("train")
                    || a.starts_with("x_tr")
                    || a.contains("train");
                if !on_train {
                    push(
                        "full_dataset_normalisation",
                        Severity::Blocking,
                        i,
                        "a scaler fitted on data that is not named as the training split learns \
                         its mean and scale from rows it will later be applied to; fit it on the \
                         training window only",
                    );
                    break;
                }
            }
            if m.contains("zscore(") && !m.contains("rolling") {
                push(
                    "full_dataset_normalisation",
                    Severity::Blocking,
                    i,
                    "scipy `zscore` standardises with the whole sample's mean and std",
                );
            }
        }
        if (m.contains(".rank(") && compact.contains("pct=True") && !m.contains(".rolling("))
            || m.contains("qcut(")
        {
            push(
                "full_window_rank",
                Severity::Flag,
                i,
                "a percentile rank or quantile bin over the whole series uses the full \
                 distribution, including the future, to place each bar",
            );
        }

        // 6. Splitting a time series at random.
        if m.contains("train_test_split(") && !compact.contains("shuffle=False") {
            push(
                "shuffled_split",
                Severity::Flag,
                i,
                "`train_test_split` shuffles by default; a shuffled split of a time series puts \
                 the future in the training set",
            );
        }
        if m.contains("KFold(") && compact.contains("shuffle=True") {
            push(
                "shuffled_split",
                Severity::Flag,
                i,
                "a shuffled K-fold on a time series puts the future in the training folds; use a \
                 purged walk-forward split",
            );
        }

        // 7. A column derived from a label, or a label named in a feature list.
        if let Some(n) = &lhs_name {
            if !label_like(n) && !tainted.contains(n) {
                let rhs = m.split_once('=').map_or("", |(_, r)| r);
                let hit = tainted
                    .iter()
                    .find(|t| find_all(rhs, t).into_iter().any(|p| {
                        let b = rhs[..p].chars().last();
                        let a = rhs[p + t.len()..].chars().next();
                        !b.is_some_and(is_ident) && !a.is_some_and(is_ident)
                    }))
                    .cloned();
                if hit.is_some() {
                    push(
                        "label_derived_feature",
                        Severity::Blocking,
                        i,
                        "this column is computed from a value built with a forward shift, so it \
                         carries the future into whatever uses it as a feature",
                    );
                    tainted.insert(n.clone());
                }
            }
        }
        if let Some(r) = raw.get(i) {
            let lower = r.to_ascii_lowercase();
            if (lower.contains("feature") || lower.contains("x =") || lower.contains("x=") || lower.contains("columns"))
                && !labels.is_empty()
            {
                for l in &labels {
                    if r.contains(&format!("'{l}'")) || r.contains(&format!("\"{l}\"")) {
                        push(
                            "label_in_features",
                            Severity::Blocking,
                            i,
                            "a label is named in a features/columns list: the model is being \
                             given the answer",
                        );
                        break;
                    }
                }
            }
        }
    }

    findings.sort_by(|a, b| a.line.cmp(&b.line).then(a.rule.cmp(b.rule)));
    findings.dedup_by(|a, b| a.line == b.line && a.rule == b.rule);
    Scan {
        findings,
        lines_scanned: masked.len(),
        labels: labels.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(src: &str) -> Vec<(&'static str, usize)> {
        scan(src).findings.iter().map(|f| (f.rule, f.line)).collect()
    }

    /// The acceptance clause: a full-dataset normalisation is flagged.
    #[test]
    fn a_full_dataset_normalisation_is_flagged_with_its_line() {
        let src = "import pandas as pd\nz = (close - close.mean()) / close.std()\n";
        assert_eq!(rules(src), vec![("full_window_statistic", 2)]);
        let scaled = "s = StandardScaler()\nX = s.fit_transform(df[cols])\n";
        assert_eq!(rules(scaled), vec![("full_dataset_normalisation", 2)]);
    }

    /// A seeded lookahead is found, and a trailing window is not.
    #[test]
    fn a_future_shift_in_a_feature_is_blocking_and_a_trailing_window_is_clean() {
        let leaky = "feat = close.shift(-1)\n";
        assert_eq!(rules(leaky), vec![("future_shift", 1)]);
        let neg_named = "feat = close.pct_change(-horizon)\n";
        assert_eq!(rules(neg_named), vec![("future_shift", 1)]);
        let clean = "z = (close - close.rolling(50).mean()) / close.rolling(50).std()\nlag = close.shift(1)\n";
        assert!(scan(clean).findings.is_empty(), "{:?}", scan(clean).findings);
    }

    #[test]
    fn a_centred_window_and_a_backward_fill_are_blocking() {
        assert_eq!(rules("m = x.rolling(5, center=True).mean()\n"), vec![("centered_window", 1)]);
        assert_eq!(rules("x = x.bfill()\n"), vec![("backward_fill", 1)]);
        assert_eq!(rules("x = x.fillna(method='bfill')\n"), vec![("backward_fill", 1)]);
        assert_eq!(rules("x = x.interpolate()\n"), vec![("interpolation", 1)]);
    }

    /// A label built from the future is what a label is: not reported, but anything
    /// derived from it, or a feature list naming it, is.
    #[test]
    fn a_label_is_allowed_and_what_is_derived_from_it_is_not() {
        let src = "fwd_ret = close.pct_change(-5)\nsmooth = fwd_ret.rolling(3).mean()\n";
        let s = scan(src);
        assert_eq!(s.labels, vec!["fwd_ret".to_string()]);
        assert_eq!(rules(src), vec![("label_derived_feature", 2)]);
        let listed = "y = close.shift(-1)\nfeatures = ['rsi', 'y']\n";
        assert_eq!(rules(listed), vec![("label_in_features", 2)]);
        let fine = "y = close.shift(-1)\nfeatures = ['rsi', 'ema']\n";
        assert!(scan(fine).findings.is_empty());
    }

    #[test]
    fn a_scaler_fitted_on_the_training_split_is_not_flagged() {
        let src = "s = StandardScaler()\nXt = s.fit_transform(X_train)\nXv = s.transform(X_test)\n";
        assert!(scan(src).findings.is_empty(), "{:?}", scan(src).findings);
    }

    #[test]
    fn a_shuffled_split_is_flagged_and_an_ordered_one_is_not() {
        assert_eq!(rules("a = train_test_split(X, y)\n"), vec![("shuffled_split", 1)]);
        assert!(scan("a = train_test_split(X, y, shuffle=False)\n").findings.is_empty());
        assert_eq!(rules("k = KFold(5, shuffle=True)\n"), vec![("shuffled_split", 1)]);
    }

    /// Comments and strings are blanked: a leak that is only described is not a leak.
    #[test]
    fn leaky_text_in_a_comment_or_a_string_is_not_a_finding() {
        let src = "# z = (x - x.mean()) / x.std()\nnote = \"close.shift(-1)\"\n\"\"\"\nm = x.rolling(5, center=True)\n\"\"\"\nok = close.shift(1)\n";
        assert!(scan(src).findings.is_empty(), "{:?}", scan(src).findings);
    }

    #[test]
    fn findings_carry_their_severity_and_the_limits_are_stated() {
        let s = scan("a = x.bfill()\nb = x.interpolate()\n");
        assert_eq!(s.blocking(), 1);
        assert_eq!(s.findings[1].severity, Severity::Flag);
        assert!(Scan::LIMITS.contains("no findings is not no leak"));
    }
}
