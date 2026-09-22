//! Credentials that cannot be printed (Set P MASTER §5, P-3.1).
//!
//! API keys come from the encrypted credential store, never from `config/`. The
//! store is the *source*; this type is what stops the key escaping once loaded.
//! `Debug` and `Display` both redact, so the ordinary ways a key reaches a log —
//! `tracing::info!(?req)`, `dbg!`, a `{:?}` in an error — print `***` instead.
//!
//! The only way to the plaintext is [`Secret::expose`], which is named so that a
//! reviewer can grep for every place it happens.

use std::fmt;

/// A credential whose plaintext is reachable only through [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The plaintext. Every call site is a place a key could leak — keep them few.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The last four characters, for display-safe status rows.
    #[must_use]
    pub fn last4(&self) -> String {
        let n = self.0.chars().count();
        self.0.chars().skip(n.saturating_sub(4)).collect()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Where a collector's credential came from.
///
/// A source of `Config` is not representable: there is no constructor for it,
/// because a key in `config/` is a key in the repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialOrigin {
    /// The encrypted credential store (migration 0007 / 0034).
    CredentialStore,
    /// Process environment, used by integration tests and local one-off runs.
    Environment,
}

/// A credential and where it came from.
#[derive(Clone, Debug)]
pub struct Credential {
    pub secret: Secret,
    pub origin: CredentialOrigin,
}

impl Credential {
    #[must_use]
    pub fn from_store(secret: Secret) -> Self {
        Self {
            secret,
            origin: CredentialOrigin::CredentialStore,
        }
    }

    #[must_use]
    pub fn from_env(secret: Secret) -> Self {
        Self {
            secret,
            origin: CredentialOrigin::Environment,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_both_redact() {
        let s = Secret::new("abcd1234efgh");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(format!("{s}"), "***");
        assert!(!format!("{s:?} {s}").contains("abcd1234efgh"));
    }

    /// The adversarial half: a struct that merely *contains* a key must not leak
    /// it through a derived Debug either, which is the usual accident.
    #[test]
    fn a_derived_debug_on_a_containing_struct_redacts_too() {
        #[derive(Debug)]
        #[allow(dead_code)] // read only through the Debug rendering under test
        struct Client {
            base: &'static str,
            key: Secret,
        }
        let c = Client {
            base: "https://api.stlouisfed.org",
            key: Secret::new("supersecretkey"),
        };
        let rendered = format!("{c:?}");
        assert!(!rendered.contains("supersecretkey"), "{rendered}");
        assert!(rendered.contains("***"));
    }

    #[test]
    fn last4_is_display_safe() {
        assert_eq!(Secret::new("abcd1234efgh").last4(), "efgh");
        assert_eq!(Secret::new("ab").last4(), "ab");
    }

    #[test]
    fn expose_is_the_only_way_out() {
        assert_eq!(Secret::new("k").expose(), "k");
    }
}
