//! What a refusal says.
//!
//! # Why a diagnostic and not an error enum
//!
//! The reader of every failure in this crate is a language model deciding what
//! to write next. `Err(())` tells it that something is wrong; a message with no
//! position tells it that something is wrong *somewhere*; and both cost a round
//! trip that the model pays for out of the same budget as its research. So the
//! type carries the four things that turn a rejection into an edit: **where**
//! ([`Span`]), **what was expected** there, **a concrete fix**, and a stable
//! **code** the guide can be anchored to. AGENT-011 §5 calls this the validator
//! teaching rather than refusing, and P4 makes it a principle.
//!
//! # The `expected` list is not optional for a syntax error
//!
//! [`Kind::Syntax`] carries an [`Expected`], which cannot be empty by
//! construction. That is deliberate and it is the reason `Expected` is a newtype
//! over a non-empty pair rather than a `Vec<String>`: a parse error that forgot
//! to say what would have been accepted is exactly the useless error this crate
//! exists to stop producing, and a comment asking authors to remember would be
//! forgotten the first busy afternoon. A semantic refusal ([`Kind::Semantic`])
//! has no "expected" — the text was well-formed and means something the target
//! cannot hold — so the two are different variants rather than one struct with a
//! nullable field.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A byte range in the source, with the line and column a human reads.
///
/// `line` and `col` are 1-based because every editor and every model that has
/// ever seen a compiler error expects them to be. `offset` and `len` are bytes,
/// so `&source[span.range()]` is the text the diagnostic is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub offset: u32,
    pub len: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    #[must_use]
    pub const fn new(offset: u32, len: u32, line: u32, col: u32) -> Self {
        Self {
            offset,
            len,
            line,
            col,
        }
    }

    /// The span covering `self` and `other`, which must come from the same
    /// source. Used to point a diagnostic at a whole expression rather than at
    /// the one token that happened to be under the cursor.
    #[must_use]
    pub fn to(self, other: Self) -> Self {
        let start = self.offset.min(other.offset);
        let end = (self.offset + self.len).max(other.offset + other.len);
        // The line/col of whichever span starts first: a merged span points at
        // its beginning, because that is where a reader starts reading.
        let (line, col) = if self.offset <= other.offset {
            (self.line, self.col)
        } else {
            (other.line, other.col)
        };
        Self {
            offset: start,
            len: end - start,
            line,
            col,
        }
    }

    /// The byte range, for slicing the source the span came from.
    #[must_use]
    pub fn range(self) -> std::ops::Range<usize> {
        self.offset as usize..(self.offset + self.len) as usize
    }
}

/// What the parser would have accepted at a [`Span`]. Never empty.
///
/// The non-emptiness is structural — `first` is a field, not the head of a
/// vector — so there is no constructor that can produce a syntax error saying
/// nothing about what belonged there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "Vec<String>", try_from = "Vec<String>")]
pub struct Expected {
    first: String,
    rest: Vec<String>,
}

impl Expected {
    #[must_use]
    pub fn one(what: impl Into<String>) -> Self {
        Self {
            first: what.into(),
            rest: Vec::new(),
        }
    }

    #[must_use]
    pub fn any<I, S>(items: I) -> Option<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut it = items.into_iter().map(Into::into);
        let first = it.next()?;
        Some(Self {
            first,
            rest: it.collect(),
        })
    }

    /// Every alternative, in the order the author listed them.
    pub fn items(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.first.as_str()).chain(self.rest.iter().map(String::as_str))
    }

    /// `a`, `a or b`, `a, b or c` — English, because the reader is reading it.
    #[must_use]
    pub fn joined(&self) -> String {
        let all: Vec<&str> = self.items().collect();
        match all.len() {
            1 => all[0].to_string(),
            2 => format!("{} or {}", all[0], all[1]),
            _ => {
                let (last, head) = all.split_last().expect("len >= 3");
                format!("{} or {last}", head.join(", "))
            }
        }
    }
}

impl From<Expected> for Vec<String> {
    fn from(e: Expected) -> Self {
        e.items().map(str::to_string).collect()
    }
}

impl TryFrom<Vec<String>> for Expected {
    type Error = &'static str;

    fn try_from(v: Vec<String>) -> Result<Self, Self::Error> {
        Self::any(v).ok_or("an `expected` list must name at least one alternative")
    }
}

/// Whether the text was ill-formed or well-formed and unrepresentable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    /// The parser reached a token it could not use. Carries what would have
    /// been accepted there.
    Syntax { expected: Expected },
    /// The text parsed. What it means has no home in the canonical
    /// `StrategyDefinition`, or contradicts something else the author wrote.
    Semantic,
}

/// A stable identifier for a class of refusal.
///
/// A closed enum rather than a free string, for the reason the repository keeps
/// relearning: a free-form string is a value nothing checks, so the guide anchor
/// and the error drift apart silently. `strategy-validator`'s `teach.rs` maps
/// codes to worked fixes, and [`Code::as_str`] is the join between the two
/// tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    // ── lexical and syntactic ────────────────────────────────────────────────
    /// A character that is not part of SLv2 at all.
    UnexpectedCharacter,
    /// A `'…'` literal with no closing quote.
    UnterminatedString,
    /// A token where a different one belonged.
    UnexpectedToken,
    /// A line whose first word is not a clause keyword.
    UnknownClause,
    /// The `strategy …` header is missing or is not the first clause.
    MissingHeader,
    /// A distance (`exit stop 1.5`) with no unit.
    DistanceWithoutUnit,
    /// A number and its unit written without a space (`1.5atr`).
    UnitNeedsSpace,

    // ── the AGENT-011 §3 legibility rules ────────────────────────────────────
    /// An inline expression longer than [`crate::parse::MAX_INLINE_EXPR`].
    ExpressionTooLong,
    /// The same call expression written in more than one place.
    ExpressionUsedTwice,
    /// Two declarations competing for one name.
    DuplicateName,
    /// A name that is neither a declared input, a declared parameter, a bar
    /// field, nor a known feature family.
    UnknownName,
    /// A feature family nothing in the catalogue knows.
    UnknownFeature,
    /// A known family called with the wrong number of parameters.
    WrongArity,
    /// A feature parameter outside the family's declared range, or not an
    /// integer.
    BadFeatureParam,

    // ── well-formed, and the canonical form cannot hold it ───────────────────
    /// A value used where a condition belonged, or the reverse.
    TypeMismatch,
    /// `crossed_above`, `rising`, … — needs a bar the expression grammar
    /// cannot see.
    NeedsHistory,
    /// `flat`, `bars_in_trade`, … — needs position state the grammar cannot
    /// see.
    NeedsPositionState,
    /// `abs`, `min`, `clip`, … — needs a bytecode operation that does not
    /// exist.
    NeedsOperator,
    /// An exit distance in a unit the canonical `ExitRule` cannot store.
    UnitNotRepresentable,
    /// A sizing rule with no `SizeMode`.
    SizeModeNotRepresentable,
    /// A risk limit with no field on `RiskOverrides`.
    RiskItemNotRepresentable,
    /// `cooldown`, `hysteresis`, `order …` — parsed, and nothing downstream
    /// can carry it.
    ClauseNotRepresentable,
    /// An `input` bound to something that is not one feature.
    InputNotAFeature,
    /// An `input` nothing references. Dead text, and the decompiler has no way
    /// to write it back, so it is refused rather than silently dropped.
    UnusedInput,
    /// A strategy with entries and no `size` clause.
    MissingSize,
    /// A 1.3-or-later strategy with no `exit` clause, not even `exit none`.
    MissingExits,
    /// A parameter declaration whose default is outside its own range, or
    /// whose range is inverted.
    BadParamSpec,

    // ── decompilation ────────────────────────────────────────────────────────
    /// A stored definition whose shape SLv2 has no text for.
    NotExpressible,
    /// A stored definition that declares features it does not use, or uses
    /// features it does not declare.
    NotCanonical,
    /// A stored expression that is not in the v1 grammar.
    BadStoredExpression,

    // ── the blocking profile ─────────────────────────────────────────────────
    /// Per-clause counts no single walk over one window could have produced,
    /// or counts of a different strategy than the one they were handed with.
    /// See [`crate::block::EntryCounts::new`].
    ImpossibleTally,

    // ── the clause grammar for constrained tiers ───────────────────────────
    /// Well-formed SLv2 that [`crate::form`] has no fill for. The refusal
    /// carries a [`crate::form::Limitation`] saying which narrowing it hit and
    /// whether writing it as text would help.
    NotInTheForm,
}

impl Code {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "unexpected_character",
            Self::UnterminatedString => "unterminated_string",
            Self::UnexpectedToken => "unexpected_token",
            Self::UnknownClause => "unknown_clause",
            Self::MissingHeader => "missing_header",
            Self::DistanceWithoutUnit => "distance_without_unit",
            Self::UnitNeedsSpace => "unit_needs_space",
            Self::ExpressionTooLong => "expression_too_long",
            Self::ExpressionUsedTwice => "expression_used_twice",
            Self::DuplicateName => "duplicate_name",
            Self::UnknownName => "unknown_name",
            Self::UnknownFeature => "unknown_feature",
            Self::WrongArity => "wrong_arity",
            Self::BadFeatureParam => "bad_feature_param",
            Self::TypeMismatch => "type_mismatch",
            Self::NeedsHistory => "needs_history",
            Self::NeedsPositionState => "needs_position_state",
            Self::NeedsOperator => "needs_operator",
            Self::UnitNotRepresentable => "unit_not_representable",
            Self::SizeModeNotRepresentable => "size_mode_not_representable",
            Self::RiskItemNotRepresentable => "risk_item_not_representable",
            Self::ClauseNotRepresentable => "clause_not_representable",
            Self::InputNotAFeature => "input_not_a_feature",
            Self::UnusedInput => "unused_input",
            Self::MissingSize => "missing_size",
            Self::MissingExits => "missing_exits",
            Self::BadParamSpec => "bad_param_spec",
            Self::NotExpressible => "not_expressible",
            Self::NotCanonical => "not_canonical",
            Self::BadStoredExpression => "bad_stored_expression",
            Self::ImpossibleTally => "impossible_tally",
            Self::NotInTheForm => "not_in_the_form",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One refusal, positioned and, where the crate knows one, carrying the edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{line}:{col}: {code}: {message}", line = self.span.line, col = self.span.col)]
pub struct Diagnostic {
    pub code: Code,
    pub span: Span,
    pub message: String,
    #[serde(flatten)]
    pub kind: Kind,
    /// A concrete edit, in the imperative. AGENT-011 §5's `fix`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// A guide anchor, e.g. `guide#exits.units`. AGENT-011 §5's `see`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub see: Option<String>,
}

impl Diagnostic {
    /// A refusal of ill-formed text. `expected` is required by the signature,
    /// not by a convention.
    #[must_use]
    pub fn syntax(code: Code, span: Span, message: impl Into<String>, expected: Expected) -> Self {
        Self {
            code,
            span,
            message: message.into(),
            kind: Kind::Syntax { expected },
            fix: None,
            see: None,
        }
    }

    /// A refusal of well-formed text that means something unrepresentable.
    #[must_use]
    pub fn semantic(code: Code, span: Span, message: impl Into<String>) -> Self {
        Self {
            code,
            span,
            message: message.into(),
            kind: Kind::Semantic,
            fix: None,
            see: None,
        }
    }

    #[must_use]
    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    #[must_use]
    pub fn with_see(mut self, see: impl Into<String>) -> Self {
        self.see = Some(see.into());
        self
    }

    /// What would have been accepted, for a syntax error.
    #[must_use]
    pub fn expected(&self) -> Option<&Expected> {
        match &self.kind {
            Kind::Syntax { expected } => Some(expected),
            Kind::Semantic => None,
        }
    }

    /// The source text this diagnostic points at, for rendering a caret line.
    /// Returns `None` when the span does not lie on a character boundary of
    /// `source`, which means the diagnostic did not come from this text.
    #[must_use]
    pub fn snippet<'a>(&self, source: &'a str) -> Option<&'a str> {
        source.get(self.span.range())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expected_list_cannot_be_empty() {
        assert!(Expected::any(Vec::<String>::new()).is_none());
        assert!(Expected::try_from(Vec::<String>::new()).is_err());
        assert_eq!(Expected::one("`entry`").items().count(), 1);
    }

    #[test]
    fn expected_reads_as_english() {
        assert_eq!(Expected::one("`%`").joined(), "`%`");
        assert_eq!(
            Expected::any(["`%`", "`atr`"]).unwrap().joined(),
            "`%` or `atr`"
        );
        assert_eq!(
            Expected::any(["`%`", "`atr`", "`sigma`"]).unwrap().joined(),
            "`%`, `atr` or `sigma`"
        );
    }

    /// The `expected` list survives JSON, because the MCP tool boundary is JSON
    /// and a list that arrived empty would defeat the type's whole guarantee.
    #[test]
    fn expected_round_trips_through_json_and_refuses_an_empty_list() {
        let e = Expected::any(["`%`", "`atr`", "`sigma`"]).unwrap();
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"["`%`","`atr`","`sigma`"]"#);
        assert_eq!(serde_json::from_str::<Expected>(&json).unwrap(), e);
        assert!(serde_json::from_str::<Expected>("[]").is_err());
    }

    /// Every code renders as a distinct snake_case string. A collision would
    /// silently merge two error classes in `teach.rs`.
    #[test]
    fn every_code_has_a_distinct_snake_case_string() {
        // The list is written out rather than derived, so that adding a variant
        // without adding it here fails the exhaustiveness check in `as_str`
        // *and* leaves this guard honest about how many it saw.
        let all = [
            Code::UnexpectedCharacter,
            Code::UnterminatedString,
            Code::UnexpectedToken,
            Code::UnknownClause,
            Code::MissingHeader,
            Code::DistanceWithoutUnit,
            Code::UnitNeedsSpace,
            Code::ExpressionTooLong,
            Code::ExpressionUsedTwice,
            Code::DuplicateName,
            Code::UnknownName,
            Code::UnknownFeature,
            Code::WrongArity,
            Code::BadFeatureParam,
            Code::TypeMismatch,
            Code::NeedsHistory,
            Code::NeedsPositionState,
            Code::NeedsOperator,
            Code::UnitNotRepresentable,
            Code::SizeModeNotRepresentable,
            Code::RiskItemNotRepresentable,
            Code::ClauseNotRepresentable,
            Code::InputNotAFeature,
            Code::UnusedInput,
            Code::MissingSize,
            Code::MissingExits,
            Code::BadParamSpec,
            Code::NotExpressible,
            Code::NotCanonical,
            Code::BadStoredExpression,
            Code::ImpossibleTally,
            Code::NotInTheForm,
        ];
        assert!(all.len() >= 25, "the scan found {} codes", all.len());
        let mut seen: Vec<&str> = all.iter().map(|c| c.as_str()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "two codes share a string");
        for s in seen {
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{s} is not snake_case"
            );
            // A code is the join with `teach.rs`, which round-trips it as JSON.
            let json = serde_json::to_string(&Code::UnknownFeature).unwrap();
            assert_eq!(json, "\"unknown_feature\"");
        }
    }

    #[test]
    fn a_span_merges_to_cover_both_ends_and_points_at_the_first() {
        let a = Span::new(10, 3, 2, 5);
        let b = Span::new(20, 4, 2, 15);
        let merged = a.to(b);
        assert_eq!(merged.offset, 10);
        assert_eq!(merged.len, 14);
        assert_eq!((merged.line, merged.col), (2, 5));
        assert_eq!(b.to(a), merged, "merging is symmetric");
    }
}
