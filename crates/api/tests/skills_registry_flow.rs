//! The skill registry end to end, against a scratch PostgreSQL (Set R-a, Set R-b).
//!
//! Live Postgres; ignored by default. It builds a scratch database, applies the
//! migrations it needs (0122 and 0125 with `embedding` as `bytea` when pgvector is
//! absent, then 0210 to 0214 and 0225), runs every flow through the same functions the
//! routes and the agent's session start call, and drops the database at the end:
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test skills_registry_flow -- --ignored --test-threads 1
//! ```
//!
//! Tests connect as the owner, which cannot see a missing GRANT, so the platform's role
//! is exercised with `SET LOCAL ROLE app_role` (the pattern of `pg_universe.rs`).

use std::collections::BTreeMap;

use api::agent::skills_mount::{pending_notices, session_start};
use api::agent::workspace_tools::{activated_skill, execute, workspace_for};
use api::skills_store::SkillStore;
use api::skills_verify::{admit_proposal, propose, record_step, verify, ProposalInput};
use harness::memory::{DataWindow, Entry, Provenance};
use serde_json::json;
use skills::lifecycle::{check_promotion, Condition, Pair};
use skills::materialiser::{Scope, Status};
use skills::registry::{Catalogue, Disposition, Draft};
use sqlx::{Connection, Executor, PgConnection, PgPool, Row};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL points at a scratch Postgres")
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

fn migration(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("migrations")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

struct Scratch {
    name: String,
    pg: PgPool,
}

impl Scratch {
    async fn new() -> Self {
        let name = format!("skills_flow_{}", &Uuid::new_v4().simple().to_string()[..8]);
        let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
        admin.execute(format!("CREATE DATABASE {name}").as_str()).await.unwrap();
        let pg = PgPool::connect(&with_db(&base_url(), &name)).await.unwrap();
        pg.execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') \
             THEN CREATE ROLE app_role NOLOGIN; END IF; END $$;",
        )
        .await
        .unwrap();
        pg.execute("CREATE EXTENSION IF NOT EXISTS pgcrypto").await.ok();
        let has_vector = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_available_extensions WHERE name = 'vector'")
            .fetch_one(&pg)
            .await
            .unwrap_or(0)
            > 0;
        let mut mem = migration("0122_semantic_memory.sql");
        if has_vector {
            pg.execute("CREATE EXTENSION IF NOT EXISTS vector").await.unwrap();
        } else {
            mem = mem.replace("embedding       vector(768),", "embedding       bytea,");
        }
        pg.execute(mem.as_str()).await.expect("0122 applies");
        for m in [
            "0125_negative_result_class.sql",
            "0210_skill_registry.sql",
            "0211_skill_registry_store.sql",
            "0212_skill_evaluation.sql",
            "0213_move_memory.sql",
            "0214_regime_strategy_prior.sql",
            "0225_campaign_arms_and_curator.sql",
        ] {
            pg.execute(migration(m).as_str()).await.unwrap_or_else(|e| panic!("{m}: {e}"));
        }
        Self { name, pg }
    }

    async fn drop(self) {
        self.pg.close().await;
        let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.unwrap();
        let _ = admin.execute(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name).as_str()).await;
    }
}

fn catalogue() -> Catalogue {
    api::skills_store::tool_catalogue()
}

fn bundle(name: &str, description: &str, body: &str) -> BTreeMap<String, Vec<u8>> {
    let mut f = BTreeMap::new();
    f.insert(
        "SKILL.md".to_string(),
        format!("---\nname: {name}\ndescription: {description}\n---\n{body}").into_bytes(),
    );
    f
}

fn finding(id: &str, subject: &str, claim: &str, run: &str) -> Entry {
    let mut e = Entry::finding(id, subject, claim, DataWindow::new("2026-01-01", "2026-02-01"), Provenance::new(run, "run_backtest", "3", "hash"));
    e.created_by = "test".into();
    e
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("skills_flow_{tag}_{}", Uuid::new_v4().simple()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn clean_up(p: &std::path::Path) {
    skills::mount::clear_readonly(p);
    let _ = std::fs::remove_dir_all(p);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn session_start_mounts_core_records_exposure_once_and_a_read_activates() {
    let db = Scratch::new().await;
    let root = tmp("mount");
    let (user, session) = (Uuid::new_v4(), Uuid::new_v4());
    let ws = workspace_for(&root, Uuid::new_v4());

    let s = session_start(&db.pg, &ws, user, session, "test-model").await;
    assert_eq!(s.degraded, None, "{s:?}");
    for want in ["playbook", "skill-finder", "skill-authoring", "tbot-howto", "qc-rules-and-pitfalls"] {
        assert!(s.resident.iter().any(|n| n == want), "{want} not resident: {:?}", s.resident);
        assert!(root.join("agents").read_dir().is_ok());
    }
    let mount = ws.root().join(".claude/skills");
    assert!(mount.join("playbook/SKILL.md").exists());
    assert!(mount.join("skill-finder/GLOSSARY.md").exists());
    assert!(s.hint.contains("Resident skills"));

    // One exposure per resident skill; asking again for the same session adds none.
    let n: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_run WHERE session_id = $1 AND state = 'exposed'")
        .bind(session)
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    assert_eq!(usize::try_from(n).unwrap(), s.resident.len());
    let again = session_start(&db.pg, &ws, user, session, "test-model").await;
    assert_eq!(again.degraded, None);
    let n2: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_run WHERE session_id = $1")
        .bind(session)
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, n2, "a repeat session start must not double-count exposures");

    // The agent reads the mounted SKILL.md through its own tool: that is the activation.
    let args = serde_json::from_value(json!({ "path": ".claude/skills/playbook/SKILL.md" })).unwrap();
    let (text, is_error) = execute(&ws, "read_file", &args).await;
    assert!(!is_error && text.contains("Playbook"), "{text}");
    assert_eq!(activated_skill(".claude/skills/playbook/SKILL.md").as_deref(), Some("playbook"));
    let store = SkillStore::new(db.pg.clone());
    assert!(store.record_activation(session, "playbook").await.unwrap());
    assert!(!store.record_activation(session, "playbook").await.unwrap(), "once per session");
    let id: Uuid = sqlx::query("SELECT skill_id FROM skill_registry.skill WHERE name = 'playbook'")
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    let st = store.stats(id).await.unwrap();
    assert_eq!((st.exposures, st.activations), (1, 1));

    // And it cannot write there: the mount is not one of the five writable areas.
    let w = serde_json::from_value(json!({ "path": ".claude/skills/playbook/SKILL.md", "content": "x" })).unwrap();
    let (msg, err) = execute(&ws, "write_file", &w).await;
    assert!(err, "{msg}");

    // The run's bill of materials is read from skill_run alone.
    let bom = store.bill_of_materials(session).await.unwrap();
    assert_eq!(bom.len(), s.resident.len());
    assert!(bom.iter().any(|b| b["name"] == "playbook" && b["activated"] == true));

    clean_up(&root);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn core_sync_is_idempotent_and_a_changed_bundle_is_a_new_version_that_leaves_the_old_readable() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let dir = api::agent::skills_mount::core_dir().expect("skills/core is found");
    let core = skills::core::load_core_dir(&dir).unwrap();
    let r1 = store.sync_core(&core).await.unwrap();
    assert_eq!((r1.admitted, r1.patched, r1.unchanged), (core.len(), 0, 0));
    let r2 = store.sync_core(&core).await.unwrap();
    assert_eq!((r2.admitted, r2.patched, r2.unchanged), (0, 0, core.len()), "loading twice is a no-op");

    // A changed SKILL.md is a second version, never an edit.
    let mut changed = core.clone();
    let pb = changed.iter_mut().find(|c| c.name == "playbook").unwrap();
    let mut files = pb.files.clone();
    files.insert("SKILL.md".into(), format!("{}\nAn added line.\n", String::from_utf8_lossy(&files["SKILL.md"])).into_bytes());
    *pb = skills::core::validate(&pb.category, "playbook", files).unwrap();
    let old_digest = core.iter().find(|c| c.name == "playbook").unwrap().digest.clone();
    let r3 = store.sync_core(&changed).await.unwrap();
    assert_eq!(r3.patched, 1);
    let versions: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_version v JOIN skill_registry.skill s USING (skill_id) WHERE s.name = 'playbook'")
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    assert_eq!(versions, 2);
    assert!(store.bundle(&old_digest).await.unwrap().is_some(), "the first version stays readable");
    // An admitted delta above a version this release already shipped is not overwritten by
    // the release loading the older bytes again (KM-05).
    let r4 = store.sync_core(&core).await.unwrap();
    assert_eq!(r4.patched, 0, "{r4:?}");
    let cur: String = sqlx::query("SELECT current_version FROM skill_registry.skill WHERE name = 'playbook'")
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    assert_ne!(cur, old_digest);
    // Every core skill is in the glossary view.
    let g: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_glossary WHERE scope = 'core'")
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    assert_eq!(usize::try_from(g).unwrap(), core.len());
    db.drop().await;
}

const GOOD_BODY: &str = "Check the cost model before trusting a backtest. Read the fee schedule, \
    compare the modelled spread with the realised one, and report the gap.\n";

fn input(user: Uuid, project: Uuid, name: &str, body: &str, source_ref: &str) -> ProposalInput {
    ProposalInput {
        user,
        project,
        name: name.into(),
        category: "research".into(),
        reason: "T2".into(),
        source_ref: source_ref.into(),
        sources: vec![],
        files: bundle(name, "use when a backtest result needs its cost model checked", body),
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn propose_verify_and_admit_with_every_refusal_naming_its_step() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let core = skills::core::load_core_dir(&api::agent::skills_mount::core_dir().unwrap()).unwrap();
    store.sync_core(&core).await.unwrap();
    let (user, project) = (Uuid::new_v4(), Uuid::new_v4());

    // A speculative proposal (no trajectory) and an untrusted-only one are refused, and
    // nothing is written for either.
    let e = propose(&db.pg, input(user, project, "cost-check", GOOD_BODY, "  ")).await.unwrap_err();
    assert!(e.to_string().contains("trajectory"), "{e}");
    let mut untrusted = input(user, project, "cost-check", GOOD_BODY, "run:1");
    untrusted.sources = vec!["web_text".into()];
    assert!(propose(&db.pg, untrusted).await.is_err());
    let n: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_proposal").fetch_one(&db.pg).await.unwrap().get(0);
    assert_eq!(n, 0);

    // The static lint rejects a ticker and a fitted constant, naming the step and the line.
    let bad = propose(&db.pg, input(user, project, "bad-skill", "Buy BTC-USD when x crosses 0.7312.\n", "run:1")).await.unwrap();
    assert_eq!(bad.status, "rejected");
    let lint = bad.verdicts.iter().find(|v| v["step"] == "lint").unwrap();
    assert_eq!(lint["passed"], json!(false));
    assert!(lint["detail"]["findings"].as_array().unwrap().len() >= 2, "{lint}");

    // A clean proposal is submitted; admission is refused until every step has passed.
    let ok = propose(&db.pg, input(user, project, "cost-check", GOOD_BODY, "run:abc")).await.unwrap();
    assert_eq!(ok.status, "submitted");
    assert_eq!(ok.disposition, Disposition::New);
    let refused = admit_proposal(&db.pg, ok.proposal_id, "admin").await.unwrap_err();
    assert!(refused.to_string().contains("`replay`"), "{refused}");

    // The harness records paired outcomes (3 variants); the verify step computes the replay verdict.
    for (i, variant) in ["source", "v1", "v2"].iter().enumerate() {
        for _ in 0..3 {
            store
                .record_pair(
                    Some(ok.proposal_id),
                    None,
                    "replay",
                    "m",
                    variant,
                    &Pair { activated: true, with_success: true, without_success: i == 99, with_cost: 80.0, without_cost: 100.0 },
                )
                .await
                .unwrap();
        }
    }
    let v = verify(&db.pg, ok.proposal_id).await.unwrap();
    assert_eq!(v["first_unmet"], json!("reviewer"), "{v}");
    record_step(&db.pg, ok.proposal_id, "reviewer", true, json!({ "by": "isolated-reviewer" })).await.unwrap();
    let v = verify(&db.pg, ok.proposal_id).await.unwrap();
    assert_eq!(v["admissible"], json!(true), "{v}");

    let skill = admit_proposal(&db.pg, ok.proposal_id, "admin").await.unwrap();
    let st: String = sqlx::query("SELECT status FROM skill_registry.skill WHERE skill_id = $1").bind(skill).fetch_one(&db.pg).await.unwrap().get(0);
    assert_eq!(st, "provisional");
    // Visible to the project, in the glossary view, and frozen into its selection.
    let vis = store.visible(user, Some(project)).await.unwrap();
    assert!(vis.iter().any(|s| s.name == "cost-check" && s.scope == Scope::Project));
    assert!(!store.visible(user, None).await.unwrap().iter().any(|s| s.name == "cost-check"), "other projects do not see it");
    assert!(store.frozen_selection(Some(project)).await.unwrap().contains(&"cost-check".to_string()));
    assert!(store.bundle(&vis.iter().find(|s| s.name == "cost-check").unwrap().version).await.unwrap().is_some());

    // A near-duplicate (a lightly edited copy) becomes a patch of the existing skill, and the
    // proposed name is rewritten to the target's.
    let dup = propose(&db.pg, input(user, project, "cost-check-v2", &format!("{GOOD_BODY}Then record it.\n"), "run:def")).await.unwrap();
    assert!(matches!(dup.disposition, Disposition::Patch { ref target, .. } if target == "cost-check"), "{:?}", dup.disposition);
    // The same name is an explicit patch.
    let same = propose(&db.pg, input(user, project, "cost-check", "A different procedure about position sizing and volatility targets.\n", "run:ghi")).await.unwrap();
    assert!(matches!(same.disposition, Disposition::Patch { .. }));

    // A proposal may not take a core name and win: it is a patch of the core skill, which only a
    // person admits.
    let core_patch = propose(&db.pg, input(user, project, "playbook", "Another playbook.\n", "run:jkl")).await.unwrap();
    assert!(matches!(core_patch.disposition, Disposition::Patch { ref target, .. } if target == "playbook"));

    // KM-05: the playbook changes only through an admitted delta. Verified and admitted, the
    // patch is a new version of the *core* skill whose parent is the old one; the old version
    // stays readable, no project selection is touched, and the release loading its own older
    // bytes again does not overwrite the delta.
    let before: String = sqlx::query("SELECT current_version FROM skill_registry.skill WHERE name = 'playbook'").fetch_one(&db.pg).await.unwrap().get(0);
    for (i, variant) in ["source", "v1", "v2"].iter().enumerate() {
        for _ in 0..3 {
            store
                .record_pair(Some(core_patch.proposal_id), None, "replay", "m", variant, &Pair { activated: true, with_success: true, without_success: i == 99, with_cost: 1.0, without_cost: 1.0 })
                .await
                .unwrap();
        }
    }
    verify(&db.pg, core_patch.proposal_id).await.unwrap();
    record_step(&db.pg, core_patch.proposal_id, "reviewer", true, json!({})).await.unwrap();
    admit_proposal(&db.pg, core_patch.proposal_id, "admin").await.unwrap();
    let (after, scope): (String, String) = {
        let r = sqlx::query("SELECT current_version, scope FROM skill_registry.skill WHERE name = 'playbook'").fetch_one(&db.pg).await.unwrap();
        (r.get(0), r.get(1))
    };
    assert_ne!(after, before);
    assert_eq!(scope, "core", "a patch changes the version, not the skill's scope");
    let parent: Option<String> = sqlx::query("SELECT parent_version FROM skill_registry.skill_version WHERE version_hash = $1").bind(&after).fetch_one(&db.pg).await.unwrap().get(0);
    assert_eq!(parent.as_deref(), Some(before.as_str()));
    assert!(store.bundle(&before).await.unwrap().is_some());
    let r = store.sync_core(&core).await.unwrap();
    assert_eq!(r.patched, 0, "the release's older bytes do not overwrite an admitted delta");
    let still: String = sqlx::query("SELECT current_version FROM skill_registry.skill WHERE name = 'playbook'").fetch_one(&db.pg).await.unwrap().get(0);
    assert_eq!(still, after);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn provisional_becomes_admitted_only_at_n_exposures_with_a_non_negative_balance() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let draft = Draft::check(Scope::Global, "research", "ramp-skill", bundle("ramp-skill", "use when ramping", "Steps.\n"), &catalogue()).unwrap();
    let id = store.admit(&draft, Status::Provisional, None, None, false, None, "admin").await.unwrap();
    let rows = |_sess: Uuid| skills::materialiser::ExposureRow { skill: "ramp-skill".into(), version: draft.version_hash.clone() };
    for i in 0..19 {
        let s = Uuid::new_v4();
        store.record_exposures(&[rows(s)], s, None).await.unwrap();
        let _ = i;
    }
    let (promoted, _) = store.settle_provisional(20).await.unwrap();
    assert!(promoted.is_empty(), "19 exposures is below N");
    let s20 = Uuid::new_v4();
    store.record_exposures(&[rows(s20)], s20, None).await.unwrap();
    // A negative vote at N keeps it provisional and flags it.
    store.vote(id, s20, -1).await.unwrap();
    let (promoted, flagged) = store.settle_provisional(20).await.unwrap();
    assert!(promoted.is_empty() && flagged == vec!["ramp-skill".to_string()]);
    store.vote(id, s20, 1).await.unwrap();
    let (promoted, _) = store.settle_provisional(20).await.unwrap();
    assert_eq!(promoted, vec!["ramp-skill".to_string()]);
    // A vote on a skill the session never saw is refused.
    assert!(store.vote(id, Uuid::new_v4(), 1).await.is_err());
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn quarantine_leaves_every_glossary_in_one_statement_and_notifies_live_sessions() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let draft = Draft::check(Scope::Global, "research", "shared-skill", bundle("shared-skill", "use when sharing", "Steps.\n"), &catalogue()).unwrap();
    let id = store.admit(&draft, Status::Admitted, None, None, true, None, "admin").await.unwrap();
    let (u1, u2) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(store.visible(u1, None).await.unwrap().iter().any(|s| s.name == "shared-skill"));
    let cursor = store.latest_notice().await.unwrap();
    assert!(store.retire(id, "quarantined", "").await.is_err(), "a reason is required");
    store.retire(id, "quarantined", "found to leak a ticker").await.unwrap();
    for u in [u1, u2] {
        assert!(!store.visible(u, None).await.unwrap().iter().any(|s| s.name == "shared-skill"), "gone for everyone at once");
    }
    let (next, msgs) = pending_notices(&db.pg, cursor, &["shared-skill".to_string()]).await;
    assert!(next > cursor);
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].contains("shared-skill") && msgs[0].contains("quarantined"));
    // A session that did not have it resident is not told.
    let (_, none) = pending_notices(&db.pg, cursor, &["playbook".to_string()]).await;
    assert!(none.is_empty());
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn promotion_to_global_needs_six_recorded_conditions_and_a_tainted_finding_blocks_it() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let mem = api::memory_store::MemoryStore::new(db.pg.clone());
    let owner = Uuid::new_v4();
    let project = Uuid::new_v4();
    let draft = Draft::check(Scope::Project, "research", "promo-skill", bundle("promo-skill", "use when promoting", "Steps.\n"), &catalogue()).unwrap();
    let id = store.admit(&draft, Status::Admitted, None, Some(project), false, None, "agent-7").await.unwrap();

    let all = [Condition::CrossProjectEvals, Condition::Sanitiser, Condition::OwnerConsent, Condition::SecurityScan, Condition::IndependentReviewer, Condition::NoTaint];
    for c in &all[..5] {
        store.record_condition(id, *c, true, if *c == Condition::IndependentReviewer { "mason" } else { "platform" }, &json!({})).await.unwrap();
    }
    let refused = check_promotion(&store.promotion_file(id).await.unwrap()).unwrap_err();
    assert_eq!(refused.0.iter().map(|(c, _)| *c).collect::<Vec<_>>(), vec![Condition::NoTaint], "names the one missing");

    // A finding derived from a fetched page is tainted; citing it taints the skill's provenance
    // whatever the caller says, and promotion is refused even with all six recorded.
    let clean = finding("fnd_clean", "BTC-USD", "a clean claim", "r1");
    let dirty = finding("fnd_dirty", "BTC-USD", "a claim from a page", "r2").mark_tainted();
    mem.put(owner, &clean).await.unwrap();
    mem.put(owner, &dirty).await.unwrap();
    assert!(!store.cite_finding(id, owner, "fnd_clean").await.unwrap());
    assert!(store.cite_finding(id, owner, "fnd_dirty").await.unwrap());
    assert!(store.cite_finding(id, owner, "nonexistent").await.is_err());
    store.record_condition(id, Condition::NoTaint, true, "platform", &json!({})).await.unwrap();
    let refused = check_promotion(&store.promotion_file(id).await.unwrap()).unwrap_err();
    assert!(refused.0.iter().any(|(c, why)| *c == Condition::NoTaint && why.contains("tainted")), "{refused}");

    // The author cannot be the reviewer.
    db.pg.execute("DELETE FROM skill_registry.skill_provenance").await.unwrap();
    store.record_condition(id, Condition::IndependentReviewer, true, "agent-7", &json!({})).await.unwrap();
    let refused = check_promotion(&store.promotion_file(id).await.unwrap()).unwrap_err();
    assert!(refused.0.iter().any(|(c, _)| *c == Condition::IndependentReviewer));
    store.record_condition(id, Condition::IndependentReviewer, true, "mason", &json!({})).await.unwrap();
    check_promotion(&store.promotion_file(id).await.unwrap()).expect("all six hold");

    // The move is logged with who approved it.
    store.move_scope(id, "global", owner, owner).await.unwrap();
    let scope: String = sqlx::query("SELECT scope FROM skill_registry.skill WHERE skill_id = $1").bind(id).fetch_one(&db.pg).await.unwrap().get(0);
    assert_eq!(scope, "global");
    let logged: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_promotion_log WHERE skill_id = $1").bind(id).fetch_one(&db.pg).await.unwrap().get(0);
    assert_eq!(logged, 1);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn the_default_retrieval_excludes_tainted_entries_and_the_browser_still_sees_them() {
    let db = Scratch::new().await;
    let mem = api::memory_store::MemoryStore::new(db.pg.clone());
    let user = Uuid::new_v4();
    let ok = finding("fnd_ok", "ETH-USD", "an ordinary claim", "r1");
    let tainted = finding("fnd_tn", "ETH-USD", "read off a fetched page", "r2").mark_tainted();
    mem.put(user, &ok).await.unwrap();
    mem.put(user, &tainted).await.unwrap();
    let default = mem.for_subject(user, "ETH-USD", 10).await.unwrap();
    assert_eq!(default.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["fnd_ok"]);
    let all = mem.for_subject_including_tainted(user, "ETH-USD", 10).await.unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().find(|e| e.id == "fnd_tn").unwrap().is_tainted());
    assert!(!all.iter().find(|e| e.id == "fnd_ok").unwrap().is_tainted());
    let by_kind = mem.by_kind(user, harness::memory::Kind::Finding, 10).await.unwrap();
    assert_eq!(by_kind.len(), 1);
    assert_eq!(mem.by_kind_including_tainted(user, harness::memory::Kind::Finding, 10).await.unwrap().len(), 2);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn app_role_can_do_what_the_platform_does_and_nothing_that_would_bypass_admission() {
    let db = Scratch::new().await;
    // The owner admits a skill; app_role then acts on it.
    let store = SkillStore::new(db.pg.clone());
    let draft = Draft::check(Scope::Global, "research", "granted", bundle("granted", "use when granted", "Steps.\n"), &catalogue()).unwrap();
    let id = store.admit(&draft, Status::Admitted, None, None, false, None, "admin").await.unwrap();

    let mut tx = db.pg.begin().await.unwrap();
    tx.execute("SET LOCAL ROLE app_role").await.unwrap();
    // The one way in works as app_role (SECURITY DEFINER).
    let draft2 = Draft::check(Scope::Global, "research", "granted-two", bundle("granted-two", "use when granted two", "Steps.\n"), &catalogue()).unwrap();
    sqlx::query("SELECT skill_registry.admit($1, 'global', NULL, NULL, 'research', 'admitted', FALSE, $2, NULL, 'use when', 3, 1, $2, 'platform')")
        .bind("granted-two")
        .bind(&draft2.version_hash)
        .execute(&mut *tx)
        .await
        .expect("app_role can execute admit()");
    sqlx::query("INSERT INTO skill_registry.skill_bundle (version_hash, files) VALUES ($1, '{\"SKILL.md\": \"x\"}')")
        .bind(&draft2.version_hash)
        .execute(&mut *tx)
        .await
        .expect("the bundle is inserted in the admitting transaction");
    tx.commit().await.ok(); // deferred FKs are satisfied by the matching insert above

    // Every direct write path around admit() is a permission error.
    for sql in [
        "INSERT INTO skill_registry.skill (name, scope, category, status, current_version) VALUES ('sneaky', 'global', 'x', 'admitted', 'a')",
        "UPDATE skill_registry.skill SET scope = 'core' WHERE name = 'granted'",
        "INSERT INTO skill_registry.skill_version (version_hash, skill_id, description, description_tokens, body_lines, bundle_digest, created_by) \
         SELECT repeat('a', 64), skill_id, 'd', 1, 1, repeat('a', 64), 'x' FROM skill_registry.skill LIMIT 1",
        "UPDATE skill_registry.skill_bundle SET files = '{}'",
        "DELETE FROM skill_registry.skill_notice",
        "UPDATE agent_memory.move_record SET outcome = 'improved'",
        "DELETE FROM agent_memory.hypothesis_source",
        "UPDATE agent_memory.campaign_arm_score SET score = 1",
    ] {
        let mut tx = db.pg.begin().await.unwrap();
        tx.execute("SET LOCAL ROLE app_role").await.unwrap();
        let r = sqlx::query(sql).execute(&mut *tx).await;
        assert!(r.is_err(), "app_role must not be able to: {sql}");
    }

    // And everything the platform does works: reads, exposures, votes, notices, verdicts, pairs.
    let mut tx = db.pg.begin().await.unwrap();
    tx.execute("SET LOCAL ROLE app_role").await.unwrap();
    for sql in [
        "SELECT count(*) FROM skill_registry.skill_glossary",
        "SELECT count(*) FROM skill_registry.skill_verdict",
        "SELECT count(*) FROM skill_registry.skill_eval_pair",
        "SELECT count(*) FROM skill_registry.skill_promotion_condition",
        "SELECT count(*) FROM skill_registry.skill_library_reeval",
        "SELECT count(*) FROM skill_registry.curator_item",
        "SELECT count(*) FROM agent_memory.move_record",
        "SELECT count(*) FROM agent_memory.campaign_arm",
        "SELECT count(*) FROM regime_causal.regime_strategy_prior",
        "SELECT count(*) FROM regime_causal.regime_jump_model",
    ] {
        sqlx::query(sql).fetch_one(&mut *tx).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    sqlx::query("INSERT INTO skill_registry.skill_run (skill_id, version_hash, state) SELECT skill_id, current_version, 'exposed' FROM skill_registry.skill WHERE name = 'granted'")
        .execute(&mut *tx)
        .await
        .expect("app_role records an exposure");
    sqlx::query("SELECT skill_registry.retire($1, 'quarantined', 'because')").bind(id).execute(&mut *tx).await.expect("app_role can quarantine through retire()");
    tx.rollback().await.unwrap();
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn the_move_veto_the_hypothesis_sources_and_the_random_arm_keep_their_records() {
    use research::move_memory::{motif, veto_move, Outcome, VetoPolicy};
    use research::propose::{ExitMode, Move};

    let db = Scratch::new().await;
    let user = Uuid::new_v4();
    let store = api::knowledge_routes::MoveStore::new(db.pg.clone());
    let m = Move::ChangeExit { mode: ExitMode::Trail };
    for i in 0..5 {
        store.record(user, "BTC-USD", &m, Outcome::Failed, &format!("camp:{i}")).await.unwrap();
    }
    let history = store.history(user, &motif(&m)).await.unwrap();
    assert_eq!(history.len(), 5);
    let v = veto_move(&m, &history, &VetoPolicy::default());
    assert!(v.blocks(), "{v:?}");
    assert!(!store.overridden(user, &motif(&m)).await.unwrap());
    sqlx::query("INSERT INTO agent_memory.move_veto_override (user_id, motif, reason) VALUES ($1, $2, 'a person decided')")
        .bind(user)
        .bind(motif(&m))
        .execute(&db.pg)
        .await
        .unwrap();
    assert!(store.overridden(user, &motif(&m)).await.unwrap());
    // Another user's veto is not this user's.
    assert!(store.history(Uuid::new_v4(), &motif(&m)).await.unwrap().is_empty());
    // The record is append-only even for the owner.
    assert!(sqlx::query("UPDATE agent_memory.move_record SET outcome = 'improved'").execute(&db.pg).await.is_err());

    // The arm: created once, with the platform's seed; incomplete arms refuse the gate.
    let arms = api::knowledge_routes::ArmStore::new(db.pg.clone());
    let camp = Uuid::new_v4();
    let seed = arms.create(camp, user, 3).await.unwrap();
    assert_eq!(seed, research::random_arm::arm_seed(&camp.to_string()));
    assert!(arms.create(camp, user, 3).await.is_err(), "one arm per campaign");
    assert!(api::knowledge_routes::arm_gate(&db.pg, camp).await.is_err());
    assert!(api::knowledge_routes::arm_gate(&db.pg, Uuid::new_v4()).await.unwrap_err().contains("no random arm"));
    for (seq, s) in [0.1, 0.2, 0.15].into_iter().enumerate() {
        arms.record_score(camp, "random", i32::try_from(seq).unwrap(), s).await.unwrap();
        arms.record_score(camp, "agent", i32::try_from(seq).unwrap(), s * 3.0).await.unwrap();
    }
    api::knowledge_routes::arm_gate(&db.pg, camp).await.expect("a completed arm passes");
    let (arm, agent) = arms.load(camp).await.unwrap().unwrap();
    let c = research::random_arm::compare_to_random(&agent, Some(&arm)).unwrap();
    assert!(c.percentile > 0.9, "{}", c.percentile);

    // A repeated hypothesis source is stored with its count; the table is append-only.
    sqlx::query(
        "INSERT INTO agent_memory.hypothesis_source (hypothesis_id, user_id, source_kind, finding_id, source_text, instrument, window_start, window_end, repeats) \
         VALUES ($1, $2, 'memory', 'fnd_1', 'memory:fnd_1', 'BTC-USD', '2026-01-01', '2026-03-01', 0)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .execute(&db.pg)
    .await
    .unwrap();
    // A memory source must name a finding (schema CHECK).
    assert!(sqlx::query(
        "INSERT INTO agent_memory.hypothesis_source (hypothesis_id, user_id, source_kind, source_text, instrument, window_start, window_end) \
         VALUES ($1, $2, 'memory', 'memory:', 'BTC-USD', '2026-01-01', '2026-03-01')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .execute(&db.pg)
    .await
    .is_err());

    // The regime prior is a bandit table: a study updates exactly its row.
    let prior = api::knowledge_routes::PriorStore::new(db.pg.clone());
    prior.update("t", "BTC-USD", "high_vol", "trend", 1.2, true).await.unwrap();
    prior.update("t", "BTC-USD", "high_vol", "trend", 0.8, true).await.unwrap();
    prior.update("t", "BTC-USD", "low_vol", "trend", -0.3, false).await.unwrap();
    let cells = prior.cells("t", "BTC-USD", "high_vol").await.unwrap();
    assert_eq!(cells["trend"].n_obs, 2);
    assert!((cells["trend"].mean().unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(prior.cells("t", "BTC-USD", "low_vol").await.unwrap()["trend"].n_success, 0);
    assert!(prior.update("t", "BTC-USD", "bubble", "trend", 1.0, true).await.is_err(), "the CHECK refuses an unknown regime");
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn retirement_and_the_model_guard_act_on_recorded_evidence() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let dead = Draft::check(Scope::Global, "research", "never-used", bundle("never-used", "use when never", "Steps.\n"), &catalogue()).unwrap();
    let useful = Draft::check(Scope::Global, "research", "used-skill", bundle("used-skill", "use when used", "Other steps.\n"), &catalogue()).unwrap();
    let dead_id = store.admit(&dead, Status::Admitted, None, None, false, None, "a").await.unwrap();
    let useful_id = store.admit(&useful, Status::Admitted, None, None, false, None, "a").await.unwrap();
    for d in [&dead, &useful] {
        for _ in 0..20 {
            let s = Uuid::new_v4();
            store
                .record_exposures(&[skills::materialiser::ExposureRow { skill: if std::ptr::eq(d, &dead) { "never-used".into() } else { "used-skill".into() }, version: d.version_hash.clone() }], s, None)
                .await
                .unwrap();
            if std::ptr::eq(d, &useful) {
                store.record_activation(s, "used-skill").await.unwrap();
            }
        }
    }
    let archived = store.sweep_retirements(20).await.unwrap();
    assert_eq!(archived.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["never-used"]);
    assert!(!store.visible(Uuid::new_v4(), None).await.unwrap().iter().any(|s| s.name == "never-used"), "an archived skill is not resident");
    let _ = dead_id;

    // Leave-one-out contribution of zero retires an activated skill.
    for _ in 0..4 {
        store
            .record_pair(None, Some(useful_id), "leave_one_out", "m", "v", &Pair { activated: true, with_success: true, without_success: true, with_cost: 1.0, without_cost: 1.0 })
            .await
            .unwrap();
    }
    assert_eq!(store.loo_contribution(useful_id).await.unwrap(), Some(0.0));
    let archived = store.sweep_retirements(20).await.unwrap();
    assert_eq!(archived.len(), 1);

    // The model guard: no completed re-evaluation, no trust in the library on that model.
    assert!(skills::lifecycle::check_model_change("new-model", &store.reevaluations("new-model").await.unwrap()).is_err());
    store.record_reevaluation("new-model", true, &["used-skill".to_string()]).await.unwrap();
    let failing = skills::lifecycle::check_model_change("new-model", &store.reevaluations("new-model").await.unwrap()).unwrap();
    assert_eq!(failing, vec!["used-skill".to_string()]);
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn a_deprecated_skill_resolves_through_alias_of_and_only_a_deprecated_one_can_be_an_alias() {
    let db = Scratch::new().await;
    let store = SkillStore::new(db.pg.clone());
    let old = Draft::check(Scope::Global, "research", "old-way", bundle("old-way", "use when old", "Old steps.\n"), &catalogue()).unwrap();
    let new = Draft::check(Scope::Global, "research", "new-way", bundle("new-way", "use when new", "New steps.\n"), &catalogue()).unwrap();
    let old_id = store.admit(&old, Status::Admitted, None, None, false, None, "a").await.unwrap();
    let new_id = store.admit(&new, Status::Admitted, None, None, false, None, "a").await.unwrap();
    // A live skill cannot be made an alias: it would vanish from the glossary behind another.
    assert!(store.set_alias(old_id, new_id).await.is_err());
    store.retire(old_id, "deprecated", "superseded").await.unwrap();
    store.set_alias(old_id, new_id).await.unwrap();
    let (rid, name, version) = store.resolve_alias(old_id).await.unwrap().expect("resolves");
    assert_eq!((rid, name.as_str()), (new_id, "new-way"));
    assert_eq!(version, new.version_hash);
    // The live skill resolves to itself; a deprecated skill with no alias resolves to nothing.
    assert_eq!(store.resolve_alias(new_id).await.unwrap().unwrap().0, new_id);
    let lone = Draft::check(Scope::Global, "research", "lone-skill", bundle("lone-skill", "use when lone", "Steps.\n"), &catalogue()).unwrap();
    let lone_id = store.admit(&lone, Status::Admitted, None, None, false, None, "a").await.unwrap();
    store.retire(lone_id, "archived", "unused").await.unwrap();
    assert!(store.resolve_alias(lone_id).await.unwrap().is_none());
    // The deprecated skill is not in any glossary.
    let vis = store.visible(Uuid::new_v4(), None).await.unwrap();
    assert!(vis.iter().all(|s| s.name != "old-way" && s.name != "lone-skill"));
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn a_stored_vector_names_its_model_and_dimension_and_a_legacy_row_stays_updatable() {
    let db = Scratch::new().await;
    // pgvector is not on the scratch server, so the insight table is the minimum 0226 touches.
    db.pg
        .execute(
            "CREATE SCHEMA IF NOT EXISTS knowledge;
             CREATE TABLE knowledge.insight (
               insight_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
               tenant_id TEXT NOT NULL, claim TEXT NOT NULL, decay_score REAL NOT NULL DEFAULT 1.0,
               embedding BYTEA);
             INSERT INTO knowledge.insight (tenant_id, claim, embedding) VALUES ('t', 'legacy', '\x01');",
        )
        .await
        .unwrap();
    db.pg.execute(migration("0226_embedding_model_and_dim.sql").as_str()).await.expect("0226 applies");
    db.pg.execute(migration("0226_embedding_model_and_dim.sql").as_str()).await.expect("and applies twice");

    // A vector with no model, or the wrong width, is refused; with both it is accepted.
    assert!(sqlx::query("INSERT INTO knowledge.insight (tenant_id, claim, embedding) VALUES ('t', 'a', '\x02')").execute(&db.pg).await.is_err());
    assert!(sqlx::query("INSERT INTO knowledge.insight (tenant_id, claim, embedding, embedding_model, embedding_dim) VALUES ('t', 'b', '\x02', 'nomic-embed-text', 512)").execute(&db.pg).await.is_err());
    sqlx::query("INSERT INTO knowledge.insight (tenant_id, claim, embedding, embedding_model, embedding_dim) VALUES ('t', 'c', '\x02', 'nomic-embed-text', 768)")
        .execute(&db.pg)
        .await
        .expect("a vector that names its model and width");
    // A row with no vector needs neither.
    sqlx::query("INSERT INTO knowledge.insight (tenant_id, claim) VALUES ('t', 'd')").execute(&db.pg).await.unwrap();

    // A legacy row (a vector, no model) can still have other columns updated: the rule fires
    // only when the embedding is written.
    sqlx::query("UPDATE knowledge.insight SET decay_score = 0.5 WHERE claim = 'legacy'").execute(&db.pg).await.expect("legacy rows stay updatable");
    // It is picked up by the model-change query (NULL differs from every model).
    let n: i64 = sqlx::query("SELECT count(*) FROM knowledge.insight WHERE embedding IS NULL OR embedding_model IS DISTINCT FROM 'nomic-embed-text'")
        .fetch_one(&db.pg)
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "the legacy row and the unembedded one");
    // Re-embedding it names the model.
    sqlx::query("UPDATE knowledge.insight SET embedding = '\x03', embedding_model = 'nomic-embed-text', embedding_dim = 768 WHERE claim = 'legacy'").execute(&db.pg).await.unwrap();
    db.drop().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch Postgres"]
async fn analog_search_over_stored_fingerprints_never_returns_the_same_episode_or_the_future() {
    use chrono::{Duration, TimeZone, Utc};
    let db = Scratch::new().await;
    // 0184 is a plain table (REAL[] vectors, no pgvector); its grants name two roles.
    db.pg
        .execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'agent_role') THEN CREATE ROLE agent_role NOLOGIN; END IF; \
             IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'internal_ml_role') THEN CREATE ROLE internal_ml_role NOLOGIN; END IF; END $$; \
             CREATE SCHEMA IF NOT EXISTS knowledge;",
        )
        .await
        .unwrap();
    db.pg.execute(migration("0184_instrument_fingerprint.sql").as_str()).await.expect("0184 applies");
    let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let dims = vec!["volatility".to_string(), "mean_return".to_string()];
    let put = |iid: i64, day: i64, v: [f32; 2]| {
        let pg = db.pg.clone();
        let dims = dims.clone();
        async move {
            sqlx::query(
                "INSERT INTO knowledge.instrument_fingerprint (instrument_id, venue_id, window_spec, dimension_order, tier1, bars_used, knowledge_time) \
                 VALUES ($1, 1, '21d@1m', $2, $3, 100, $4)",
            )
            .bind(iid)
            .bind(&dims)
            .bind(&v[..])
            .bind(t0 + Duration::days(day))
            .execute(&pg)
            .await
            .unwrap();
        }
    };
    // Instrument 1: its own latest window (day 100), a near-identical window 10 days later
    // (same episode in all but name), a similar one far away (day 250) and one in the future.
    put(1, 100, [1.0, 1.0]).await;
    put(1, 110, [1.0, 1.0]).await;
    put(1, 250, [1.2, 1.1]).await;
    put(1, 400, [1.0, 1.0]).await;
    // Instrument 2 at the same time as the query: a legitimate analog.
    put(2, 100, [1.1, 1.0]).await;
    put(2, 300, [5.0, 5.0]).await;
    let as_of = t0 + Duration::days(120);
    // 110 is after the query's window but inside as_of; the query is the latest fingerprint
    // of instrument 1 known by as_of, i.e. day 110.
    let out = api::knowledge_routes::analog_search(&db.pg, 1, 1, as_of, 10, "21d@1m").await.unwrap();
    let ids: Vec<String> = out["analogs"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect();
    let day = |d: i64| (t0 + Duration::days(d)).timestamp().div_euclid(86_400);
    assert!(!ids.contains(&format!("1:1:{}", day(110))), "the query itself");
    assert!(!ids.contains(&format!("1:1:{}", day(100))), "the same instrument 10 days earlier is inside the 21-day zone");
    assert!(!ids.contains(&format!("1:1:{}", day(250))), "known only after as_of");
    assert!(!ids.contains(&format!("1:1:{}", day(400))), "the future");
    assert!(ids.contains(&format!("2:1:{}", day(100))), "another market at the same time is an analog: {ids:?}");
    assert_eq!(out["exclusion_zone_days"], 21);
    assert_eq!(out["vector"], "explicit_standardised_fingerprint");
    assert_eq!(out["dimensions"], serde_json::json!(["volatility", "mean_return"]));
    // With a later as_of the far window of the same instrument appears.
    let later = api::knowledge_routes::analog_search(&db.pg, 1, 1, t0 + Duration::days(300), 10, "21d@1m").await.unwrap();
    let later_ids: Vec<String> = later["analogs"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect();
    // The query is now day 250; day 100 and 110 are far outside its zone.
    assert!(later_ids.contains(&format!("1:1:{}", day(110))), "{later_ids:?}");
    assert!(!later_ids.contains(&format!("1:1:{}", day(250))));
    // An instrument with no fingerprint is a typed absence.
    assert!(matches!(
        api::knowledge_routes::analog_search(&db.pg, 99, 1, as_of, 5, "21d@1m").await,
        Err(api::knowledge_routes::AnalogError::NoFingerprint)
    ));
    db.drop().await;
}
