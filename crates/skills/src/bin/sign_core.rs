//! Sign the core skill set (Set R-a RA-2.5): the release step.
//!
//! ```text
//! TBOT_SKILLS_SIGNING_SECRET=<64 hex> cargo run -p skills --bin sign_core -- skills/core
//! ```
//!
//! Loads and validates `skills/core`, signs its canonical manifest with the Ed25519 secret in
//! the environment, writes `skills/core/MANIFEST.sig`, and prints the **public** key to set
//! as `TBOT_SKILLS_SIGNING_PUBKEY` in the deployment. The secret is read from the
//! environment and never written anywhere; the platform itself never sees it.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "skills/core".into()));
    let Some(secret) = std::env::var("TBOT_SKILLS_SIGNING_SECRET")
        .ok()
        .and_then(|s| skills::signing::parse_secret(&s))
    else {
        eprintln!("TBOT_SKILLS_SIGNING_SECRET must be set to 64 hex characters (an Ed25519 secret key)");
        return ExitCode::from(2);
    };
    let core = match skills::core::load_core_dir(&dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", dir.display());
            return ExitCode::from(1);
        }
    };
    let manifest = skills::core::manifest(&core);
    let sig = skills::signing::sign(&manifest, &secret);
    let out = dir.join(skills::signing::SIGNATURE_FILE);
    if let Err(e) = std::fs::write(&out, &sig) {
        eprintln!("{}: {e}", out.display());
        return ExitCode::from(1);
    }
    println!("signed {} core skills -> {}", core.len(), out.display());
    println!("deploy with {}={}", skills::signing::PUBKEY_ENV, skills::signing::public_key_hex(&secret));
    ExitCode::SUCCESS
}
