//! Daily signed anchors and whole-ledger verification (SPEC §4.6, ADR-019).
//!
//! An anchor fixes the tip of a tenant's trial and decision chains on a given day.
//! It is signed with an ed25519 key and written once to a write-once store before
//! its row is inserted, so rewriting history needs the database, the WORM store and
//! the signing key at the same time. The verifier recomputes both chains, every
//! trial's event chain, and checks each anchor's heads and signature against them.

use std::io::Write;
use std::path::PathBuf;

use chrono::NaiveDate;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{verify_events, verify_trials, ChainBreak, EventRow, TrialRow, GENESIS_HASH};

/// The facts an anchor attests to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorBody {
    pub tenant_id: String,
    pub anchor_date: NaiveDate,
    /// `-1` when the stream is empty; the head is then the genesis hash.
    pub trial_max_seq: i64,
    #[serde(with = "hex_bytes")]
    pub trial_head: Vec<u8>,
    pub decision_max_seq: i64,
    #[serde(with = "hex_bytes")]
    pub decision_head: Vec<u8>,
}

impl AnchorBody {
    /// The exact bytes that are signed. Field order and separators are fixed.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        format!(
            "ledger-anchor/v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
            self.tenant_id,
            self.anchor_date,
            self.trial_max_seq,
            hex::encode(&self.trial_head),
            self.decision_max_seq,
            hex::encode(&self.decision_head),
        )
        .into_bytes()
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        hex::decode(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    #[error("signing key must be 32 bytes of hex (64 characters)")]
    BadKey,
    #[error("anchor for {tenant} on {date} already exists in the WORM store")]
    AlreadyWritten { tenant: String, date: NaiveDate },
    #[error("WORM store: {0}")]
    Worm(String),
    #[error("refusing to anchor a broken chain: {0}")]
    Broken(ChainBreak),
    #[error("backend: {0}")]
    Backend(String),
}

/// An ed25519 signing identity. `key_id` names the public key, so rotated keys stay
/// verifiable: the verifier is handed every key it should accept.
pub struct AnchorSigner {
    key: SigningKey,
    key_id: String,
}

impl AnchorSigner {
    /// # Errors
    /// Anything other than 64 hex characters.
    pub fn from_hex_seed(seed_hex: &str) -> Result<Self, AnchorError> {
        let bytes: [u8; 32] = hex::decode(seed_hex.trim()).ok().and_then(|b| b.try_into().ok()).ok_or(AnchorError::BadKey)?;
        let key = SigningKey::from_bytes(&bytes);
        let key_id = key_id_of(&key.verifying_key());
        Ok(Self { key, key_id })
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    #[must_use]
    pub fn sign(&self, body: &AnchorBody) -> Vec<u8> {
        self.key.sign(&body.canonical_bytes()).to_bytes().to_vec()
    }
}

/// `ed25519:` + the first 16 hex characters of sha256(public key).
#[must_use]
pub fn key_id_of(key: &VerifyingKey) -> String {
    format!("ed25519:{}", &hex::encode(Sha256::digest(key.as_bytes()))[..16])
}

/// Check one signature against the set of accepted keys.
#[must_use]
pub fn signature_valid(body: &AnchorBody, signature: &[u8], key_id: &str, keys: &[VerifyingKey]) -> bool {
    let Ok(sig) = Signature::from_slice(signature) else { return false };
    keys.iter().filter(|k| key_id_of(k) == key_id).any(|k| k.verify(&body.canonical_bytes(), &sig).is_ok())
}

/// A store that accepts each object exactly once.
pub trait WormStore: Send + Sync {
    /// Write `bytes` at `key`, refusing if anything already exists there.
    ///
    /// # Errors
    /// [`AnchorError::AlreadyWritten`] or a store failure.
    fn put_once(&self, key: &str, bytes: &[u8]) -> Result<String, AnchorError>;
    /// # Errors
    /// Store failures; `Ok(None)` when absent.
    fn get(&self, uri: &str) -> Result<Option<Vec<u8>>, AnchorError>;
}

/// Filesystem write-once store: `create_new` refuses to overwrite and the file is
/// made read-only. Point it at a volume with object-lock semantics in production.
pub struct FsWorm {
    root: PathBuf,
}

impl FsWorm {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl WormStore for FsWorm {
    fn put_once(&self, key: &str, bytes: &[u8]) -> Result<String, AnchorError> {
        if key.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
            return Err(AnchorError::Worm(format!("invalid key {key}")));
        }
        let path = self.root.join(key);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AnchorError::Worm(e.to_string()))?;
        }
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(AnchorError::Worm(format!("{} already exists", path.display())));
            }
            Err(e) => return Err(AnchorError::Worm(e.to_string())),
        };
        f.write_all(bytes).and_then(|()| f.sync_all()).map_err(|e| AnchorError::Worm(e.to_string()))?;
        let mut perms = f.metadata().map_err(|e| AnchorError::Worm(e.to_string()))?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).map_err(|e| AnchorError::Worm(e.to_string()))?;
        Ok(format!("file://{}", path.display()).replace('\\', "/"))
    }

    fn get(&self, uri: &str) -> Result<Option<Vec<u8>>, AnchorError> {
        let path = uri.strip_prefix("file://").ok_or_else(|| AnchorError::Worm(format!("not a file uri: {uri}")))?;
        match std::fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AnchorError::Worm(e.to_string())),
        }
    }
}

/// The document written to WORM for one anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorDocument {
    pub body: AnchorBody,
    pub key_id: String,
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
}

/// A decision row as read back. `candidate_set_text` / `chosen_text` are Postgres's
/// own `jsonb::text` rendering, which is what the trigger hashes.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRow {
    pub decision_id: Uuid,
    pub tenant_id: String,
    pub seq: i64,
    pub decision_kind: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub context_hash: String,
    pub candidate_set_text: String,
    pub chosen_text: String,
    pub policy_id: String,
    pub policy_version: i32,
    pub propensity: Option<f64>,
    pub exploration_flag: bool,
    pub decision_tier: String,
    pub prev_hash: Vec<u8>,
    pub row_hash: Vec<u8>,
}

impl DecisionRow {
    /// Mirrors `mlops.decision_chain()` byte for byte.
    #[must_use]
    pub fn recompute_hash(&self) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(&self.prev_hash);
        h.update(self.decision_id.to_string());
        h.update(self.seq.to_string());
        h.update(&self.tenant_id);
        h.update(&self.decision_kind);
        h.update(&self.actor_kind);
        h.update(&self.actor_id);
        h.update(&self.context_hash);
        h.update(Sha256::digest(self.candidate_set_text.as_bytes()));
        h.update(Sha256::digest(self.chosen_text.as_bytes()));
        h.update(&self.policy_id);
        h.update(self.policy_version.to_string());
        if let Some(p) = self.propensity {
            h.update(p.to_be_bytes());
        }
        h.update(if self.exploration_flag { "true" } else { "false" });
        h.update(&self.decision_tier);
        h.finalize().to_vec()
    }
}

/// Verify a tenant's decision stream (ascending `seq`).
///
/// # Errors
/// The first row whose sequence, linkage or content hash disagrees.
pub fn verify_decisions(rows: &[DecisionRow]) -> Result<usize, ChainBreak> {
    let mut prev = GENESIS_HASH.to_vec();
    for (i, r) in rows.iter().enumerate() {
        let brk = |reason: String| ChainBreak { id: r.decision_id, seq: r.seq, reason };
        if r.seq != i as i64 {
            return Err(brk(format!("expected seq {i}, found {}", r.seq)));
        }
        if r.prev_hash != prev {
            return Err(brk("prev_hash does not match the previous decision".into()));
        }
        if r.recompute_hash() != r.row_hash {
            return Err(brk("row_hash does not match the decision's contents".into()));
        }
        prev.clone_from(&r.row_hash);
    }
    Ok(rows.len())
}

/// A stored anchor row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorRow {
    pub anchor_id: Uuid,
    pub body: AnchorBody,
    pub signature: Vec<u8>,
    pub key_id: String,
    pub worm_uri: String,
}

/// Head of a stream at `max_seq` (genesis for `-1`).
fn head_at(hashes: &[Vec<u8>], max_seq: i64) -> Option<Vec<u8>> {
    if max_seq < 0 {
        return Some(GENESIS_HASH.to_vec());
    }
    usize::try_from(max_seq).ok().and_then(|i| hashes.get(i).cloned())
}

/// Compute the body an anchor would attest to right now.
#[must_use]
pub fn body_for(tenant_id: &str, date: NaiveDate, trials: &[TrialRow], decisions: &[DecisionRow]) -> AnchorBody {
    AnchorBody {
        tenant_id: tenant_id.to_string(),
        anchor_date: date,
        trial_max_seq: trials.len() as i64 - 1,
        trial_head: trials.last().map_or_else(|| GENESIS_HASH.to_vec(), |t| t.row_hash.clone()),
        decision_max_seq: decisions.len() as i64 - 1,
        decision_head: decisions.last().map_or_else(|| GENESIS_HASH.to_vec(), |d| d.row_hash.clone()),
    }
}

/// Everything a verification pass found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub tenant_id: String,
    pub trials: usize,
    pub events: usize,
    pub decisions: usize,
    pub anchors: usize,
    pub failures: Vec<String>,
}

impl VerificationReport {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Verify every chain and every anchor for one tenant.
pub fn verify_all(
    tenant_id: &str,
    trials: &[TrialRow],
    events: &[(Uuid, Vec<EventRow>)],
    decisions: &[DecisionRow],
    anchors: &[AnchorRow],
    keys: &[VerifyingKey],
    worm: Option<&dyn WormStore>,
) -> VerificationReport {
    let mut r = VerificationReport { tenant_id: tenant_id.to_string(), ..VerificationReport::default() };
    match verify_trials(trials) {
        Ok(n) => r.trials = n,
        Err(b) => r.failures.push(format!("trial chain: {b}")),
    }
    let by_id: std::collections::HashMap<Uuid, &TrialRow> = trials.iter().map(|t| (t.trial_id, t)).collect();
    for (trial_id, evs) in events {
        match by_id.get(trial_id) {
            Some(t) => match verify_events(t, evs) {
                Ok(n) => r.events += n,
                Err(b) => r.failures.push(format!("event chain of {trial_id}: {b}")),
            },
            None => r.failures.push(format!("events for unknown trial {trial_id}")),
        }
    }
    match verify_decisions(decisions) {
        Ok(n) => r.decisions = n,
        Err(b) => r.failures.push(format!("decision chain: {b}")),
    }
    let trial_hashes: Vec<Vec<u8>> = trials.iter().map(|t| t.row_hash.clone()).collect();
    let decision_hashes: Vec<Vec<u8>> = decisions.iter().map(|d| d.row_hash.clone()).collect();
    for a in anchors {
        let label = format!("anchor {} ({})", a.body.anchor_date, a.anchor_id);
        if head_at(&trial_hashes, a.body.trial_max_seq).as_ref() != Some(&a.body.trial_head) {
            r.failures.push(format!("{label}: trial head no longer matches the chain"));
        }
        if head_at(&decision_hashes, a.body.decision_max_seq).as_ref() != Some(&a.body.decision_head) {
            r.failures.push(format!("{label}: decision head no longer matches the chain"));
        }
        if !signature_valid(&a.body, &a.signature, &a.key_id, keys) {
            r.failures.push(format!("{label}: signature does not verify under any accepted key"));
        }
        if let Some(w) = worm {
            match w.get(&a.worm_uri) {
                Ok(Some(bytes)) => match serde_json::from_slice::<AnchorDocument>(&bytes) {
                    Ok(doc) if doc.body == a.body && doc.signature == a.signature && doc.key_id == a.key_id => {}
                    Ok(_) => r.failures.push(format!("{label}: database row disagrees with the WORM copy")),
                    Err(e) => r.failures.push(format!("{label}: WORM copy unreadable: {e}")),
                },
                Ok(None) => r.failures.push(format!("{label}: WORM copy missing")),
                Err(e) => r.failures.push(format!("{label}: {e}")),
            }
        }
        r.anchors += 1;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> AnchorSigner {
        AnchorSigner::from_hex_seed(&"07".repeat(32)).unwrap()
    }

    fn body() -> AnchorBody {
        AnchorBody {
            tenant_id: "t".into(),
            anchor_date: NaiveDate::from_ymd_opt(2026, 9, 12).unwrap(),
            trial_max_seq: -1,
            trial_head: GENESIS_HASH.to_vec(),
            decision_max_seq: -1,
            decision_head: GENESIS_HASH.to_vec(),
        }
    }

    #[test]
    fn signatures_bind_every_field() {
        let s = signer();
        let b = body();
        let sig = s.sign(&b);
        let keys = [s.verifying_key()];
        assert!(signature_valid(&b, &sig, s.key_id(), &keys));
        let mut moved = b.clone();
        moved.trial_max_seq = 0;
        assert!(!signature_valid(&moved, &sig, s.key_id(), &keys));
        let mut other_day = b.clone();
        other_day.anchor_date = NaiveDate::from_ymd_opt(2026, 9, 13).unwrap();
        assert!(!signature_valid(&other_day, &sig, s.key_id(), &keys));
        let stranger = AnchorSigner::from_hex_seed(&"08".repeat(32)).unwrap();
        assert!(!signature_valid(&b, &sig, s.key_id(), &[stranger.verifying_key()]));
    }

    #[test]
    fn bad_keys_are_refused() {
        assert!(AnchorSigner::from_hex_seed("").is_err());
        assert!(AnchorSigner::from_hex_seed(&"zz".repeat(32)).is_err());
        assert!(AnchorSigner::from_hex_seed(&"07".repeat(31)).is_err());
    }

    #[test]
    fn worm_refuses_a_second_write() {
        let dir = std::env::temp_dir().join(format!("worm-{}", Uuid::new_v4()));
        let w = FsWorm::new(&dir);
        let uri = w.put_once("t/2026-09-12.json", b"one").unwrap();
        assert!(w.put_once("t/2026-09-12.json", b"two").is_err());
        assert_eq!(w.get(&uri).unwrap().unwrap(), b"one");
        assert!(w.put_once("../escape.json", b"x").is_err());
        let path = uri.strip_prefix("file://").unwrap();
        let mut p = std::fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        std::fs::set_permissions(path, p).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_moved_anchor_head_is_detected() {
        let s = signer();
        let b = body();
        let sig = s.sign(&b);
        let row = AnchorRow { anchor_id: Uuid::nil(), body: b.clone(), signature: sig, key_id: s.key_id().into(), worm_uri: String::new() };
        let ok = verify_all("t", &[], &[], &[], std::slice::from_ref(&row), &[s.verifying_key()], None);
        assert!(ok.ok(), "{:?}", ok.failures);
        let mut forged = row;
        forged.body.trial_head = vec![1; 32];
        let bad = verify_all("t", &[], &[], &[], &[forged], &[s.verifying_key()], None);
        assert_eq!(bad.failures.len(), 2, "head mismatch and signature failure: {:?}", bad.failures);
    }
}
