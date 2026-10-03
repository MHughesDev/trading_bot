//! SLv3 tokens.
//!
//! Whitespace and newlines separate tokens and mean nothing else: every
//! statement starts with a keyword, so a strategy can be laid out however reads
//! best and still be the same graph. `#` starts a comment that runs to the end of
//! the line.

/// One token and where it starts (1-based line and column).
#[derive(Clone, Debug, PartialEq)]
pub struct Tok {
    pub kind: TokKind,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokKind {
    /// Letters, digits and `_`, not starting with a digit; or `$name`.
    Ident(String),
    /// A number as written, e.g. `12`, `0.03`, `1e-4`.
    Number(String),
    /// A word that starts with a digit and runs into letters: `1h`, `15m`.
    Word(String),
    /// A double-quoted string, escapes resolved.
    Str(String),
    /// `( ) [ ] { } , : = . + - * / < > <= >= == != ->`
    Sym(&'static str),
    Eof,
}

/// Where lexing failed.
#[derive(Clone, Debug, PartialEq)]
pub struct LexError {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

const SYMS: [&str; 20] = [
    "->", "<=", ">=", "==", "!=", "(", ")", "[", "]", "{", "}", ",", ":", "=", ".", "+", "-", "*",
    "/", "<",
];

/// Split `src` into tokens, ending with [`TokKind::Eof`].
///
/// # Errors
/// An unterminated string, or a character that starts no token.
pub fn lex(src: &str) -> Result<Vec<Tok>, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    let advance = |i: &mut usize, line: &mut u32, col: &mut u32, c: char| {
        *i += 1;
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let (tl, tc) = (line, col);
        if c.is_whitespace() {
            advance(&mut i, &mut line, &mut col, c);
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                let ch = chars[i];
                advance(&mut i, &mut line, &mut col, ch);
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            let mut s = String::new();
            s.push(c);
            advance(&mut i, &mut line, &mut col, c);
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                s.push(chars[i]);
                let ch = chars[i];
                advance(&mut i, &mut line, &mut col, ch);
            }
            out.push(Tok {
                kind: TokKind::Ident(s),
                line: tl,
                col: tc,
            });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            let digits = |i: &mut usize| {
                while *i < chars.len() && chars[*i].is_ascii_digit() {
                    *i += 1;
                }
            };
            digits(&mut i);
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                i += 1;
                digits(&mut i);
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let signed = i + 2 < chars.len()
                    && (chars[i + 1] == '-' || chars[i + 1] == '+')
                    && chars[i + 2].is_ascii_digit();
                let plain = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
                if signed || plain {
                    i += if signed { 2 } else { 1 };
                    digits(&mut i);
                }
            }
            let mut word = false;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                word = true;
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            col += u32::try_from(i - start).unwrap_or(u32::MAX);
            out.push(Tok {
                kind: if word {
                    TokKind::Word(s)
                } else {
                    TokKind::Number(s)
                },
                line: tl,
                col: tc,
            });
            continue;
        }
        if c == '"' {
            advance(&mut i, &mut line, &mut col, c);
            let mut s = String::new();
            let mut closed = false;
            while i < chars.len() {
                let d = chars[i];
                advance(&mut i, &mut line, &mut col, d);
                match d {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' if i < chars.len() => {
                        let e = chars[i];
                        advance(&mut i, &mut line, &mut col, e);
                        s.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    }
                    other => s.push(other),
                }
            }
            if !closed {
                return Err(LexError {
                    line: tl,
                    col: tc,
                    message: "this string is never closed: add the ending `\"`".into(),
                });
            }
            out.push(Tok {
                kind: TokKind::Str(s),
                line: tl,
                col: tc,
            });
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let sym = SYMS
            .iter()
            .find(|s| rest.starts_with(**s))
            .copied()
            .or_else(|| (c == '>').then_some(">"));
        let Some(sym) = sym else {
            return Err(LexError {
                line: tl,
                col: tc,
                message: format!("`{c}` does not start anything SLv3 knows"),
            });
        };
        for ch in sym.chars() {
            advance(&mut i, &mut line, &mut col, ch);
        }
        out.push(Tok {
            kind: TokKind::Sym(sym),
            line: tl,
            col: tc,
        });
    }
    out.push(Tok {
        kind: TokKind::Eof,
        line,
        col,
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<TokKind> {
        lex(s).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers_words_and_symbols() {
        assert_eq!(
            kinds("let f = ema(close, $fast) # comment\n1h 0.03 1e-4 -> <= >"),
            vec![
                TokKind::Ident("let".into()),
                TokKind::Ident("f".into()),
                TokKind::Sym("="),
                TokKind::Ident("ema".into()),
                TokKind::Sym("("),
                TokKind::Ident("close".into()),
                TokKind::Sym(","),
                TokKind::Ident("$fast".into()),
                TokKind::Sym(")"),
                TokKind::Word("1h".into()),
                TokKind::Number("0.03".into()),
                TokKind::Number("1e-4".into()),
                TokKind::Sym("->"),
                TokKind::Sym("<="),
                TokKind::Sym(">"),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn strings_escape_and_must_close() {
        assert_eq!(kinds(r#""a\"b""#)[0], TokKind::Str("a\"b".into()));
        assert!(lex("\"open").unwrap_err().message.contains("never closed"));
    }
}
