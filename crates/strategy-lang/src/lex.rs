//! Tokens, and the one place a byte offset becomes a line and a column.
//!
//! # Newlines are tokens
//!
//! AGENT-011 §3's first amendment is "every clause on its own line, keywords
//! first", and the reason is a small model: a clause that can wrap is a clause
//! that can be half-written, and a grammar-constrained tier fills one clause per
//! completion. So the lexer keeps newlines instead of skipping them, the parser
//! recovers at them, and a syntax error in one clause never swallows the next —
//! which is what lets [`crate::parse()`] return every clause's error at once
//! rather than the first.
//!
//! # `1h` is one token
//!
//! A timeframe and a distance unit sit next to numbers, and `1h` lexed as
//! `1` then `h` produces a parse error about `h` that says nothing useful. A
//! digit followed immediately by a letter is therefore a [`Tok::Word`], so the
//! parser can accept `1h` as a timeframe and can tell an author who wrote
//! `1.5atr` to put a space in — a specific instruction instead of a generic
//! surprise.

use crate::diagnostic::{Code, Diagnostic, Expected, Span};

/// One lexical token.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// `ema`, `entry`, `$bound_at_init` — an identifier or keyword. Keywords
    /// are not reserved at the lexical level; the parser decides by position,
    /// so a feature family called `time` stays usable.
    Ident(String),
    /// A decimal literal, kept as written. Never parsed to `f64` here: the
    /// canonical form stores sizes and risk limits as decimal strings
    /// (ADR-0002), and a detour through binary floating point would change
    /// `0.1`.
    Num(String),
    /// A number followed immediately by letters: `1h`, `15m`, `1.5atr`.
    Word(String),
    /// A `'…'` literal, as in `feature('ema_7')`.
    Str(String),
    Percent,
    Comma,
    Colon,
    LParen,
    RParen,
    LBracket,
    RBracket,
    DotDot,
    Assign,
    Gt,
    Lt,
    Ge,
    Le,
    EqEq,
    Ne,
    Plus,
    Minus,
    Star,
    Slash,
    /// End of a clause.
    Newline,
    /// End of the source. Always the last token, so the parser never indexes
    /// past the end.
    Eof,
}

impl Tok {
    /// How the token is named in a diagnostic.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Ident(s) => format!("`{s}`"),
            Self::Num(s) => format!("the number `{s}`"),
            Self::Word(s) => format!("`{s}`"),
            Self::Str(s) => format!("the text `'{s}'`"),
            Self::Percent => "`%`".into(),
            Self::Comma => "`,`".into(),
            Self::Colon => "`:`".into(),
            Self::LParen => "`(`".into(),
            Self::RParen => "`)`".into(),
            Self::LBracket => "`[`".into(),
            Self::RBracket => "`]`".into(),
            Self::DotDot => "`..`".into(),
            Self::Assign => "`=`".into(),
            Self::Gt => "`>`".into(),
            Self::Lt => "`<`".into(),
            Self::Ge => "`>=`".into(),
            Self::Le => "`<=`".into(),
            Self::EqEq => "`==`".into(),
            Self::Ne => "`!=`".into(),
            Self::Plus => "`+`".into(),
            Self::Minus => "`-`".into(),
            Self::Star => "`*`".into(),
            Self::Slash => "`/`".into(),
            Self::Newline => "the end of the line".into(),
            Self::Eof => "the end of the strategy".into(),
        }
    }
}

/// A token and where it was written.
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned {
    pub tok: Tok,
    pub span: Span,
}

/// A `# …` comment and the line it was written on.
///
/// Comments are part of the language (AGENT-011 §3, S-5.15): the parser attaches
/// each to the clause that follows it, so intent survives being read back. They
/// are kept out of [`Tok`] because a comment is never a syntactic element — no
/// production mentions one — and putting it in the token stream would mean every
/// parser rule having to skip it.
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    /// The text after `#`, trimmed.
    pub text: String,
    pub line: u32,
    /// True when tokens preceded it on the same line, i.e. it trails a clause
    /// rather than introducing one.
    pub trailing: bool,
}

/// The result of lexing: the tokens, and the comments that sat between them.
#[derive(Clone, Debug, PartialEq)]
pub struct Lexed {
    pub tokens: Vec<Spanned>,
    pub comments: Vec<Comment>,
}

/// Turn source into tokens, or say where it stopped making sense.
///
/// Returns every lexical error rather than the first: a model that wrote two
/// stray characters should be told about both.
pub fn lex(source: &str) -> Result<Lexed, Vec<Diagnostic>> {
    let bytes = source.as_bytes();
    let mut tokens: Vec<Spanned> = Vec::new();
    let mut comments: Vec<Comment> = Vec::new();
    let mut errors: Vec<Diagnostic> = Vec::new();

    let mut i = 0usize;
    let mut line = 1u32;
    let mut line_start = 0usize;
    // Whether this line has produced a token yet, which is what distinguishes a
    // comment that introduces a clause from one that trails it.
    let mut line_has_token = false;

    let span_at = |i: usize, len: usize, line: u32, line_start: usize| -> Span {
        let col = source[line_start..i].chars().count() as u32 + 1;
        Span::new(i as u32, len as u32, line, col)
    };

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                if line_has_token {
                    tokens.push(Spanned {
                        tok: Tok::Newline,
                        span: span_at(i, 1, line, line_start),
                    });
                }
                i += 1;
                line += 1;
                line_start = i;
                line_has_token = false;
            }
            b'#' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'\n' {
                    end += 1;
                }
                comments.push(Comment {
                    text: source[start..end].trim().to_string(),
                    line,
                    trailing: line_has_token,
                });
                i = end;
            }
            b'0'..=b'9' => {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                // A single `.` followed by a digit continues the number; `..`
                // is the range operator and ends it.
                if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                // `1h`, `1.5atr`: letters glued to a number.
                if i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    tokens.push(Spanned {
                        tok: Tok::Word(source[start..i].to_string()),
                        span: span_at(start, i - start, line, line_start),
                    });
                } else {
                    tokens.push(Spanned {
                        tok: Tok::Num(source[start..i].to_string()),
                        span: span_at(start, i - start, line, line_start),
                    });
                }
                line_has_token = true;
            }
            b'.' if i + 1 < bytes.len() && bytes[i + 1] == b'.' => {
                tokens.push(Spanned {
                    tok: Tok::DotDot,
                    span: span_at(i, 2, line, line_start),
                });
                i += 2;
                line_has_token = true;
            }
            b'\'' => {
                let start = i;
                i += 1;
                let content_start = i;
                while i < bytes.len() && bytes[i] != b'\'' && bytes[i] != b'\n' {
                    i += 1;
                }
                if i >= bytes.len() || bytes[i] == b'\n' {
                    errors.push(
                        Diagnostic::syntax(
                            Code::UnterminatedString,
                            span_at(start, i - start, line, line_start),
                            "this text literal has no closing quote",
                            Expected::one("a closing `'` on the same line"),
                        )
                        .with_fix("close the quote, as in `feature('ema_7')`"),
                    );
                    line_has_token = true;
                    continue;
                }
                tokens.push(Spanned {
                    tok: Tok::Str(source[content_start..i].to_string()),
                    span: span_at(start, i + 1 - start, line, line_start),
                });
                i += 1;
                line_has_token = true;
            }
            _ if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
                {
                    i += 1;
                }
                tokens.push(Spanned {
                    tok: Tok::Ident(source[start..i].to_string()),
                    span: span_at(start, i - start, line, line_start),
                });
                line_has_token = true;
            }
            _ => {
                let (tok, len) = match (c, bytes.get(i + 1)) {
                    (b'>', Some(b'=')) => (Tok::Ge, 2),
                    (b'<', Some(b'=')) => (Tok::Le, 2),
                    (b'=', Some(b'=')) => (Tok::EqEq, 2),
                    (b'!', Some(b'=')) => (Tok::Ne, 2),
                    (b'>', _) => (Tok::Gt, 1),
                    (b'<', _) => (Tok::Lt, 1),
                    (b'=', _) => (Tok::Assign, 1),
                    (b'%', _) => (Tok::Percent, 1),
                    (b',', _) => (Tok::Comma, 1),
                    (b':', _) => (Tok::Colon, 1),
                    (b'(', _) => (Tok::LParen, 1),
                    (b')', _) => (Tok::RParen, 1),
                    (b'[', _) => (Tok::LBracket, 1),
                    (b']', _) => (Tok::RBracket, 1),
                    (b'+', _) => (Tok::Plus, 1),
                    (b'-', _) => (Tok::Minus, 1),
                    (b'*', _) => (Tok::Star, 1),
                    (b'/', _) => (Tok::Slash, 1),
                    _ => {
                        let ch = source[i..].chars().next().unwrap_or('?');
                        errors.push(
                            Diagnostic::syntax(
                                Code::UnexpectedCharacter,
                                span_at(i, ch.len_utf8(), line, line_start),
                                format!("`{ch}` is not part of SLv2"),
                                Expected::one("a clause keyword, a name, a number or an operator"),
                            )
                            .with_see("guide#grammar"),
                        );
                        i += ch.len_utf8();
                        line_has_token = true;
                        continue;
                    }
                };
                tokens.push(Spanned {
                    tok,
                    span: span_at(i, len, line, line_start),
                });
                i += len;
                line_has_token = true;
            }
        }
    }

    if line_has_token {
        tokens.push(Spanned {
            tok: Tok::Newline,
            span: span_at(i, 0, line, line_start),
        });
    }
    tokens.push(Spanned {
        tok: Tok::Eof,
        span: span_at(i, 0, line, line_start),
    });

    if errors.is_empty() {
        Ok(Lexed { tokens, comments })
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        lex(src)
            .expect("lexes")
            .tokens
            .into_iter()
            .map(|s| s.tok)
            .collect()
    }

    #[test]
    fn a_clause_ends_with_a_newline_token_even_without_a_trailing_line_break() {
        assert_eq!(
            toks("size fixed 0.01"),
            vec![
                Tok::Ident("size".into()),
                Tok::Ident("fixed".into()),
                Tok::Num("0.01".into()),
                Tok::Newline,
                Tok::Eof,
            ]
        );
    }

    /// Blank lines and comment-only lines produce no `Newline`, so the parser
    /// never sees an empty clause it would have to special-case.
    #[test]
    fn blank_and_comment_only_lines_emit_no_clause_terminator() {
        let lexed = lex("# why\n\n  \nsize fixed 1\n").expect("lexes");
        let kinds: Vec<Tok> = lexed.tokens.iter().map(|s| s.tok.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Tok::Ident("size".into()),
                Tok::Ident("fixed".into()),
                Tok::Num("1".into()),
                Tok::Newline,
                Tok::Eof,
            ]
        );
        assert_eq!(lexed.comments.len(), 1);
        assert_eq!(lexed.comments[0].text, "why");
        assert!(!lexed.comments[0].trailing);
    }

    #[test]
    fn a_comment_after_a_clause_is_marked_trailing() {
        let lexed = lex("size fixed 1  # a whole bitcoin\n").expect("lexes");
        assert_eq!(lexed.comments.len(), 1);
        assert!(lexed.comments[0].trailing);
        assert_eq!(lexed.comments[0].text, "a whole bitcoin");
    }

    /// The reason `Word` exists: `1h` must survive as one thing, and `..` must
    /// not be eaten by the decimal-point rule.
    #[test]
    fn digits_glued_to_letters_are_one_word_and_a_range_is_not_a_decimal() {
        assert_eq!(
            toks("timeframe 1h"),
            vec![
                Tok::Ident("timeframe".into()),
                Tok::Word("1h".into()),
                Tok::Newline,
                Tok::Eof,
            ]
        );
        assert_eq!(
            toks("param n: int[5..50] = 7"),
            vec![
                Tok::Ident("param".into()),
                Tok::Ident("n".into()),
                Tok::Colon,
                Tok::Ident("int".into()),
                Tok::LBracket,
                Tok::Num("5".into()),
                Tok::DotDot,
                Tok::Num("50".into()),
                Tok::RBracket,
                Tok::Assign,
                Tok::Num("7".into()),
                Tok::Newline,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn a_decimal_keeps_its_written_form() {
        assert_eq!(toks("risk max_position 0.50")[2], Tok::Num("0.50".into()));
    }

    #[test]
    fn an_unterminated_literal_names_the_missing_quote() {
        let errs = lex("input a = feature('ema_7\n").expect_err("refused");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].code, Code::UnterminatedString);
        assert!(
            errs[0].expected().is_some(),
            "a syntax error says what belonged"
        );
    }

    #[test]
    fn a_stray_character_is_named_and_lexing_continues_past_it() {
        let errs = lex("entry long when a @ b\nsize fixed ~1\n").expect_err("refused");
        assert_eq!(
            errs.len(),
            2,
            "both stray characters reported, not just the first"
        );
        assert!(errs.iter().all(|e| e.code == Code::UnexpectedCharacter));
        assert_eq!((errs[0].span.line, errs[0].span.col), (1, 19));
        assert_eq!(errs[1].span.line, 2);
    }

    /// Columns count characters, not bytes, because a caret placed by byte
    /// offset lands in the wrong place the first time someone writes a comment
    /// in a language with accents.
    #[test]
    fn columns_count_characters_not_bytes() {
        let lexed = lex("# café\nsize fixed 1\n").expect("lexes");
        let size = &lexed.tokens[0];
        assert_eq!((size.span.line, size.span.col), (2, 1));
        let one = &lexed.tokens[2];
        assert_eq!(one.span.col, 12);
    }
}
