//! S-10.9, the clause that was left: **no agent tool writes the promoted set**, except
//! through an approval envelope (AGENT-013 §4, §7.5; Set S task S-10.9).
//!
//! # The product decision this encodes (ADR-S10-13)
//!
//! The task said "removing every other write path into the promoted set" could not mean
//! all thirty-five ring-1 writers, because ring 1 is the agent's whole working surface,
//! and left open *which registries count as promoted*. The answer is the one the rings
//! already draw: the **promoted set is what a trade, an order or a limit reads** --
//!
//! * automations' armed state (`/api/automations/{id}/arm|disarm`),
//! * deployments and the proposal-deciding routes (`/api/deployments`, `/api/deployment-*`
//!   except the proposal door itself),
//! * the model alias a strategy resolves (`/api/models/.../promote`, `.../deployments`),
//! * orders, the trading kill switch, control limits and the monitor (`/api/orders`,
//!   `/api/trading/`, `/api/monitor/`),
//! * paper accounts (`/api/paper/`).
//!
//! A strategy row, an experiment, a model version, a draft or a skill is **not** in it:
//! those are the platform's artifacts (ring 1), and an agent that cannot write them cannot
//! do research. Rejected alternative: treating every `Risk::Write` registry as promoted,
//! which removes the agent's ability to create a strategy at all -- the platform's main flow.
//!
//! # What is checked
//!
//! By source scan, because a tool handler is a thin HTTP client and the route it names is
//! the whole of what it can reach. A function in `crates/mcp-server/src/tools/**` that both
//! names a promoted path and issues a write (`post`/`put`/`delete`/`patch`) must be on
//! [`ALLOWED`], and every allowed writer must be a catalogue tool that is either
//! `Envelope::Promotion` or a stated exposure-reducing exception. A new tool reaching the
//! promoted set therefore fails here until somebody decides, in this file, that it may.
//!
//! **What the scan cannot see:** a path assembled from pieces that never appear as a
//! literal, or a write through a helper in another module. The first would have to avoid
//! every prefix below; the second is caught from the other side by
//! `every_promotion_action_that_is_a_tool_reaches_ring_two` and
//! `a_proposal_and_the_tool_it_authorises_reach_the_same_ring`, which read the catalogue.

use std::path::{Path, PathBuf};

/// A path that, written to, changes what the platform trades or may trade.
const PROMOTED: &[(&str, &str)] = &[
    ("/api/automations/", ""),
    ("/api/deployments", ""),
    ("/api/deployment-ceilings", ""),
    ("/api/orders", ""),
    ("/api/trading/", ""),
    ("/api/monitor/", ""),
    ("/api/paper/", ""),
    ("/api/models/", "/promote"),
    ("/api/models/", "/deployments"),
];

/// The proposal door: asking is never gated, deciding is a human's.
const DOORS: &[&str] = &["/api/deployment-proposals", "/api/lifecycle/proposals"];

/// Functions that may write a promoted path, each with why. A function is named once; the
/// catalogue tools that dispatch to it are checked below.
const ALLOWED: &[(&str, &str)] = &[
    (
        "toggle_armed",
        "`arm_automation` (Promotion) and `disarm_automation` (a reduction in exposure: \
         switching something off is the safe direction and is not gated)",
    ),
    // `promote_model_version` was on this list while it POSTed `/api/models/{id}/promote`. Since
    // Set N N-4.3 it files a proposal (`.../promotion-proposals`) a person decides, which is the
    // proposal door and no longer a write into the promoted set, so it came off the list (the
    // staleness check below demands that). It stays Promotion-enveloped, ring 2: see
    // `the_allowed_writers_are_gated_or_say_why_not`, which still names it.
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The code that ships: before `#[cfg(test)]`, comment lines removed.
fn shipped(src: &str) -> String {
    src.split("#[cfg(test)]")
        .next()
        .unwrap_or(src)
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn names_promoted_path(body: &str) -> bool {
    let mut from = 0;
    while let Some(p) = body[from..].find("\"/api/") {
        let start = from + p + 1;
        let end = body[start..].find('"').map_or(body.len(), |e| start + e);
        let lit = &body[start..end];
        from = end;
        if DOORS.iter().any(|d| lit.starts_with(d)) {
            continue;
        }
        if PROMOTED
            .iter()
            .any(|(pre, suf)| lit.starts_with(pre) && lit.contains(suf))
        {
            return true;
        }
    }
    false
}

fn issues_a_write(body: &str) -> bool {
    [".post(", ".put(", ".delete(", ".patch("]
        .iter()
        .any(|m| body.contains(m))
}

/// `(function name)` for every function that names a promoted path and writes.
fn promoted_writers(src: &str) -> Vec<String> {
    let code = shipped(src);
    let mut out = Vec::new();
    let mut starts: Vec<(usize, String)> = Vec::new();
    let mut from = 0;
    while let Some(p) = code[from..].find("fn ") {
        let at = from + p;
        from = at + 3;
        let before = code[..at].chars().last();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let name: String = code[at + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            starts.push((at, name));
        }
    }
    for (i, (at, name)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(code.len(), |(n, _)| *n);
        let body = &code[*at..end];
        if names_promoted_path(body) && issues_a_write(body) {
            out.push(name.clone());
        }
    }
    out
}

#[test]
fn the_scan_flags_a_planted_writer_and_passes_a_reader() {
    let planted = r#"
        pub async fn sneaky(api: &ApiClient) -> Value {
            api.post("/api/orders", json!({})).await.unwrap()
        }
        pub async fn reads(api: &ApiClient) -> Value {
            api.get("/api/monitor/kill-switch").await.unwrap()
        }
        pub async fn asks(api: &ApiClient) -> Value {
            api.post("/api/deployment-proposals", json!({})).await.unwrap()
        }
        pub async fn built_elsewhere(api: &ApiClient, id: &str) -> Value {
            let p = format!("/api/automations/{id}/arm");
            api.post(&p, json!({})).await.unwrap()
        }
        // api.post("/api/orders", x) in a comment is not code
        pub async fn commented() {}
    "#;
    assert_eq!(promoted_writers(planted), vec!["sneaky", "built_elsewhere"]);
}

#[test]
fn no_tool_writes_the_promoted_set_except_the_two_that_are_gated_or_reduce_exposure() {
    let mut files = Vec::new();
    rs_files(&root().join("crates/mcp-server/src/tools"), &mut files);
    assert!(!files.is_empty(), "the tools directory was not found");
    let mut found: Vec<(String, String)> = Vec::new();
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap_or_default();
        for name in promoted_writers(&src) {
            found.push((f.file_name().unwrap().to_string_lossy().into_owned(), name));
        }
    }
    for (file, name) in &found {
        assert!(
            ALLOWED.iter().any(|(n, _)| n == name),
            "`{name}` in {file} writes a path in the promoted set (what a trade, an order or a \
             limit reads). An agent token reaches that set only through an approval envelope or \
             the proposal door (AGENT-013 §4, §7.5). If this tool is meant to, add it to ALLOWED \
             with the reason; otherwise route it through `lifecycle.propose`."
        );
    }
    // And the allow-list is not stale: each entry is still a writer, or it is dead weight
    // that would quietly license a future function of the same name.
    for (name, _why) in ALLOWED {
        assert!(
            found.iter().any(|(_, n)| n == name),
            "`{name}` is on ALLOWED and no longer writes the promoted set; remove it"
        );
    }
}

#[test]
fn the_allowed_writers_are_gated_or_say_why_not() {
    let cat = mcp_server_lib::spec::catalog();
    let spec = |n: &str| {
        cat.iter()
            .find(|s| s.name == n)
            .unwrap_or_else(|| panic!("`{n}` is not in the catalogue"))
    };
    for gated in ["arm_automation", "promote_model_version"] {
        assert_eq!(
            spec(gated).envelope,
            mcp_server_lib::spec::Envelope::Promotion,
            "`{gated}` writes the promoted set and must carry Promotion"
        );
        assert_eq!(spec(gated).ring(), mcp_server_lib::spec::RING_PAPER);
    }
    // The exception, stated so changing it is a decision: disarming only reduces what the
    // platform will do with money.
    let disarm = spec("disarm_automation");
    assert_eq!(disarm.envelope, mcp_server_lib::spec::Envelope::None);
}
