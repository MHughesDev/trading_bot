//! Syntax validator for v1.0 condition expressions.
//!
//! Validates that an expression string parses correctly against the frozen grammar
//! defined in `domain::strategy_def::nodes`. Does not evaluate — no feature lookup,
//! no I/O. Returns structured errors the caller can act on.

use crate::ValidationError;

/// Validate the syntax of one condition expression.
pub fn validate_expression(expr: &str, path: &str) -> Vec<ValidationError> {
    match check_expr(expr) {
        Ok(()) => vec![],
        Err(msg) => vec![ValidationError {
            path: path.to_owned(),
            message: msg,
        }],
    }
}

// ── Tokenizer ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num,
    Ident(String),
    Str(String),
    Plus,
    Minus,
    Star,
    Slash,
    Gt,
    Lt,
    GtEq,
    LtEq,
    EqEq,
    BangEq,
    LParen,
    RParen,
    Comma,
    Eof,
}

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            '0'..='9' | '.' => {
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                tokens.push(Tok::Num);
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Tok::Ident(ident));
            }
            '\'' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("unterminated string literal".into());
                }
                let s: String = chars[start..i].iter().collect();
                tokens.push(Tok::Str(s));
                i += 1;
            }
            '+' => {
                tokens.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Tok::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Tok::Slash);
                i += 1;
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::GtEq);
                    i += 2;
                } else {
                    tokens.push(Tok::Gt);
                    i += 1;
                }
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::LtEq);
                    i += 2;
                } else {
                    tokens.push(Tok::Lt);
                    i += 1;
                }
            }
            '=' if i + 1 < chars.len() && chars[i + 1] == '=' => {
                tokens.push(Tok::EqEq);
                i += 2;
            }
            '!' if i + 1 < chars.len() && chars[i + 1] == '=' => {
                tokens.push(Tok::BangEq);
                i += 2;
            }
            '(' => {
                tokens.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Tok::Comma);
                i += 1;
            }
            c => return Err(format!("unexpected character '{c}'")),
        }
    }
    tokens.push(Tok::Eof);
    Ok(tokens)
}

// ── Check-only recursive-descent parser ──────────────────────────────────────

struct Checker {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Checker {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> Tok {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect_lparen(&mut self) -> Result<(), String> {
        match self.advance() {
            Tok::LParen => Ok(()),
            t => Err(format!("expected '(', got {t:?}")),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), String> {
        match self.advance() {
            Tok::RParen => Ok(()),
            t => Err(format!("expected ')', got {t:?}")),
        }
    }

    fn expect_str(&mut self) -> Result<String, String> {
        match self.advance() {
            Tok::Str(s) => Ok(s),
            t => Err(format!("expected string literal, got {t:?}")),
        }
    }

    fn check_expr(&mut self) -> Result<(), String> {
        self.check_comparison()
    }

    fn check_comparison(&mut self) -> Result<(), String> {
        self.check_term()?;
        match self.peek() {
            Tok::Gt | Tok::Lt | Tok::GtEq | Tok::LtEq | Tok::EqEq | Tok::BangEq => {
                self.advance();
                self.check_term()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn check_term(&mut self) -> Result<(), String> {
        self.check_factor()?;
        loop {
            match self.peek() {
                Tok::Plus | Tok::Minus => {
                    self.advance();
                    self.check_factor()?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn check_factor(&mut self) -> Result<(), String> {
        self.check_unary()?;
        loop {
            match self.peek() {
                Tok::Star | Tok::Slash => {
                    self.advance();
                    self.check_unary()?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn check_unary(&mut self) -> Result<(), String> {
        if matches!(self.peek(), Tok::Minus) {
            self.advance();
            return self.check_unary();
        }
        self.check_primary()
    }

    fn check_primary(&mut self) -> Result<(), String> {
        match self.advance() {
            Tok::Num => Ok(()),
            Tok::LParen => {
                self.check_expr()?;
                self.expect_rparen()
            }
            Tok::Ident(name) => self.check_call(&name),
            other => Err(format!("unexpected token {other:?}")),
        }
    }

    fn check_call(&mut self, name: &str) -> Result<(), String> {
        self.expect_lparen()?;
        let arg = self.expect_str()?;
        self.expect_rparen()?;

        match name {
            "feature" => Ok(()),
            "bar" => {
                if !matches!(arg.as_str(), "open" | "high" | "low" | "close" | "volume") {
                    return Err(format!(
                        "unknown bar field '{arg}'; expected open, high, low, close, or volume"
                    ));
                }
                Ok(())
            }
            other => Err(format!("unknown function '{other}'; expected feature or bar")),
        }
    }
}

fn check_expr(input: &str) -> Result<(), String> {
    let tokens = tokenize(input)?;
    let mut checker = Checker { tokens, pos: 0 };
    checker.check_expr()?;
    if !matches!(checker.peek(), Tok::Eof) {
        return Err(format!("unexpected token after expression: {:?}", checker.peek()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(expr: &str) {
        let errs = validate_expression(expr, "test");
        assert!(errs.is_empty(), "expected ok for {:?}, got: {:?}", expr, errs);
    }

    fn err(expr: &str) {
        let errs = validate_expression(expr, "test");
        assert!(!errs.is_empty(), "expected error for {:?}", expr);
    }

    #[test]
    fn valid_feature_comparison() {
        ok("feature('ema_7') > feature('ema_21')");
    }

    #[test]
    fn valid_bar_field() {
        ok("bar('close') > 100.0");
    }

    #[test]
    fn valid_arithmetic() {
        ok("feature('ema_7') + 5.0 > 14.0");
    }

    #[test]
    fn valid_parenthesised() {
        ok("(feature('ema_7') + feature('ema_21')) / 2.0 > 10.0");
    }

    #[test]
    fn invalid_unknown_function() {
        err("unknown('x') > 1.0");
    }

    #[test]
    fn invalid_bad_bar_field() {
        err("bar('vwap') > 100.0");
    }

    #[test]
    fn invalid_unterminated_string() {
        err("feature('ema_7) > 1.0");
    }

    #[test]
    fn invalid_unexpected_char() {
        err("feature('ema_7') & 1.0");
    }

    #[test]
    fn invalid_trailing_garbage() {
        err("feature('ema_7') > 1.0 garbage");
    }
}
