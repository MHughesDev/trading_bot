//! Release signing of the core skill set (Set R-a RA-2.5; AGENT-003 §6.2 step 6).
//!
//! The bundle *hash* check has always been there (`core::materialise`, `mount::write_mount`):
//! a bundle whose bytes differ from its recorded digest aborts before anything is written.
//! A hash proves the bytes are the ones the manifest names; it does not prove the manifest
//! is the release's. The **signature** does: the release signs the canonical manifest (every
//! core skill's name and digest) and the platform verifies it against a public key it was
//! deployed with.
//!
//! # When a key is configured
//!
//! With `TBOT_SKILLS_SIGNING_PUBKEY` unset, nothing is required and nothing is claimed:
//! [`check_core`] returns [`Status::NotRequired`]. With it set (64 hex characters, an
//! Ed25519 public key), an unsigned core set is **refused**: `skills/core/MANIFEST.sig` must
//! exist and verify. The refusal is the whole point, so there is no "warn and continue".
//!
//! # What this does not decide
//!
//! Where the *secret* key lives, who holds it and how the public half reaches the deploy.
//! That is a human decision (Set R-a RA-2.5's blocker); the steps are in the task. Signing is
//! done by `cargo run -p skills --bin sign_core`, which reads the secret from the environment
//! and writes `MANIFEST.sig`; the secret is never read by the platform.

use std::collections::BTreeMap;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::core::{manifest, CoreSkill};

/// The environment variable holding the release's Ed25519 public key, hex.
pub const PUBKEY_ENV: &str = "TBOT_SKILLS_SIGNING_PUBKEY";
/// The detached signature, next to the skills it covers.
pub const SIGNATURE_FILE: &str = "MANIFEST.sig";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignError {
    #[error("{PUBKEY_ENV} is not 64 hex characters (an Ed25519 public key)")]
    BadPublicKey,
    #[error("the core skill set is unsigned: a signing key is configured and `{SIGNATURE_FILE}` is missing or unreadable")]
    Unsigned,
    #[error("`{SIGNATURE_FILE}` is not a valid signature encoding")]
    BadSignatureEncoding,
    #[error("the core manifest does not match its signature: the skills are not the release's")]
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No key is configured; nothing was checked or claimed.
    NotRequired,
    /// A key is configured and the manifest verified.
    Verified,
}

/// The bytes that are signed: `name<TAB>digest<LF>` for every skill, sorted by name.
#[must_use]
pub fn canonical_manifest(m: &BTreeMap<String, String>) -> Vec<u8> {
    let mut out = String::new();
    for (name, digest) in m {
        out.push_str(name);
        out.push('\t');
        out.push_str(digest);
        out.push('\n');
    }
    out.into_bytes()
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let v = hex::decode(s.trim()).ok()?;
    <[u8; 32]>::try_from(v).ok()
}

/// Sign a manifest. Used by release tooling only.
#[must_use]
pub fn sign(m: &BTreeMap<String, String>, secret: &[u8; 32]) -> String {
    let key = SigningKey::from_bytes(secret);
    hex::encode(key.sign(&canonical_manifest(m)).to_bytes())
}

/// Verify a manifest against a hex signature and a hex public key.
///
/// # Errors
/// [`SignError`].
pub fn verify(m: &BTreeMap<String, String>, sig_hex: &str, pubkey_hex: &str) -> Result<(), SignError> {
    let pk = VerifyingKey::from_bytes(&hex32(pubkey_hex).ok_or(SignError::BadPublicKey)?)
        .map_err(|_| SignError::BadPublicKey)?;
    let raw = hex::decode(sig_hex.trim()).map_err(|_| SignError::BadSignatureEncoding)?;
    let sig = Signature::from_slice(&raw).map_err(|_| SignError::BadSignatureEncoding)?;
    pk.verify(&canonical_manifest(m), &sig).map_err(|_| SignError::Mismatch)
}

/// The policy applied to a loaded core set: with no key, not required; with one, the
/// signature file must exist and verify.
///
/// # Errors
/// [`SignError`] when a key is configured and the set is unsigned or does not verify.
pub fn check_core(core: &[CoreSkill], core_dir: &Path, pubkey_hex: Option<&str>) -> Result<Status, SignError> {
    let Some(pk) = pubkey_hex.map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(Status::NotRequired);
    };
    let sig = std::fs::read_to_string(core_dir.join(SIGNATURE_FILE)).map_err(|_| SignError::Unsigned)?;
    verify(&manifest(core), &sig, pk)?;
    Ok(Status::Verified)
}

/// [`check_core`] with the key read from the environment.
///
/// # Errors
/// As [`check_core`].
pub fn check_core_from_env(core: &[CoreSkill], core_dir: &Path) -> Result<Status, SignError> {
    check_core(core, core_dir, std::env::var(PUBKEY_ENV).ok().as_deref())
}

/// The public key (hex) for a secret, so the human step can print what to deploy.
#[must_use]
pub fn public_key_hex(secret: &[u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(secret).verifying_key().to_bytes())
}

/// Parse a 64-hex-character secret.
#[must_use]
pub fn parse_secret(hex_str: &str) -> Option<[u8; 32]> {
    hex32(hex_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/core")
    }

    const SECRET: [u8; 32] = [7; 32];

    #[test]
    fn a_signed_manifest_verifies_and_a_changed_one_does_not() {
        let core = crate::core::load_core_dir(&core_root()).unwrap();
        let m = manifest(&core);
        let sig = sign(&m, &SECRET);
        let pk = public_key_hex(&SECRET);
        assert_eq!(verify(&m, &sig, &pk), Ok(()));
        let mut changed = m.clone();
        changed.insert("playbook".into(), "0".repeat(64));
        assert_eq!(verify(&changed, &sig, &pk), Err(SignError::Mismatch));
        let mut extra = m.clone();
        extra.insert("sneaked-in".into(), "1".repeat(64));
        assert_eq!(verify(&extra, &sig, &pk), Err(SignError::Mismatch), "an added skill invalidates it too");
        // Another key's signature is refused.
        assert_eq!(verify(&m, &sign(&m, &[9; 32]), &pk), Err(SignError::Mismatch));
        assert_eq!(verify(&m, "zz", &pk), Err(SignError::BadSignatureEncoding));
        assert_eq!(verify(&m, &sig, "short"), Err(SignError::BadPublicKey));
    }

    #[test]
    fn with_no_key_nothing_is_required_and_with_a_key_an_unsigned_set_is_refused() {
        let core = crate::core::load_core_dir(&core_root()).unwrap();
        let dir = std::env::temp_dir().join(format!("signing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(check_core(&core, &dir, None), Ok(Status::NotRequired));
        assert_eq!(check_core(&core, &dir, Some("  ")), Ok(Status::NotRequired), "a blank key is no key");
        let pk = public_key_hex(&SECRET);
        assert_eq!(check_core(&core, &dir, Some(&pk)), Err(SignError::Unsigned));
        std::fs::write(dir.join(SIGNATURE_FILE), sign(&manifest(&core), &SECRET)).unwrap();
        assert_eq!(check_core(&core, &dir, Some(&pk)), Ok(Status::Verified));
        // A tampered core skill no longer matches the signed manifest.
        let mut tampered = core.clone();
        tampered[0].digest = "f".repeat(64);
        assert_eq!(check_core(&tampered, &dir, Some(&pk)), Err(SignError::Mismatch));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_manifest_encoding_is_sorted_and_stable() {
        let mut m = BTreeMap::new();
        m.insert("b".to_string(), "2".to_string());
        m.insert("a".to_string(), "1".to_string());
        assert_eq!(canonical_manifest(&m), b"a\t1\nb\t2\n".to_vec());
    }
}
