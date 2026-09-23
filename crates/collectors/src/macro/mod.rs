//! Macro-economic series (DATA-005 §11 S4; Set P, P-3.1).
//!
//! The module name is spelled `r#macro` in Rust source because `macro` is a
//! reserved word; the path `collectors::macro::fred` in DATA-005 §11 resolves to
//! it unchanged.

pub mod fred;
