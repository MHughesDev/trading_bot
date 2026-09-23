//! The charter form's shape, translated into the charter the spec defines
//! (AGENT-012 §2b, AGENT-014 §2.2; Set S tasks S-11.14, S-11.13).
//!
//! # Why there are two shapes at all
//!
//! The desk's form is flat: four chip lists, a 7 × 24 schedule grid, a
//! `budget_usd_day` next to a `max_runs`, an autonomy select. That is what a
//! person fills in, and it is the right shape for a form — every field is one
//! control and nothing is nested.
//!
//! [`research_session::Charter`] is the shape AGENT-014 §2.2 defines: a `Scope`,
//! a tagged `Objective`, an `ExplorationShare` that cannot be below its floor,
//! and a `Schedule` of `{days, start, end, tz}` windows. That is the right shape
//! for a stored object, because each of its parts is a thing the driver asks a
//! question of.
//!
//! Translating between them is a real step with real decisions in it, so it is a
//! function with tests rather than a `serde` alias list. The decisions:
//!
//! * **The grid becomes contiguous runs, not 168 one-hour windows.** A person who
//!   selects Monday 09:00–17:00 has described one window and the digest should
//!   say one window.
//! * **An empty grid means always open**, matching `Schedule::default` — a person
//!   who selected no hours has not forbidden every hour.
//! * **`delta_practical` is refused rather than dropped when it does not parse.**
//!   It is an effect size the runs under this charter will be suggested to
//!   pre-register against, and silently discarding one the user typed is the
//!   failure mode that makes a suggestion meaningless.
//! * **The form has no exploration field**, so the share is the default. It
//!   cannot be set below the floor from anywhere, which is the point of the floor
//!   being a type.

use research_session::charter::{
    Autonomy, Charter, ExplorationShare, Objective, ReviewEvery, Schedule, Scope, Seed,
    SessionBudget, Window, DEFAULT_MAX_RUNS, DEFAULT_USD_PER_DAY,
};
use serde::Deserialize;
use serde_json::json;

/// What the charter form posts.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeskCharter {
    #[serde(default)]
    pub instruments: Vec<String>,
    #[serde(default)]
    pub asset_classes: Vec<String>,
    #[serde(default)]
    pub families: Vec<String>,
    #[serde(default)]
    pub timeframes: Vec<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub seeds: Vec<String>,
    /// The workflows the charter permits. The form pre-selects all of them, and
    /// "all" is expressed here as the full list rather than as an empty one — so
    /// a user who *deselects* every workflow gets an empty list, which the
    /// charter reads as "all" and which is therefore refused before it is stored.
    #[serde(default)]
    pub workflows: Vec<String>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub standing_constraints: Vec<String>,
    #[serde(default)]
    pub budget_usd_day: Option<f64>,
    #[serde(default)]
    pub budget_usd_total: Option<f64>,
    #[serde(default)]
    pub max_trials: Option<i64>,
    #[serde(default)]
    pub max_runs: Option<i32>,
    /// 7 × 24 booleans, Monday first. Empty means any time.
    #[serde(default)]
    pub schedule: Vec<Vec<bool>>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub concurrency: Option<i16>,
    #[serde(default)]
    pub autonomy: Option<String>,
    #[serde(default)]
    pub replenish: bool,
    #[serde(default)]
    pub review_every: Option<u32>,
    /// A number as typed. Empty is "not stated"; unparseable is refused.
    #[serde(default)]
    pub delta_practical: Option<String>,
}

/// What the form can be refused for, in words a person can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub message: String,
    pub fix: String,
}

/// Everything the form decides, in the shapes the session stores.
#[derive(Debug, Clone, PartialEq)]
pub struct Translated {
    pub charter: Charter,
    pub budget: SessionBudget,
    pub schedule: Schedule,
    pub autonomy: Autonomy,
    pub review_every: ReviewEvery,
    pub replenish: bool,
}

/// Translate the form.
///
/// # Errors
///
/// Refuses an unparseable `delta_practical`, a concurrency outside 1..=3, an
/// autonomy word that is not one of the three, and a schedule grid that is not
/// 7 × 24 when one is given at all.
pub fn translate(form: &DeskCharter) -> Result<Translated, Refusal> {
    let delta = match form.delta_practical.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => Some(raw.parse::<f64>().map_err(|_| {
            Refusal {
                message: format!("`{raw}` is not a number, and delta_practical is an effect size"),
                fix:
                    "write it as a decimal, for example 0.25 — or leave it blank and let each run \
                  seal its own. Dropping what you typed would make the suggestion mean nothing."
                        .into(),
            }
        })?),
    };
    if let Some(d) = delta {
        if !(d.is_finite() && d > 0.0) {
            return Err(Refusal {
                message: format!("delta_practical {d} is not a positive effect size"),
                fix: "a practical effect is a size you would act on; zero is every effect".into(),
            });
        }
    }

    let autonomy = match form.autonomy.as_deref() {
        None => Autonomy::default(),
        Some(a) => Autonomy::parse(a).ok_or_else(|| Refusal {
            message: format!("`{a}` is not an autonomy setting"),
            fix: "one of supervised, notify, autonomous".into(),
        })?,
    };

    if form.max_trials == Some(0) {
        return Err(Refusal {
            message: "a trial cap of zero is a session that cannot research".into(),
            fix: "leave it blank for no cap, or give it a number of trials it may spend".into(),
        });
    }

    // Every window's timezone, checked here rather than at the first iteration
    // of the driver. `form.timezone.unwrap_or("UTC")` used to be written into
    // every window with no check at all, so `Amercia/New_York` was accepted at
    // the charter form and only failed days later — and a fence that silently
    // became UTC is a session spending money in hours the operator believed it
    // was fenced out of. `OpenHours::parse` is the same reader the driver uses,
    // so the form and the driver cannot disagree about what a schedule means.
    let concurrency = form.concurrency.unwrap_or(1);
    let schedule = Schedule {
        windows: windows_from_grid(&form.schedule, form.timezone.as_deref().unwrap_or("UTC"))?,
        max_concurrent_runs: 1,
    }
    .with_concurrency(concurrency)
    .map_err(|e| Refusal {
        message: e.to_string(),
        fix: "one, two or three at a time".into(),
    })?;

    // The schedule must be readable by the thing that will read it.
    research_session::schedule::OpenHours::parse(&schedule).map_err(|e| Refusal {
        message: e.to_string(),
        fix: "pick the timezone from the list rather than typing it, and give a window an end               later than its start"
            .into(),
    })?;

    let usd_per_day = form.budget_usd_day.unwrap_or(DEFAULT_USD_PER_DAY);
    let max_runs = form.max_runs.unwrap_or(DEFAULT_MAX_RUNS);
    let budget = SessionBudget {
        usd_per_day,
        // The form's own default, not a second policy: a total left blank is
        // thirty days of the day budget, which is what `SessionBudget::default`
        // says and what the charter form pre-fills.
        usd_total: form
            .budget_usd_total
            .unwrap_or(usd_per_day * research_session::charter::DEFAULT_BUDGET_DAYS),
        // `unwrap_or(0)` was here and it conflated two answers: a user who left
        // the field blank and a user who typed a cap of zero arrived identical,
        // and the desk reads a stored 0 as "no cap recorded" because that is the
        // safe reading — which makes a genuine cap of zero unexpressible. Zero
        // is not a cap anybody wants, so the honest fix is to refuse it at the
        // door rather than to widen the column: a session that may spend no
        // trials is a session that cannot research, and saying so is better than
        // storing a number that means "blank" to everyone downstream.
        trials_total: form.max_trials.unwrap_or(0),
        max_runs,
    };

    let subjects: Vec<String> = form
        .instruments
        .iter()
        .chain(form.families.iter())
        .chain(form.asset_classes.iter())
        .cloned()
        .collect();
    let objective = match form.objective.as_deref().unwrap_or("find_deployable") {
        "build_knowledge" => Objective::BuildKnowledge { about: subjects },
        "maintain" => Objective::Maintain {
            strategy_ids: form.families.clone(),
        },
        "mixed" => Objective::Mixed {
            parts: vec![
                Objective::FindDeployable {
                    constraints: json!({ "subjects": subjects }),
                },
                Objective::BuildKnowledge {
                    about: form.families.clone(),
                },
            ],
        },
        // `find_deployable` and anything unrecognised. Not refused: the form is a
        // select, so an unknown word means an older or newer client, and the
        // conservative reading is the one the form defaults to.
        _ => Objective::FindDeployable {
            constraints: json!({ "subjects": subjects }),
        },
    };

    let charter = Charter {
        scope: Scope {
            instruments: form.instruments.clone(),
            asset_classes: form.asset_classes.clone(),
            families: form.families.clone(),
            timeframes: form.timeframes.clone(),
            windows: Vec::new(),
        },
        objective,
        seeds: form
            .seeds
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| Seed {
                question: s.to_string(),
                // The form asks for a sentence, not a subject. Left empty rather
                // than guessed: the dedupe key is `subject|question`, and a
                // guessed subject would make two identical questions look like
                // different ones.
                subject: String::new(),
                workflow: None,
            })
            .collect(),
        permitted_workflows: form.workflows.clone(),
        exclusions: form.exclusions.clone(),
        standing_constraints: form.standing_constraints.clone(),
        exploration_share: ExplorationShare::default(),
        stop_when: Vec::new(),
        delta_practical_default: delta,
        version: 1,
    };

    Ok(Translated {
        charter,
        budget,
        schedule,
        autonomy,
        review_every: form
            .review_every
            .map_or(ReviewEvery::default(), |runs| ReviewEvery::Runs { runs }),
        replenish: form.replenish,
    })
}

/// Turn the 7 × 24 grid into contiguous windows.
///
/// A person who selects Monday 09:00–17:00 has described one window, and the
/// digest and the header should say one window rather than eight. Days that
/// share an identical set of hours are **not** merged into one multi-day window
/// here — that would be a second, cleverer pass whose only benefit is a shorter
/// list, and a window per day per run is already what `{days: [...]}` can hold if
/// it ever matters.
fn windows_from_grid(grid: &[Vec<bool>], tz: &str) -> Result<Vec<Window>, Refusal> {
    if grid.is_empty() {
        return Ok(Vec::new());
    }
    if grid.len() != 7 || grid.iter().any(|d| d.len() != 24) {
        return Err(Refusal {
            message: format!(
                "the schedule grid is {} rows and should be 7 days of 24 hours",
                grid.len()
            ),
            fix: "send an empty grid for `any time`, or a full week".into(),
        });
    }
    let mut out = Vec::new();
    for (day, hours) in grid.iter().enumerate() {
        let mut h = 0;
        while h < 24 {
            if !hours[h] {
                h += 1;
                continue;
            }
            let start = h;
            while h < 24 && hours[h] {
                h += 1;
            }
            out.push(Window {
                // ISO weekday numbers, Monday = 1 — the grid is Monday first.
                days: vec![u8::try_from(day + 1).unwrap_or(1)],
                start: format!("{start:02}:00"),
                // An hour that ends at 24 ends at the end of the day. `24:00` is
                // written rather than `00:00` because `00:00` would read as a
                // window that ends before it starts.
                end: format!("{h:02}:00"),
                tz: tz.to_string(),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(day: usize, hours: std::ops::Range<usize>) -> Vec<Vec<bool>> {
        let mut g = vec![vec![false; 24]; 7];
        for h in hours {
            g[day][h] = true;
        }
        g
    }

    #[test]
    fn a_selected_block_becomes_one_window_not_eight() {
        let w = windows_from_grid(&grid(0, 9..17), "Europe/London").expect("a valid grid");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].days, vec![1]);
        assert_eq!(w[0].start, "09:00");
        assert_eq!(w[0].end, "17:00");
        assert_eq!(w[0].tz, "Europe/London");
    }

    #[test]
    fn two_blocks_in_one_day_are_two_windows() {
        let mut g = vec![vec![false; 24]; 7];
        for h in 9..12 {
            g[2][h] = true;
        }
        for h in 14..18 {
            g[2][h] = true;
        }
        let w = windows_from_grid(&g, "UTC").expect("a valid grid");
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].start.as_str(), w[0].end.as_str()), ("09:00", "12:00"));
        assert_eq!((w[1].start.as_str(), w[1].end.as_str()), ("14:00", "18:00"));
        assert!(w.iter().all(|x| x.days == vec![3]));
    }

    #[test]
    fn a_block_running_to_midnight_ends_at_twenty_four() {
        let w = windows_from_grid(&grid(6, 22..24), "UTC").expect("a valid grid");
        assert_eq!(
            w[0].end, "24:00",
            "00:00 would read as ending before it starts"
        );
        assert_eq!(w[0].days, vec![7]);
    }

    #[test]
    fn an_empty_grid_is_always_open_and_a_wrong_sized_one_is_refused() {
        assert!(windows_from_grid(&[], "UTC")
            .expect("empty is fine")
            .is_empty());
        assert!(windows_from_grid(&vec![vec![false; 24]; 5], "UTC").is_err());
        assert!(windows_from_grid(&vec![vec![false; 23]; 7], "UTC").is_err());
    }

    #[test]
    fn an_unparseable_effect_size_is_refused_rather_than_dropped() {
        let form = DeskCharter {
            delta_practical: Some("about a quarter".into()),
            ..DeskCharter::default()
        };
        let e = translate(&form).expect_err("refused");
        assert!(e.message.contains("about a quarter"));

        let zero = DeskCharter {
            delta_practical: Some("0".into()),
            ..DeskCharter::default()
        };
        assert!(translate(&zero).is_err(), "zero is every effect");

        let blank = DeskCharter {
            delta_practical: Some("   ".into()),
            ..DeskCharter::default()
        };
        assert_eq!(
            translate(&blank)
                .expect("blank is not stated")
                .charter
                .delta_practical_default,
            None
        );
    }

    #[test]
    fn a_timezone_the_platform_does_not_know_is_refused_at_the_form() {
        // It used to be written into every window unchecked, so a typo was
        // accepted here and only failed when the driver first read it — days
        // later, on a session that had been running in UTC rather than in the
        // hours the operator fenced.
        let form = DeskCharter {
            schedule: grid(0, 9..17),
            timezone: Some("Amercia/New_York".into()),
            ..DeskCharter::default()
        };
        let e = translate(&form).expect_err("refused");
        assert!(e.message.contains("Amercia/New_York"), "{}", e.message);

        // And the spelling that is right still passes.
        let ok = DeskCharter {
            schedule: grid(0, 9..17),
            timezone: Some("America/New_York".into()),
            ..DeskCharter::default()
        };
        assert!(translate(&ok).is_ok());
    }

    #[test]
    fn a_trial_cap_of_zero_is_refused_rather_than_stored_as_blank() {
        // Blank and zero used to arrive identical, and the desk reads a stored
        // zero as "no cap recorded" because that is the safe reading — which
        // made a genuine cap of zero unexpressible. Refusing it keeps blank
        // meaning blank.
        let zero = DeskCharter {
            max_trials: Some(0),
            ..DeskCharter::default()
        };
        assert!(translate(&zero).is_err());
        let blank = translate(&DeskCharter::default()).expect("blank is fine");
        assert_eq!(blank.budget.trials_total, 0, "blank still stores 0");
        let real = DeskCharter {
            max_trials: Some(40),
            ..DeskCharter::default()
        };
        assert_eq!(translate(&real).expect("a cap").budget.trials_total, 40);
    }

    #[test]
    fn the_form_cannot_produce_a_share_below_the_floor() {
        // The form has no field for it, which is the point: there is no path
        // from the desk to a session that explores less than one pick in ten.
        let t = translate(&DeskCharter::default()).expect("the empty form");
        assert!(t.charter.exploration_share.get() >= research_session::EXPLORATION_FLOOR);
    }

    #[test]
    fn the_budget_defaults_are_the_platform_policy_and_nothing_is_required() {
        let t = translate(&DeskCharter::default()).expect("the empty form");
        assert!((t.budget.usd_per_day - 25.0).abs() < 1e-12);
        assert!((t.budget.usd_total - 750.0).abs() < 1e-12);
        assert_eq!(t.budget.max_runs, 60);
        assert_eq!(t.autonomy, Autonomy::Notify);
        assert_eq!(t.schedule.max_concurrent_runs, 1);
        assert!(t.schedule.windows.is_empty());
    }

    #[test]
    fn concurrency_and_autonomy_are_refused_rather_than_clamped() {
        let too_many = DeskCharter {
            concurrency: Some(9),
            ..DeskCharter::default()
        };
        assert!(translate(&too_many).is_err());
        let unknown = DeskCharter {
            autonomy: Some("whatever".into()),
            ..DeskCharter::default()
        };
        assert!(translate(&unknown).is_err());
    }

    #[test]
    fn the_scope_and_the_objective_carry_what_the_chips_said() {
        let form = DeskCharter {
            instruments: vec!["BTC-USD".into()],
            families: vec!["momentum".into()],
            objective: Some("build_knowledge".into()),
            seeds: vec!["  does momentum survive costs?  ".into(), "   ".into()],
            workflows: vec!["investigate".into(), "replicate".into()],
            exclusions: vec!["optimise".into()],
            ..DeskCharter::default()
        };
        let t = translate(&form).expect("a valid form");
        assert!(t.charter.scope.admits("BTC-USD"));
        assert!(t.charter.scope.admits("momentum"));
        assert!(!t.charter.scope.admits("ETH-USD"));
        assert!(t.charter.permits("investigate"));
        assert!(!t.charter.permits("optimise"), "excluded and not permitted");
        assert!(!t.charter.permits("survey"), "not in the permitted list");
        // The blank seed is dropped and the other is trimmed.
        assert_eq!(t.charter.seeds.len(), 1);
        assert_eq!(t.charter.seeds[0].question, "does momentum survive costs?");
        assert!(matches!(
            t.charter.objective,
            Objective::BuildKnowledge { .. }
        ));
        assert!((t.charter.objective.weight_for("momentum") - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_translated_charter_survives_a_round_trip_through_json() {
        // It is stored as a document and read back by `sessions::get`; a field
        // that serialises and does not deserialise would be a charter that
        // silently loses a constraint the user typed.
        let form = DeskCharter {
            instruments: vec!["BTC-USD".into()],
            delta_practical: Some("0.25".into()),
            schedule: grid(1, 8..12),
            timezone: Some("America/New_York".into()),
            concurrency: Some(2),
            autonomy: Some("supervised".into()),
            review_every: Some(3),
            ..DeskCharter::default()
        };
        let t = translate(&form).expect("a valid form");
        let back: Charter =
            serde_json::from_value(serde_json::to_value(&t.charter).expect("serialises"))
                .expect("deserialises");
        assert_eq!(back, t.charter);
        assert_eq!(t.review_every, ReviewEvery::Runs { runs: 3 });
        assert_eq!(t.autonomy, Autonomy::Supervised);
        assert_eq!(t.schedule.max_concurrent_runs, 2);
        assert_eq!(t.schedule.windows.len(), 1);
    }
}
