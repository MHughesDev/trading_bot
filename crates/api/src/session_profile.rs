//! What the platform knows about a tier profile's standing and its `judgement`
//! pass rates (AGENT-014 §7 row 9, §8; AGENT-009 §5; Set S tasks S-11.7, S-11.9;
//! ADR-P11-04).
//!
//! # What was missing
//!
//! Two inputs to the continuous-research gate had no store, so each was declared
//! unmeasured and the gate leaned on the declaration:
//!
//! * **Health.** [`research_session::couplings::ProfileHealth`] has three words
//!   and the driver read the profile's *static* tier. "Withdrawing a scorecard
//!   pauses a session on that profile" had nothing to read, and a hosted
//!   provider whose credential had been removed still read as `measured`.
//! * **`judgement`.** [`research_session::charter::admit`] wants a pass rate per
//!   (profile, workflow) and `eval_scorecards.metrics` is a harness-version
//!   aggregate. So `autonomous` was refused on every profile (ADR-P11-04) and the
//!   half of §8 that matters when it is *on* had never run.
//!
//! `profile_standing` and `profile_judgement` (0163) are those two records,
//! append-only, with this module as their one writer and their one reader.
//!
//! # Three rules
//!
//! **A missing row is `not_measured`, never a pass and never a zero.**
//! [`charter::Profile::judgement_for`] already says so and this module never
//! fills a gap the table left.
//!
//! **A rate is judged against the line it was measured against.** `pass_line` is
//! stored beside `pass_rate`, so a suite that later moves its line does not
//! re-grade rows already written.
//!
//! **A removed credential is a withdrawn profile, whatever the scorecard says.**
//! A hosted provider whose key is gone cannot start a child; the standing that
//! says `measured` is true of a profile nobody can call. The platform notices
//! this itself ([`credential_withdrawn`]) rather than waiting for someone to
//! record it.

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use research_session::charter::{self, Judgement};
use research_session::couplings::ProfileHealth;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// The latest standing recorded for a profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub health: ProfileHealth,
    pub reason: String,
    /// The profile's own `autonomous` line. `None` is "not declared", read as the
    /// strictest, 1.0 — [`STRICTEST_THRESHOLD`].
    pub autonomous_threshold: Option<f64>,
    pub recorded_by: String,
    pub recorded_at: DateTime<Utc>,
}

/// What `autonomous` requires of a profile that never declared a line.
pub const STRICTEST_THRESHOLD: f64 = 1.0;

/// One workflow's latest `judgement` measurement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JudgementRow {
    pub workflow: String,
    pub pass_rate: f64,
    pub pass_line: f64,
    pub trials: i32,
    pub suite_version: String,
    pub measured_at: DateTime<Utc>,
}

impl JudgementRow {
    /// The verdict, against the line the row was measured against.
    #[must_use]
    pub fn verdict(&self) -> Judgement {
        if !self.pass_rate.is_finite() {
            return Judgement::NotMeasured;
        }
        if self.pass_rate >= self.pass_line {
            Judgement::Passed {
                rate: self.pass_rate,
            }
        } else {
            Judgement::Failed {
                rate: self.pass_rate,
            }
        }
    }
}

/// One scorecard as the eval harness (or an operator) records it.
#[derive(Debug, Clone, Deserialize)]
pub struct ScorecardInput {
    /// `measured | degraded | withdrawn`.
    pub health: String,
    pub reason: String,
    #[serde(default)]
    pub autonomous_threshold: Option<f64>,
    #[serde(default)]
    pub judgement: Vec<JudgementInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JudgementInput {
    pub workflow: String,
    pub pass_rate: f64,
    pub pass_line: f64,
    pub trials: i32,
    pub suite_version: String,
}

/// Why a scorecard was refused. The table's CHECKs refuse the same things; this
/// is the sentence.
#[derive(Debug, thiserror::Error)]
pub enum ScorecardError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Db(#[from] sqlx::Error),
}

fn health_word(h: ProfileHealth) -> &'static str {
    h.as_str()
}

fn parse_health(s: &str) -> Option<ProfileHealth> {
    match s {
        "measured" => Some(ProfileHealth::Measured),
        "degraded" => Some(ProfileHealth::Degraded),
        "withdrawn" => Some(ProfileHealth::Withdrawn),
        _ => None,
    }
}

impl ScorecardInput {
    /// Refuse what the table would refuse, with the reason in words.
    ///
    /// # Errors
    /// [`ScorecardError::Invalid`] naming the field.
    pub fn validate(&self) -> Result<ProfileHealth, ScorecardError> {
        let health = parse_health(&self.health).ok_or_else(|| {
            ScorecardError::Invalid(format!(
                "`health` is `{}`; it is one of measured, degraded or withdrawn",
                self.health
            ))
        })?;
        if self.reason.trim().is_empty() {
            return Err(ScorecardError::Invalid(
                "a standing says why: a profile that goes `degraded` with no reason is a pause \
                 nobody can explain"
                    .into(),
            ));
        }
        if let Some(t) = self.autonomous_threshold {
            if !(t.is_finite() && t > 0.0 && t <= 1.0) {
                return Err(ScorecardError::Invalid(format!(
                    "`autonomous_threshold` is {t}; it is a pass rate in (0, 1]"
                )));
            }
        }
        for j in &self.judgement {
            if j.workflow.trim().is_empty() || j.suite_version.trim().is_empty() {
                return Err(ScorecardError::Invalid(
                    "every judgement names its workflow and the suite version that measured it"
                        .into(),
                ));
            }
            if !(j.pass_rate.is_finite() && (0.0..=1.0).contains(&j.pass_rate)) {
                return Err(ScorecardError::Invalid(format!(
                    "`{}`: `pass_rate` is {}; it is a rate in [0, 1]",
                    j.workflow, j.pass_rate
                )));
            }
            if !(j.pass_line.is_finite() && j.pass_line > 0.0 && j.pass_line <= 1.0) {
                return Err(ScorecardError::Invalid(format!(
                    "`{}`: `pass_line` is {}; it is a rate in (0, 1]",
                    j.workflow, j.pass_line
                )));
            }
            if j.trials <= 0 {
                return Err(ScorecardError::Invalid(format!(
                    "`{}`: a pass rate over {} trials is not a measurement",
                    j.workflow, j.trials
                )));
            }
        }
        Ok(health)
    }
}

/// Record one scorecard: its standing and each judgement, in one transaction, so
/// a standing that says `measured` never exists without the rates it was
/// measured on.
///
/// # Errors
/// [`ScorecardError`].
pub async fn record_scorecard(
    pg: &PgPool,
    profile: &str,
    input: &ScorecardInput,
    recorded_by: &str,
) -> Result<ProfileHealth, ScorecardError> {
    let health = input.validate()?;
    if profile.trim().is_empty() {
        return Err(ScorecardError::Invalid("a scorecard names its profile".into()));
    }
    let mut tx = pg.begin().await?;
    for j in &input.judgement {
        sqlx::query(
            "INSERT INTO profile_judgement
                 (profile_id, workflow, pass_rate, pass_line, trials, suite_version)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(profile)
        .bind(&j.workflow)
        .bind(j.pass_rate)
        .bind(j.pass_line)
        .bind(j.trials)
        .bind(&j.suite_version)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO profile_standing
             (profile_id, health, reason, autonomous_threshold, recorded_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(health_word(health))
    .bind(input.reason.trim())
    .bind(input.autonomous_threshold)
    .bind(recorded_by)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(health)
}

/// The latest standing recorded for `profile`, or `None` when nobody ever
/// recorded one — which is a statement about the record and not about the
/// profile.
///
/// # Errors
/// Database errors.
pub async fn latest_standing(pg: &PgPool, profile: &str) -> Result<Option<Standing>, sqlx::Error> {
    type Row = (String, String, Option<f64>, String, DateTime<Utc>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT health, reason, autonomous_threshold, recorded_by, recorded_at
           FROM profile_standing WHERE profile_id = $1
          ORDER BY recorded_at DESC, standing_id DESC LIMIT 1",
    )
    .bind(profile)
    .fetch_optional(pg)
    .await?;
    Ok(row.and_then(|(health, reason, threshold, by, at)| {
        parse_health(&health).map(|health| Standing {
            health,
            reason,
            autonomous_threshold: threshold,
            recorded_by: by,
            recorded_at: at,
        })
    }))
}

/// The latest measurement per workflow for `profile`.
///
/// # Errors
/// Database errors.
pub async fn latest_judgement(
    pg: &PgPool,
    profile: &str,
) -> Result<Vec<JudgementRow>, sqlx::Error> {
    type Row = (String, f64, f64, i32, String, DateTime<Utc>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT DISTINCT ON (workflow)
                workflow, pass_rate, pass_line, trials, suite_version, measured_at
           FROM profile_judgement WHERE profile_id = $1
          ORDER BY workflow, measured_at DESC, judgement_id DESC",
    )
    .bind(profile)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(workflow, pass_rate, pass_line, trials, suite_version, measured_at)| JudgementRow {
                workflow,
                pass_rate,
                pass_line,
                trials,
                suite_version,
                measured_at,
            },
        )
        .collect())
}

/// §8's `Profile`, from the loaded harness profile and what the platform has
/// recorded about it.
///
/// `measured` is true only when the latest standing says `measured`: a
/// `degraded` or `withdrawn` profile is not one a session may be chartered on,
/// and a profile with **no** record is not measured either (ADR-P11-04 — an
/// unmeasured profile is refused `autonomous` and a note under `notify`).
/// `judgement` has one entry per workflow that has a row; a workflow without one
/// is absent and reads `NotMeasured`.
#[must_use]
pub fn charter_profile(
    name: &str,
    p: &harness::Profile,
    standing: Option<&Standing>,
    judgement: &[JudgementRow],
) -> charter::Profile {
    let map: BTreeMap<String, Judgement> = judgement
        .iter()
        .map(|j| (j.workflow.clone(), j.verdict()))
        .collect();
    charter::Profile {
        name: name.to_string(),
        frontier: p.tier == harness::profile::Tier::Frontier,
        measured: standing.is_some_and(|s| s.health == ProfileHealth::Measured),
        autonomous_threshold: standing
            .and_then(|s| s.autonomous_threshold)
            .unwrap_or(STRICTEST_THRESHOLD),
        judgement: map,
        supports: p.workflows.iter().map(|w| w.as_str().to_string()).collect(),
    }
}

/// Whether the hosted provider this session's journal runs on has lost its
/// credential.
///
/// `Ok(false)` for a local provider (it needs none) and for one whose
/// credential row exists. An unreadable table is an `Err`, so the caller can
/// hold the iteration rather than pause a session on a read that failed.
///
/// # Errors
/// Database errors.
pub async fn credential_withdrawn(
    pg: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let provider: Option<(String,)> =
        sqlx::query_as("SELECT provider FROM agent_conversations WHERE conversation_id = $1")
            .bind(conversation_id)
            .fetch_optional(pg)
            .await?;
    let Some((provider,)) = provider else {
        return Ok(None);
    };
    let Ok(parsed) = llm::Provider::from_str(&provider) else {
        // A provider this build does not know cannot be said to have lost its
        // key; the child start refuses it by name.
        return Ok(None);
    };
    if !parsed.requires_api_key() {
        return Ok(None);
    }
    let present: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM llm_credentials WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(&provider)
            .fetch_optional(pg)
            .await?;
    Ok(present.is_none().then_some(provider))
}

/// What the driver reads each iteration: a session's profile health, from the
/// strongest evidence the platform holds.
///
/// In order, and the first that speaks wins: a withdrawn credential; the latest
/// recorded standing; the static answer (`fallback`) for a profile nobody ever
/// recorded. `fallback` is the old behaviour, kept for the profile with no
/// scorecard so that recording nothing pauses nothing — the driver's own frontier
/// rule is applied before this and is not softened by it.
#[must_use]
pub fn health_from(
    fallback: ProfileHealth,
    withdrawn_credential: Option<&str>,
    standing: Option<&Standing>,
) -> (ProfileHealth, Option<String>) {
    if let Some(provider) = withdrawn_credential {
        return (
            ProfileHealth::Withdrawn,
            Some(format!(
                "the `{provider}` credential this session runs on has been removed"
            )),
        );
    }
    if let Some(s) = standing {
        // `measured` on record does not lift a static finding that the profile is
        // degraded (a continuous session on a local tier): the stricter reading
        // stands.
        let strict = match (fallback, s.health) {
            (ProfileHealth::Withdrawn, _) | (_, ProfileHealth::Withdrawn) => {
                ProfileHealth::Withdrawn
            }
            (ProfileHealth::Degraded, _) | (_, ProfileHealth::Degraded) => ProfileHealth::Degraded,
            _ => ProfileHealth::Measured,
        };
        return (strict, Some(s.reason.clone()));
    }
    (fallback, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(h: ProfileHealth) -> Standing {
        Standing {
            health: h,
            reason: "scorecard 2026-10-04".into(),
            autonomous_threshold: None,
            recorded_by: "eval".into(),
            recorded_at: Utc::now(),
        }
    }

    fn row(rate: f64, line: f64) -> JudgementRow {
        JudgementRow {
            workflow: "investigate".into(),
            pass_rate: rate,
            pass_line: line,
            trials: 40,
            suite_version: "judgement-v1".into(),
            measured_at: Utc::now(),
        }
    }

    #[test]
    fn a_rate_is_judged_against_the_line_it_was_measured_against() {
        assert!(row(0.9, 0.8).verdict().passed());
        assert!(!row(0.7, 0.8).verdict().passed());
        assert!(
            matches!(row(0.7, 0.8).verdict(), Judgement::Failed { .. }),
            "below the line is a measured failure and not an absence"
        );
        assert_eq!(row(f64::NAN, 0.8).verdict(), Judgement::NotMeasured);
    }

    #[test]
    fn a_degraded_or_withdrawn_standing_pauses_and_a_removed_credential_outranks_a_good_one() {
        let (h, why) = health_from(
            ProfileHealth::Measured,
            None,
            Some(&standing(ProfileHealth::Degraded)),
        );
        assert_eq!(h, ProfileHealth::Degraded);
        assert!(why.is_some());
        let (h, _) = health_from(
            ProfileHealth::Measured,
            Some("anthropic"),
            Some(&standing(ProfileHealth::Measured)),
        );
        assert_eq!(h, ProfileHealth::Withdrawn, "a profile nobody can call");
    }

    #[test]
    fn a_profile_nobody_recorded_keeps_the_static_answer_and_a_good_record_cannot_lift_a_bad_one() {
        assert_eq!(
            health_from(ProfileHealth::Measured, None, None).0,
            ProfileHealth::Measured
        );
        assert_eq!(
            health_from(
                ProfileHealth::Degraded,
                None,
                Some(&standing(ProfileHealth::Measured))
            )
            .0,
            ProfileHealth::Degraded,
            "the frontier rule is not softened by a scorecard"
        );
    }

    /// The first time `autonomous` can be granted - and only on a measurement.
    #[test]
    fn autonomous_is_granted_on_a_recorded_passing_judgement_and_refused_below_the_line() {
        use research_session::charter::{admit, Autonomy, Charter, ExplorationShare, Objective, Scope};
        let charter = Charter {
            scope: Scope::default(),
            objective: Objective::BuildKnowledge { about: vec!["BTC-USD".into()] },
            seeds: vec![],
            permitted_workflows: vec!["investigate".into()],
            exclusions: vec![],
            standing_constraints: vec![],
            exploration_share: ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        };
        let profile = |rate: f64, health: ProfileHealth| {
            let p = charter::Profile {
                name: "opus".into(),
                frontier: true,
                measured: false,
                autonomous_threshold: STRICTEST_THRESHOLD,
                judgement: BTreeMap::new(),
                supports: vec!["investigate".into()],
            };
            // Built the way `charter_profile` builds it, from rows.
            let st = Standing {
                health,
                reason: "suite".into(),
                autonomous_threshold: Some(0.9),
                recorded_by: "eval".into(),
                recorded_at: Utc::now(),
            };
            let rows = [row(rate, 0.8)];
            charter::Profile {
                measured: st.health == ProfileHealth::Measured,
                autonomous_threshold: st.autonomous_threshold.unwrap_or(STRICTEST_THRESHOLD),
                judgement: rows.iter().map(|j| (j.workflow.clone(), j.verdict())).collect(),
                ..p
            }
        };
        assert!(admit(&charter, Autonomy::Autonomous, &profile(0.95, ProfileHealth::Measured)).is_ok());
        assert!(
            admit(&charter, Autonomy::Autonomous, &profile(0.85, ProfileHealth::Measured)).is_err(),
            "passes the suite's line (0.8) and not the profile's own autonomous line (0.9)"
        );
        assert!(admit(&charter, Autonomy::Autonomous, &profile(0.95, ProfileHealth::Degraded)).is_err());
    }

    #[test]
    fn a_scorecard_is_refused_for_what_the_table_would_refuse() {
        let ok = ScorecardInput {
            health: "measured".into(),
            reason: "judgement-v1 passed".into(),
            autonomous_threshold: Some(0.9),
            judgement: vec![JudgementInput {
                workflow: "investigate".into(),
                pass_rate: 0.95,
                pass_line: 0.8,
                trials: 40,
                suite_version: "judgement-v1".into(),
            }],
        };
        assert!(ok.validate().is_ok());
        for bad in [
            ScorecardInput {
                health: "fine".into(),
                ..ok.clone()
            },
            ScorecardInput {
                reason: "  ".into(),
                ..ok.clone()
            },
            ScorecardInput {
                autonomous_threshold: Some(1.5),
                ..ok.clone()
            },
            ScorecardInput {
                judgement: vec![JudgementInput {
                    trials: 0,
                    ..ok.judgement[0].clone()
                }],
                ..ok.clone()
            },
            ScorecardInput {
                judgement: vec![JudgementInput {
                    pass_rate: f64::NAN,
                    ..ok.judgement[0].clone()
                }],
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }
}
