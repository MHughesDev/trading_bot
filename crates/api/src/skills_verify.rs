//! Propose, verify and admit (Set R-a RA-3.1, RA-3.3, RA-3.7; AGENT-003 §6.3).
//!
//! The agent only *proposes*. A proposal always cites the trajectory it was distilled
//! from, is checked for near-duplicates (a close enough one becomes a patch of the
//! existing skill), and is verified step by step; each step is a row in
//! `skill_registry.skill_verdict` and admission reads those rows, so a skill failing
//! any step is not admitted and the verdict names the step.
//!
//! What runs here: the lint, the procedure check, the trigger check, the coexistence
//! check and the replay verdict (computed from the paired outcomes the evaluation
//! harness recorded). What does **not** run here, and is recorded as *missing* rather
//! than skipped: the Reviewer sub-agent's verdict and the paired runs themselves,
//! which need the job service and a model. A missing step is a failed step.
//!
//! A proposal is always **project-scope**: it originates in a project, and a wider
//! scope is reached only by promotion (`skills::lifecycle::check_promotion`).

use std::collections::BTreeMap;

use serde_json::{json, Value};
use skills::lifecycle::{replay_verdict, SequentialDesign};
use skills::materialiser::{Scope, Status};
use skills::registry::{
    body_of, check_proposal, decide, first_unmet_in, is_analysis_code, rename_bundle,
    required_steps, Disposition, Draft,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::skills_store::{files_from_json, files_to_json, patch_threshold, tool_catalogue, SkillStore, StoreError};

#[derive(Debug)]
pub struct ProposalInput {
    pub user: Uuid,
    pub project: Uuid,
    pub name: String,
    pub category: String,
    pub reason: String,
    pub source_ref: String,
    pub sources: Vec<String>,
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
pub struct ProposalOutcome {
    pub proposal_id: Uuid,
    pub status: &'static str,
    pub disposition: Disposition,
    pub similarity: f64,
    pub verdicts: Vec<Value>,
}

async fn record(pg: &PgPool, proposal: Uuid, step: &str, passed: bool, detail: Value) -> Result<Value, StoreError> {
    sqlx::query(
        "INSERT INTO skill_registry.skill_verdict (proposal_id, step, passed, detail) VALUES ($1, $2, $3, $4)",
    )
    .bind(proposal)
    .bind(step)
    .bind(passed)
    .bind(&detail)
    .execute(pg)
    .await?;
    Ok(json!({ "step": step, "passed": passed, "detail": detail }))
}

/// Existing skills this proposer can see, as `(name, body)`, for the near-duplicate
/// search.
async fn existing_bodies(store: &SkillStore, user: Uuid, project: Uuid) -> Result<Vec<(String, String)>, StoreError> {
    let vis = store.visible(user, Some(project)).await?;
    let refs: Vec<&skills::materialiser::RegistrySkill> = vis.iter().collect();
    let bundles = store.bundles_for(&refs).await?;
    Ok(bundles
        .into_iter()
        .map(|(n, f)| (n, body_of(&f)))
        .collect())
}

/// Submit a proposal (RA-3.1).
///
/// # Errors
/// [`StoreError::Refused`] for a proposal with no trajectory, a bad reason or only
/// untrusted sources (nothing is written: such a proposal never existed); the database.
pub async fn propose(pg: &PgPool, input: ProposalInput) -> Result<ProposalOutcome, StoreError> {
    check_proposal(&input.reason, &input.source_ref, &input.sources)
        .map_err(|e| StoreError::Refused(e.to_string()))?;
    let store = SkillStore::new(pg.clone());
    let existing = existing_bodies(&store, input.user, input.project).await?;

    // Same name as a visible project skill: an explicit patch of it.
    let vis = store.visible(input.user, Some(input.project)).await?;
    let same = vis.iter().find(|s| s.name == input.name);
    let mut files = input.files.clone();
    let body = body_of(&files);
    let (disposition, similarity) = match same {
        // A project skill of the same name, or a core or global one (whose only way to
        // change is a verified patch that a person admits; KM-05): either way a patch.
        Some(s) if matches!(s.scope, Scope::Project | Scope::Core | Scope::Global) => (
            Disposition::Patch {
                target: s.name.clone(),
                similarity: 1.0,
            },
            1.0,
        ),
        _ => {
            let others: Vec<(String, String)> = existing
                .into_iter()
                .filter(|(n, _)| *n != input.name)
                .collect();
            decide(&body, &others, patch_threshold())
        }
    };
    let target_id: Option<Uuid> = match &disposition {
        Disposition::Patch { target, .. } => {
            let t = sqlx::query(
                "SELECT skill_id FROM skill_registry.skill_glossary
                  WHERE name = $1 AND (scope IN ('core','global') OR project_id = $2 OR owner_user_id = $3)",
            )
            .bind(target)
            .bind(input.project)
            .bind(input.user)
            .fetch_optional(pg)
            .await?;
            if let Disposition::Patch { target, .. } = &disposition {
                files = rename_bundle(&files, target);
            }
            t.map(|r| r.get(0))
        }
        Disposition::New => None,
    };
    let proposed_name = match &disposition {
        Disposition::Patch { target, .. } => target.clone(),
        Disposition::New => input.name.clone(),
    };

    let proposal_id: Uuid = sqlx::query(
        "INSERT INTO skill_registry.skill_proposal
           (target_skill_id, proposed_name, project_id, proposer, reason, source_ref, similarity)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING proposal_id",
    )
    .bind(target_id)
    .bind(&proposed_name)
    .bind(input.project)
    .bind(input.user.to_string())
    .bind(&input.reason)
    .bind(&input.source_ref)
    .bind(similarity)
    .fetch_one(pg)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref d) if d.message().contains("proposed_name") => {
            StoreError::Refused(format!("{:?} is not a valid skill name: lowercase letters, digits and hyphens", input.name))
        }
        other => StoreError::Db(other),
    })?
    .get(0);
    sqlx::query(
        "INSERT INTO skill_registry.skill_proposal_bundle (proposal_id, category, files, sources) VALUES ($1, $2, $3, $4)",
    )
    .bind(proposal_id)
    .bind(&input.category)
    .bind(files_to_json(&files))
    .bind(json!(input.sources))
    .execute(pg)
    .await?;

    let verdicts = run_static(pg, proposal_id, &proposed_name, &input.category, &files, &disposition, similarity, &input.reason).await?;
    let failed = verdicts.iter().any(|v| v["passed"] == json!(false));
    if failed {
        sqlx::query(
            "UPDATE skill_registry.skill_proposal SET status = 'rejected', verdict = $2, decided_at = now() WHERE proposal_id = $1",
        )
        .bind(proposal_id)
        .bind(json!({ "failed_step": verdicts.iter().find(|v| v["passed"] == json!(false)).map(|v| v["step"].clone()) }))
        .execute(pg)
        .await?;
    }
    Ok(ProposalOutcome {
        proposal_id,
        status: if failed { "rejected" } else { "submitted" },
        disposition,
        similarity,
        verdicts,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_static(
    pg: &PgPool,
    proposal: Uuid,
    name: &str,
    category: &str,
    files: &BTreeMap<String, Vec<u8>>,
    disposition: &Disposition,
    similarity: f64,
    reason: &str,
) -> Result<Vec<Value>, StoreError> {
    let mut out = Vec::new();
    // Core and global names are a patch only through the release / promotion path:
    // a project proposal may not overwrite a name it does not own.
    let catalogue = tool_catalogue();
    let checked = Draft::check(Scope::Project, category, name, files.clone(), &catalogue);
    let is_procedure = skills::registry::is_procedure(files);
    match &checked {
        Ok(_) => {
            out.push(record(pg, proposal, "lint", true, json!({})).await?);
            out.push(record(pg, proposal, "procedure", true, json!({ "checked": is_procedure })).await?);
        }
        Err(skills::registry::AdmissionError::Procedure(p)) => {
            out.push(record(pg, proposal, "lint", true, json!({})).await?);
            out.push(record(pg, proposal, "procedure", false, json!({ "problems": p })).await?);
        }
        Err(skills::registry::AdmissionError::Lint(f)) => {
            let findings: Vec<Value> = f
                .iter()
                .map(|x| json!({ "file": x.file, "line": x.line, "rule": x.rule, "text": x.text }))
                .collect();
            out.push(record(pg, proposal, "lint", false, json!({ "findings": findings })).await?);
        }
        Err(e) => {
            out.push(record(pg, proposal, "lint", false, json!({ "error": e.to_string() })).await?);
        }
    }
    out.push(record(pg, proposal, "triggers", skills::registry::Reason::parse(reason).is_some(), json!({ "reason": reason })).await?);
    out.push(
        record(
            pg,
            proposal,
            "coexistence",
            true,
            json!({
                "disposition": match disposition { Disposition::New => "new", Disposition::Patch { .. } => "patch" },
                "nearest_similarity": similarity,
                "mode": "lexical",
            }),
        )
        .await?,
    );
    Ok(out)
}

/// The latest verdict of each step.
async fn latest(pg: &PgPool, proposal: Uuid) -> Result<BTreeMap<String, bool>, StoreError> {
    Ok(sqlx::query(
        "SELECT DISTINCT ON (step) step, passed FROM skill_registry.skill_verdict
          WHERE proposal_id = $1 ORDER BY step, verdict_id DESC",
    )
    .bind(proposal)
    .fetch_all(pg)
    .await?
    .iter()
    .map(|r| (r.get::<String, _>(0), r.get::<bool, _>(1)))
    .collect())
}

/// Record an externally produced step result (the Reviewer's verdict, a harness run).
///
/// # Errors
/// A step name the table does not know; the database.
pub async fn record_step(pg: &PgPool, proposal: Uuid, step: &str, passed: bool, detail: Value) -> Result<Value, StoreError> {
    if !skills::registry::REQUIRED_STEPS.contains(&step) && !matches!(step, "tests" | "savings") {
        return Err(StoreError::Refused(format!("{step:?} is not a verification step")));
    }
    record(pg, proposal, step, passed, detail).await
}

/// The quality tests an `analysis-code` bundle owes (RA-3.5): truncation (a back-fill
/// reads the future) and, when the bundle declares it, the noise null. Frontmatter
/// `series`, `ts`, `output` and `p_value` name the payload and output keys, and `checks`
/// lists which tests apply (default `truncation`).
///
/// The trap series carries gaps on purpose: a skill that fills them from later rows is
/// the leak this test exists to catch, so an analysis skill must treat a `null` as
/// missing.
async fn analysis_tests(files: &BTreeMap<String, Vec<u8>>) -> (bool, Value) {
    use skills::quality::{default_cuts, noise_null, truncation_check, white_noise, SeriesSpec};
    use skills::{Contract, Skill};
    let text = files
        .get("SKILL.md")
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or_default();
    let fields = skills::core::split_skill_md("skill", text)
        .map(|(f, _)| f)
        .unwrap_or_default();
    let key = |k: &str, d: &str| fields.get(k).cloned().unwrap_or_else(|| d.to_string());
    let spec = SeriesSpec {
        ts_key: key("ts", "ts"),
        series_key: key("series", "values"),
        out_key: key("output", "result"),
        p_key: key("p_value", "p_value"),
    };
    let checks: Vec<String> = key("checks", "truncation")
        .split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    let Some(script) = files
        .get("scripts/run.py")
        .and_then(|b| std::str::from_utf8(b).ok())
    else {
        return (
            false,
            json!({ "error": "an analysis-code skill needs scripts/run.py defining run(payload)" }),
        );
    };
    let root = std::env::var("TBOT_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let cfg = skills::runner::RunnerConfig::from_workspace(&root);
    let name = format!("{}_check", key("name", "skill").replace('-', "_"));
    let description = "An analysis-code skill under the platform's quality tests, run in the sandbox.";
    let make = |outputs: Vec<String>| {
        let contract = Contract::new(
            vec![spec.ts_key.clone(), spec.series_key.clone()],
            outputs,
            "quality test",
        )
        .map_err(|e| e.to_string())?;
        Skill::author(name.clone(), description, script, contract, None).map_err(|e| e.to_string())
    };
    let mut detail = serde_json::Map::new();
    let mut ok = true;
    for check in &checks {
        match check.as_str() {
            "truncation" => {
                let n = 160usize;
                let ts: Vec<i64> = (0..n as i64).map(|i| 1_700_000_000 + i * 60).collect();
                let mut x = white_noise(n, 3);
                for i in [40usize, 41, 90, 91] {
                    x[i] = f64::NAN;
                }
                let skill = match make(vec![spec.out_key.clone()]) {
                    Ok(s) => s,
                    Err(e) => return (false, json!({ "error": e })),
                };
                let mut cuts = default_cuts(n, 6);
                cuts.extend([40, 90]);
                cuts.sort_unstable();
                cuts.dedup();
                match truncation_check(&skill, &cfg, &spec, &ts, &x, &cuts).await {
                    Ok(r) => {
                        ok &= r.passed;
                        detail.insert(
                            "truncation".into(),
                            json!({
                                "passed": r.passed, "cuts_checked": r.cuts_checked,
                                "first_differing_ts": r.first_differing_ts, "cut_ts": r.cut_ts,
                            }),
                        );
                    }
                    Err(e) => {
                        ok = false;
                        detail.insert("truncation".into(), json!({ "passed": false, "error": e.to_string() }));
                    }
                }
            }
            "noise_null" => {
                let skill = match make(vec![spec.p_key.clone()]) {
                    Ok(s) => s,
                    Err(e) => return (false, json!({ "error": e })),
                };
                match noise_null(&skill, &cfg, &spec, 500, 200, 11).await {
                    Ok(r) => {
                        ok &= r.passed;
                        detail.insert(
                            "noise_null".into(),
                            json!({
                                "passed": r.passed, "trials": r.trials,
                                "false_positive_rate": r.false_positive_rate,
                                "nominal": skills::quality::NOMINAL_LEVEL,
                                "max": skills::quality::MAX_NOISE_FALSE_POSITIVE_RATE,
                            }),
                        );
                    }
                    Err(e) => {
                        ok = false;
                        detail.insert("noise_null".into(), json!({ "passed": false, "error": e.to_string() }));
                    }
                }
            }
            other => {
                ok = false;
                detail.insert(
                    other.to_string(),
                    json!({ "passed": false, "error": "unknown check; use truncation or noise_null" }),
                );
            }
        }
    }
    (ok, Value::Object(detail))
}

async fn bundle_of(pg: &PgPool, proposal: Uuid) -> Result<BTreeMap<String, Vec<u8>>, StoreError> {
    let row = sqlx::query("SELECT files FROM skill_registry.skill_proposal_bundle WHERE proposal_id = $1")
        .bind(proposal)
        .fetch_optional(pg)
        .await?
        .ok_or_else(|| StoreError::NotFound(proposal.to_string()))?;
    Ok(files_from_json(&row.get::<Value, _>(0)))
}

/// Re-run what the platform can run itself and report what is still missing: the
/// replay verdict from the recorded pairs, the quality tests of an analysis-code bundle,
/// and the reviewer row if one exists.
///
/// # Errors
/// The database.
pub async fn verify(pg: &PgPool, proposal: Uuid) -> Result<Value, StoreError> {
    let store = SkillStore::new(pg.clone());
    let files = bundle_of(pg, proposal).await?;
    let (pairs, variants) = store.pairs(Some(proposal), None, "replay").await?;
    let replay = replay_verdict(&pairs, variants, &SequentialDesign::default());
    let detail = match &replay {
        Ok(d) => json!({ "decision": format!("{d:?}"), "pairs": pairs.len(), "variants": variants }),
        Err(f) => json!({ "failure": format!("{f:?}"), "pairs": pairs.len(), "variants": variants }),
    };
    record(pg, proposal, "replay", replay.is_ok(), detail).await?;
    if is_analysis_code(&files) {
        let (passed, detail) = analysis_tests(&files).await;
        record(pg, proposal, "tests", passed, detail).await?;
    }
    if skills::registry::claims_savings(&files) {
        // T5 (RA-3.6): at least 20% cheaper at equal success, from the same paired runs.
        let (rows, _) = store.pairs(Some(proposal), None, "replay").await?;
        let (passed, detail) = match skills::lifecycle::savings_condition(&rows, 0.20) {
            Ok(saved) => (true, json!({ "saved": saved, "required": 0.20 })),
            Err(f) => (false, json!({ "failure": format!("{f:?}"), "required": 0.20 })),
        };
        record(pg, proposal, "savings", passed, detail).await?;
    }
    let l = latest(pg, proposal).await?;
    if !l.contains_key("reviewer") {
        record(
            pg,
            proposal,
            "reviewer",
            false,
            json!({ "missing": "no Reviewer verdict has been recorded" }),
        )
        .await?;
    }
    let l = latest(pg, proposal).await?;
    let required = required_steps(&files);
    let unmet = first_unmet_in(&required, &l);
    Ok(json!({
        "proposal_id": proposal,
        "steps": l,
        "required": required,
        "first_unmet": unmet,
        "admissible": unmet.is_none(),
    }))
}

/// Admit a verified proposal as a **provisional** skill (RA-3.1, RA-4.5). The caller
/// has already checked `skills.admit`.
///
/// # Errors
/// [`StoreError::Refused`] naming the first unmet step.
pub async fn admit_proposal(pg: &PgPool, proposal: Uuid, admitted_by: &str) -> Result<Uuid, StoreError> {
    let l = latest(pg, proposal).await?;
    let required = required_steps(&bundle_of(pg, proposal).await?);
    if let Some(step) = first_unmet_in(&required, &l) {
        return Err(StoreError::Refused(format!(
            "proposal {proposal} is not admissible: step `{step}` has not passed"
        )));
    }
    let row = sqlx::query(
        "SELECT p.proposed_name, p.project_id, p.target_skill_id, p.status, b.category, b.files,
                p.proposer, b.sources
           FROM skill_registry.skill_proposal p
           JOIN skill_registry.skill_proposal_bundle b ON b.proposal_id = p.proposal_id
          WHERE p.proposal_id = $1",
    )
    .bind(proposal)
    .fetch_optional(pg)
    .await?
    .ok_or_else(|| StoreError::NotFound(proposal.to_string()))?;
    let (name, project, target, status): (String, Uuid, Option<Uuid>, String) =
        (row.get(0), row.get(1), row.get(2), row.get(3));
    if status != "submitted" && status != "verifying" {
        return Err(StoreError::Refused(format!("proposal is already {status}")));
    }
    let (category, files): (String, Value) = (row.get(4), row.get(5));
    let draft = Draft::check(Scope::Project, &category, &name, files_from_json(&files), &tool_catalogue())
        .map_err(|e| StoreError::Refused(e.to_string()))?;
    let store = SkillStore::new(pg.clone());
    // A patch's parent is the target's current version, read at admission, so a
    // second patch of the same skill chains onto the first.
    let parent: Option<String> = match target {
        Some(t) => sqlx::query("SELECT current_version FROM skill_registry.skill WHERE skill_id = $1")
            .bind(t)
            .fetch_optional(pg)
            .await?
            .and_then(|r| r.get::<Option<String>, _>(0)),
        None => None,
    };
    let id = store
        .admit(&draft, Status::Provisional, None, Some(project), false, parent.as_deref(), admitted_by)
        .await?;
    if target.is_none() {
        store.freeze_with(project, &name).await?;
    }
    // Findings the proposal cites become the skill's provenance, with the memory
    // store's own taint mark (KM-10): `finding:<id>` entries in the proposal's sources.
    let proposer: String = row.get(6);
    let sources: Value = row.get(7);
    if let Ok(user) = proposer.parse::<Uuid>() {
        for f in sources
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().and_then(|s| s.strip_prefix("finding:")))
        {
            store.cite_finding(id, user, f).await?;
        }
    }
    sqlx::query(
        "UPDATE skill_registry.skill_proposal SET status = 'admitted', verdict = $2, decided_at = now() WHERE proposal_id = $1",
    )
    .bind(proposal)
    .bind(json!({ "skill_id": id, "steps": l }))
    .execute(pg)
    .await?;
    Ok(id)
}
