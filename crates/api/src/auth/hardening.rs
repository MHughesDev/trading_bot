//! The bind guard: may this process listen on a non-loopback address? (Set E 1.7)
//!
//! `apps/platform` refused every non-loopback bind while auth was a placeholder
//! (M-17). Auth is no longer a placeholder, so the predicate flips from "is the
//! address loopback" to **"is auth hardened"** — and hardened is checked against
//! the running database and environment, not asserted by a comment:
//!
//! 1. **The session store holds digests.** Migration 0150's trigger
//!    (`sessions_store_only_the_hash_trg`) and CHECK exist, so a database dump
//!    carries no bearer credential.
//! 2. **No tenant-less automations can exist.** Migration 0151's
//!    `automations_owner_is_not_nil` CHECK exists (Set E 1.4/1.6).
//! 3. **No dev bypass is switched on.** None of [`DEV_BYPASS_VARS`] is set.
//! 4. **Transport is encrypted.** The operator declares a TLS-terminating proxy
//!    in front of the process with `TRADING_TLS_TERMINATED=1`. A bearer token
//!    over plain HTTP on a network is a credential handed to every hop.
//!
//! CSRF: the SPA authenticates with an `Authorization: Bearer` header from
//! `localStorage`, never a cookie, so a cross-site request carries no
//! credential and there is no CSRF surface to protect. If auth ever moves to
//! cookies, this guard must grow a CSRF check before the move ships.
//!
//! Loopback binds are always allowed: they are the development posture.

use sqlx::PgPool;

/// Environment switches that would weaken auth. None exists in the code today;
/// listing them means one cannot be added and enabled on a network bind
/// without this guard noticing.
pub const DEV_BYPASS_VARS: &[&str] = &["AUTH_DEV_BYPASS", "TRADING_DEV_AUTH", "DEV_LOCAL_TOKEN"];

/// The env var an operator sets to declare TLS termination in front of the API.
pub const TLS_DECLARATION_VAR: &str = "TRADING_TLS_TERMINATED";

/// What the guard knows about this process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HardeningFacts {
    pub sessions_hashed: bool,
    pub automations_owner_scoped: bool,
    /// Names of dev-bypass variables that are set.
    pub dev_bypasses_set: Vec<String>,
    pub tls_declared: bool,
}

/// Whether `host` is a loopback address.
#[must_use]
pub fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The decision, pure. `Ok` to bind; `Err` lists every reason not to.
///
/// # Errors
/// Every failed requirement, in words an operator can act on.
pub fn assess(host: &str, facts: &HardeningFacts) -> Result<(), Vec<String>> {
    if is_loopback(host) {
        return Ok(());
    }
    let mut why = Vec::new();
    if !facts.sessions_hashed {
        why.push(
            "the sessions table does not store token digests (migration 0150 is not applied)"
                .to_owned(),
        );
    }
    if !facts.automations_owner_scoped {
        why.push(
            "automations are not owner-scoped in the database (migration 0151 is not applied)"
                .to_owned(),
        );
    }
    for v in &facts.dev_bypasses_set {
        why.push(format!("dev auth bypass {v} is set"));
    }
    if !facts.tls_declared {
        why.push(format!(
            "no TLS termination is declared; put a TLS proxy in front and set {TLS_DECLARATION_VAR}=1"
        ));
    }
    if why.is_empty() {
        Ok(())
    } else {
        Err(why)
    }
}

/// Read the facts from the database and the environment. A database that
/// cannot be asked yields `false` for its facts — the guard fails closed.
pub async fn gather(pg: &PgPool) -> HardeningFacts {
    let exists = |sql: &'static str| async move {
        sqlx::query_scalar::<_, bool>(sql)
            .fetch_one(pg)
            .await
            .unwrap_or(false)
    };
    HardeningFacts {
        sessions_hashed: exists(
            "SELECT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'sessions_store_only_the_hash_trg') \
               AND EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'sessions_token_is_a_sha256_hex')",
        )
        .await,
        automations_owner_scoped: exists(
            "SELECT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'automations_owner_is_not_nil')",
        )
        .await,
        dev_bypasses_set: DEV_BYPASS_VARS
            .iter()
            .filter(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
            .map(|v| (*v).to_owned())
            .collect(),
        tls_declared: std::env::var(TLS_DECLARATION_VAR).is_ok_and(|v| v.trim() == "1"),
    }
}

/// The call `apps/platform` makes before binding.
///
/// # Errors
/// A message naming every unmet requirement.
pub async fn require_bindable(host: &str, pg: &PgPool) -> Result<(), String> {
    if is_loopback(host) {
        return Ok(());
    }
    assess(host, &gather(pg).await).map_err(|why| {
        format!(
            "SECURITY: refusing to bind non-loopback address '{host}' because auth is not \
             hardened: {}",
            why.join("; ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hardened() -> HardeningFacts {
        HardeningFacts {
            sessions_hashed: true,
            automations_owner_scoped: true,
            dev_bypasses_set: vec![],
            tls_declared: true,
        }
    }

    #[test]
    fn loopback_always_binds() {
        for h in ["127.0.0.1", "::1", "localhost"] {
            assert!(assess(h, &HardeningFacts::default()).is_ok(), "{h}");
        }
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.5"));
    }

    #[test]
    fn a_network_bind_needs_every_requirement() {
        assert!(assess("0.0.0.0", &hardened()).is_ok());
        let mut f = hardened();
        f.sessions_hashed = false;
        assert!(assess("0.0.0.0", &f).is_err());
        let mut f = hardened();
        f.automations_owner_scoped = false;
        assert!(assess("0.0.0.0", &f).is_err());
        let mut f = hardened();
        f.tls_declared = false;
        assert!(assess("0.0.0.0", &f).is_err());
        let mut f = hardened();
        f.dev_bypasses_set = vec!["AUTH_DEV_BYPASS".into()];
        assert!(assess("0.0.0.0", &f).is_err());
        let why = assess("0.0.0.0", &HardeningFacts::default()).unwrap_err();
        assert_eq!(why.len(), 3, "every unmet requirement is named: {why:?}");
    }
}
