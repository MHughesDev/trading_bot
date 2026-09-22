//! The insight embedder is local, and that is a property of the build rather
//! than a promise in a comment (SPEC §5.6, checklist 3.10, ADR-P3-11).
//!
//! Research text is the most sensitive corpus this platform holds: it is what
//! the operator has learned and not yet traded on. The platform already
//! contains an embedder that posts text to a third party — `crates/semantic`,
//! driven by `apps/embedder` — so "the insight embedder is local" is a claim
//! that can quietly stop being true with one `use` line.
//!
//! These tests read the actual source and the actual manifest that compiled,
//! via `include_str!`, and assert three things:
//!
//!   1. the `api` crate does not depend on `semantic` at all, so no code in it
//!      can reach the third-party embedding path even by accident;
//!   2. the insight read path names no HTTP client, socket type, URL or API
//!      key — checked over code, with comments stripped, because a comment
//!      cannot make a request but can trivially defeat a naive grep;
//!   3. `Embedder::embed` is a synchronous function taking no client, key or
//!      URL, and is exercised here outside any async runtime.
//!
//! What this proves and what it does not: it proves no *source-level* egress
//! path exists from a claim to the network in this crate. It is not a runtime
//! packet capture, and it would not catch egress introduced through a
//! transitive dependency's side effect. It is the strongest check available
//! without adding a dependency, and it is checked by CI on every change.

/// Source of the read path, as it compiled.
const INSIGHT_MEMORY: &str = include_str!("../src/insight_memory.rs");
/// Source of the HTTP surface, as it compiled.
const INSIGHT_ROUTES: &str = include_str!("../src/routes/insights.rs");
/// The manifest that decides what this crate can even name.
const API_MANIFEST: &str = include_str!("../Cargo.toml");

/// Strip line comments so the scan looks at code, not at prose about code.
fn code_only(src: &str) -> String {
    src.lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every way this workspace has ever made an outbound request.
const NETWORK_SYMBOLS: &[&str] = &[
    "reqwest",
    "http://",
    "https://",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "std::net",
    "tokio::net",
    "api_key",
    "OPENAI",
    "openai",
    "semantic::",
    "MilvusClient",
];

#[test]
fn the_insight_read_path_has_no_network_egress() {
    for (name, src) in [
        ("insight_memory.rs", INSIGHT_MEMORY),
        ("routes/insights.rs", INSIGHT_ROUTES),
    ] {
        let code = code_only(src);
        for sym in NETWORK_SYMBOLS {
            assert!(
                !code.contains(sym),
                "{name} names `{sym}`. Research text does not leave this machine; if an \
                 embedding service is ever wanted it belongs behind an explicit, \
                 loopback-pinned adapter with its own decision record, not inside the \
                 retrieval path."
            );
        }
    }
}

/// The strongest form of the guarantee: the crate cannot name the offending
/// code. `crates/semantic` embeds through `api.openai.com` and upserts into
/// Milvus; if `api` does not depend on it, no handler in `api` can call it, and
/// no future edit to the insight path can reach it without a manifest change
/// that shows up in review.
#[test]
fn the_api_crate_cannot_reach_the_third_party_embedder() {
    let deps = code_only(API_MANIFEST);
    for forbidden in ["semantic", "embedder"] {
        assert!(
            !deps
                .lines()
                .any(|l| l.trim_start().starts_with(forbidden)),
            "crates/api/Cargo.toml declares `{forbidden}`. The insight store's embedder is \
             local; a dependency on the third-party embedding path removes that guarantee \
             whether or not today's code calls it."
        );
    }
}

/// `embed` is synchronous and takes nothing but the text. A function with no
/// client, no key, no URL and no async runtime cannot make a request, and this
/// test runs it with no runtime at all to say so out loud.
#[test]
fn embedding_runs_with_no_async_runtime_and_no_credentials() {
    use api::insight_memory::{Embedder, LocalHashEmbedder, EMBEDDING_DIMS};

    // Deliberately not `#[tokio::test]`: there is no reactor here, so anything
    // that tried to await a socket would panic rather than succeed quietly.
    let v = LocalHashEmbedder.embed("BTC-USD 1h: the EMA cross edge is spanned by momentum");
    assert_eq!(v.len(), EMBEDDING_DIMS);
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4);

    // Whatever credentials happen to be in the environment, the vector is the
    // same function of the text — nothing here consults them.
    std::env::set_var("OPENAI_API_KEY", "sk-not-used-by-anything-here");
    let again = LocalHashEmbedder.embed("BTC-USD 1h: the EMA cross edge is spanned by momentum");
    std::env::remove_var("OPENAI_API_KEY");
    assert_eq!(v, again);
}

/// The one-line version of the rule, kept where a reader of the module will see
/// it: retrieved memory is quoted material for hypothesis generation, and never
/// evidence.
#[test]
fn the_retrieval_contract_is_stated_on_every_response() {
    let usage = api::insight_memory::RETRIEVAL_USAGE;
    assert!(usage.contains("never evidence"));
    assert!(usage.contains("not instructions"));
}
