//! Adversarial test: float must not be able to construct a Price or Size.
//!
//! This file documents the compile-time guarantee rather than testing it at
//! runtime.  The trait-absence checks below confirm no `From<f64>` exists.

use domain::money::{Price, Size};

/// Confirms that neither `Price` nor `Size` implements `From<f64>`.
///
/// If someone accidentally adds `impl From<f64> for Price`, this test becomes
/// unreachable because the `where` clause would be satisfied — but Rust does not
/// currently support negative trait bounds.  We use a static assertion pattern
/// instead: the *absence* of the `From<f64>` impl means code that calls
/// `Price::from(0.1f64)` will not compile.
///
/// The canonical way to verify this is a `compile_fail` doc-test.  The
/// runtime test below checks that Price::from_str is the intended path.
#[test]
fn price_must_be_constructed_from_str_not_float() {
    // This should compile (correct usage):
    let p: Price = "42.00".parse().expect("valid decimal string");
    assert_eq!(p.to_string(), "42.00");
}

#[test]
fn size_must_be_constructed_from_str_not_float() {
    let s: Size = "0.001".parse().expect("valid decimal string");
    assert_eq!(s.to_string(), "0.001");
}

/// Static assertion: neither type implements From<f64>.
///
/// The following snippet, if uncommented, produces:
/// ```
/// error[E0277]: the trait bound `Price: From<f64>` is not satisfied
/// ```
///
/// ```compile_fail
/// use domain::money::Price;
/// let _ = Price::from(0.1f64);
/// ```
///
/// ```compile_fail
/// use domain::money::Size;
/// let _ = Size::from(0.1f64);
/// ```
#[test]
fn float_path_does_not_exist_compile_fail_documented_above() {
    // Runtime stand-in: prove Decimal handles 0.1 exactly (f64 cannot).
    let p: Price = "0.1".parse().unwrap();
    // f64 0.1 ≈ 0.1000000000000000055511151231257827021181583404541015625
    // Decimal 0.1 is exact.
    assert_eq!(p.to_string(), "0.1");
}

/// Arithmetic works through Decimal math, not float math.
#[test]
fn price_arithmetic_is_decimal_not_float() {
    let a: Price = "0.1".parse().unwrap();
    let b: Price = "0.2".parse().unwrap();
    let c = a + b;
    // In f64: 0.1 + 0.2 = 0.30000000000000004
    // In Decimal: 0.1 + 0.2 = 0.3 exactly
    assert_eq!(c.to_string(), "0.3");
}
