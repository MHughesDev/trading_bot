//! Execution: what a child is told, what it is allowed to spend, and what the
//! session may do next (AGENT-014 §6.2–§6.4, §7 rows 1–2).
//!
//! [`crate::harvest`] is what a run puts back. This is the other end: what it
//! is handed before it starts. Three decisions live here, and the one in the
//! middle is the one the whole phase turns on.
//!
//! * the **brief** (§6.2) — the charter's standing constraints, the session's
//!   last five verdicts in scope, and one sentence saying why this item is
//!   running now;
//! * the **lease** (§6.3) — how much of the session's remaining budget the
//!   child may spend, and what the child's `max_trials_per_task` is;
//! * the **steer** (§6.4) — a redirect rewrites the item's question, and with
//!   it the dedupe key, so the agenda does not lie about what ran.
//!
//! Plus the two things §7 asks that are decisions and not plumbing: [`plan`]
//! says what the driver does this iteration and in which order — the order is
//! load-bearing, a suspended child is resumed *before* anything new is claimed
//! — and [`stop`] answers row 3, *"total budget / `max_runs` / `stop_when` met
//! → `complete`"*, which was a vocabulary with no reader anywhere in the
//! repository until now.
//!
//! # A budget has a unit, and this module refuses to guess it
//!
//! [`crate::digest`] found this trap and made it loud; the lease is where it
//! actually bites. §6.3 sizes the lease in **USD** — `estimate_cost × 1.5`,
//! capped by the tier's `max_usd_per_task` — and `SessionBudget::usd_per_day`
//! is dollars. But `llm_usage.cost_usd` keys on the **auth session**, not on
//! the run, so nothing on this platform can price one run, which is why
//! [`crate::select`] scores cost in trials and why `SessionBudget` also carries
//! `trials_total`. A lease that mixed the two would cap a trial count with a
//! dollar figure and nobody would ever see it.
//!
//! So every quantity here is a [`Meter`] — a number **and** its
//! [`SpendUnit`] — a [`LeaseRequest`] declares the one unit it is denominated
//! in, and a bound in another unit is refused by name rather than converted.
//! There is no conversion in this module and no price table to do one with.
//!
//! # Three ways a bound can be missing, and they are not the same thing
//!
//! | shape | means | lease |
//! |---|---|---|
//! | [`Observed::Is`]`(`[`Bound::Cap`]`)` | the platform holds this cap | applied |
//! | [`Observed::Is`]`(`[`Bound::NotDenominated`]`)` | the platform holds **no** cap of this kind in this unit, and names the column that would carry one | recorded in [`Lease::unbounded_by`] |
//! | [`Observed::Unknown`] | the read did not happen | **refused** |
//!
//! The middle row is the one this platform is in today for the day budget:
//! `SessionBudget` has `usd_per_day` and no `trials_per_day`, so a lease
//! metered in trials has no day bound to apply. That is a fact about the
//! schema, and a lease that quietly behaved as though the day budget were
//! infinite would be indistinguishable from one that had applied it. The third
//! row is a failed measurement, and a failed measurement is never a number —
//! not a zero, and not an infinity either, which is the same mistake facing the
//! other way.
//!
//! # What a caller has to supply that this crate cannot read
//!
//! [`day_bound`] and [`total_bound`] map `SessionBudget` — which the driver
//! holds — onto two of the three caps. The third has no source in this crate
//! and is named rather than invented: `LeaseRequest::per_task` is §6.3's
//! *"capped by `max_usd_per_task` of the tier"*, and
//! [`crate::charter::Profile`] carries **neither `max_usd_per_task` nor
//! `max_trials_per_task`** — it holds `frontier`, `measured`,
//! `autonomous_threshold`, `judgement` and `supports`. Whoever wires this
//! reads the per-task cap off the model profile the tier names and passes it;
//! passing [`Bound::NotDenominated`] for it is a true statement only if that
//! profile really declares no per-task cap, and on a session whose other two
//! bounds are also undenominated it would leave a child's spend bounded by
//! nothing but the estimate.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::agenda::{dedupe_key, AgendaItem, ItemStatus, Source};
use crate::charter::Charter;
use crate::couplings::{Observed, PauseReason, Verdict};
use crate::digest::SpendUnit;
use crate::schedule::Openness;

/// §6.3: the lease is `estimate_cost × 1.5`.
///
/// The headroom is what lets a run that costs a little more than the median
/// finish rather than stopping one step from its report turn. It is not a
/// licence to borrow: §6.3's own next sentence is *"it may not borrow"*, and
/// the lease is still capped by everything below it.
pub const LEASE_HEADROOM: f64 = 1.5;

/// §6.2: *"the session's recent verdicts in scope (last five, one line each)"*.
pub const RECENT_VERDICTS: usize = 5;

// ── Quantities that carry their unit ────────────────────────────────────────

/// Why a quantity is not a budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MeterError {
    #[error("a budget of {unit} has to be a finite number, and this one is not")]
    NotFinite { unit: &'static str },
    #[error("a budget of {unit} cannot be negative")]
    Negative { unit: &'static str },
}

/// An amount **and** what it is counted in.
///
/// Sealed: the field is private and [`Meter::new`] is the only way in, so a
/// non-finite or negative budget cannot exist. `Serialize` only and no
/// `Deserialize`, deliberately — a type whose constructor enforces something
/// and whose `Deserialize` does not is a type with a second door, and
/// [`crate::charter::ExplorationShare`] is the crate's standing example of
/// closing it. Nothing needs to parse a `Meter` from JSON; if something ever
/// does, it gets a `try_from` and not a derive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Meter {
    amount: f64,
    unit: SpendUnit,
}

impl Meter {
    /// # Errors
    ///
    /// Refuses a non-finite or negative amount.
    pub fn new(amount: f64, unit: SpendUnit) -> Result<Self, MeterError> {
        if !amount.is_finite() {
            return Err(MeterError::NotFinite {
                unit: unit.in_prose(),
            });
        }
        if amount < 0.0 {
            return Err(MeterError::Negative {
                unit: unit.in_prose(),
            });
        }
        Ok(Self { amount, unit })
    }

    /// # Errors
    ///
    /// See [`Meter::new`].
    pub fn trials(amount: f64) -> Result<Self, MeterError> {
        Self::new(amount, SpendUnit::Trials)
    }

    /// # Errors
    ///
    /// See [`Meter::new`].
    pub fn usd(amount: f64) -> Result<Self, MeterError> {
        Self::new(amount, SpendUnit::Usd)
    }

    #[must_use]
    pub fn amount(self) -> f64 {
        self.amount
    }

    #[must_use]
    pub fn unit(self) -> SpendUnit {
        self.unit
    }

    /// What a human reads: `12.00 trials`, `3.40 USD`.
    #[must_use]
    pub fn in_prose(self) -> String {
        format!("{:.2} {}", self.amount, self.unit.in_prose())
    }
}

/// A cap the lease is bounded by, or a statement that there is not one.
///
/// See the module header's table. [`Bound::NotDenominated`] is a **declared**
/// absence — the platform holds no cap of this kind in this unit — and it
/// names the column that would have to exist for one, so the gap is a thing
/// somebody can close rather than a thing somebody has to notice.
#[derive(Debug, Clone, PartialEq)]
pub enum Bound {
    Cap(Meter),
    NotDenominated { wants: &'static str },
}

/// Which of §6.3's caps a lease was cut by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapKind {
    /// Not a cap: the lease is the estimate plus its headroom, and nothing
    /// below it was tighter.
    Estimate,
    /// The tier's `max_usd_per_task`.
    PerTask,
    /// §7 row 2's day budget.
    Day,
    /// The session's total budget.
    Total,
}

impl CapKind {
    /// In the order [`lease`] applies them, which is also the order a tie is
    /// broken in: the loosest statement of intent first, then the three real
    /// ceilings from the narrowest scope outwards.
    pub const ALL: &'static [CapKind] = &[
        CapKind::Estimate,
        CapKind::PerTask,
        CapKind::Day,
        CapKind::Total,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CapKind::Estimate => "estimate",
            CapKind::PerTask => "per_task",
            CapKind::Day => "day",
            CapKind::Total => "total",
        }
    }

    /// What the digest calls it.
    #[must_use]
    pub fn in_prose(self) -> &'static str {
        match self {
            CapKind::Estimate => "the workflow's own estimate",
            CapKind::PerTask => "the tier's per-task cap",
            CapKind::Day => "the day budget",
            CapKind::Total => "the session's total budget",
        }
    }
}

/// Why a child could not be given a lease.
///
/// Every variant is a refusal to start a run, which is the safe direction:
/// §6.3's *"it may not borrow"* has no meaning if a lease can be issued
/// without knowing what is left.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LeaseRefusal {
    #[error(
        "this lease is denominated in {wanted} and {what} is in {got}. There is no conversion \
         between them on this platform: `llm_usage.cost_usd` keys on the auth session rather \
         than on the run, so no price per run exists to convert with. Denominate the request in \
         one unit, or add the per-run price."
    )]
    UnitMismatch {
        what: String,
        wanted: &'static str,
        got: &'static str,
    },
    #[error(
        "{what} was not read this iteration, so the size of this lease is not something anybody \
         knows. A child is not started on an unread budget: an unread number is not a zero and \
         it is not an infinity either."
    )]
    NotRead { what: String },
    #[error(
        "{} is spent: the cap is {cap} and {spent} is already committed, {in_flight} of it by \
         children still running. Nothing is claimed until it resets.",
        .which.in_prose()
    )]
    Exhausted {
        which: CapKind,
        cap: String,
        spent: String,
        in_flight: String,
    },
    #[error(
        "the lease would be {size}, which buys no steps. A child with nothing to spend cannot \
         even reach its report turn, so it is not started — the estimate or the remaining budget \
         is the thing to look at."
    )]
    WouldBeZero { size: String },
    #[error(
        "nothing bounds this lease: {missing}. The estimate and its headroom are a guess at what \
         the child will want, not a ceiling on what it may take, so a child issued this would be \
         an unbounded run wearing a lease's name. Supply one real cap — the tier's per-task cap \
         is the one that exists on every profile."
    )]
    NothingBounds { missing: String },
    #[error("{0}")]
    Unusable(String),
}

/// How many trials the child may take (§6.3: *"the child's `max_trials_per_task`
/// is the lease's trial count"*).
///
/// Two states and no escape hatch — no `Option`, no `unwrap_or`, no
/// `From<TrialCap> for i64`. A caller has to type the second arm and decide
/// what it says, which is the same shape `harness::Spend` takes for the same
/// reason: the one method call that turns "nobody can price this" into a number
/// is the one that must not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrialCap {
    /// The lease is metered in trials, so the trial count is the lease.
    Trials { max_trials_per_task: i64 },
    /// The lease is metered in USD and no per-run price exists, so no trial
    /// count follows from it.
    Unpriceable { because: &'static str },
}

/// A child's budget (§6.3).
///
/// Sealed: every field is private and [`lease`] is the only constructor, so a
/// lease that was never measured against the session's remaining budget cannot
/// be constructed — including by a test, which is where the shortcut would
/// otherwise be taken.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Lease {
    session: Uuid,
    item: Uuid,
    size: Meter,
    capped_by: CapKind,
    /// Every cap that does not exist in this lease's unit, so a reader can tell
    /// a lease bounded by three ceilings from one bounded by two. Empty is the
    /// good case and is **not** the case on this platform today.
    unbounded_by: Vec<CapKind>,
}

impl Lease {
    #[must_use]
    pub fn session(&self) -> Uuid {
        self.session
    }

    #[must_use]
    pub fn item(&self) -> Uuid {
        self.item
    }

    #[must_use]
    pub fn size(&self) -> Meter {
        self.size
    }

    #[must_use]
    pub fn capped_by(&self) -> CapKind {
        self.capped_by
    }

    #[must_use]
    pub fn unbounded_by(&self) -> &[CapKind] {
        &self.unbounded_by
    }

    /// §6.3's last clause.
    #[must_use]
    pub fn max_trials_per_task(&self) -> TrialCap {
        match self.size.unit {
            // Floor: a fractional trial is not a trial, and rounding up would
            // hand out budget the lease does not have.
            SpendUnit::Trials => TrialCap::Trials {
                max_trials_per_task: self.size.amount.floor() as i64,
            },
            SpendUnit::Usd => TrialCap::Unpriceable {
                because: "the lease is in USD and `llm_usage.cost_usd` keys on the auth session \
                          rather than on the run, so no price per trial exists to divide by",
            },
        }
    }

    /// What `run_started` records about the lease.
    #[must_use]
    pub fn event_payload(&self) -> serde_json::Value {
        json!({
            "size": self.size.amount,
            "unit": self.size.unit.as_str(),
            "capped_by": self.capped_by.as_str(),
            "unbounded_by": self.unbounded_by.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "max_trials_per_task": self.max_trials_per_task(),
        })
    }

    /// One sentence for the digest.
    #[must_use]
    pub fn sentence(&self) -> String {
        let base = format!(
            "{} leased, cut by {}",
            self.size.in_prose(),
            self.capped_by.in_prose()
        );
        if self.unbounded_by.is_empty() {
            return base;
        }
        format!(
            "{base}; not bounded by {} — no cap of that kind is denominated in {}",
            self.unbounded_by
                .iter()
                .map(|c| c.in_prose())
                .collect::<Vec<_>>()
                .join(" or "),
            self.size.unit.in_prose()
        )
    }
}

/// Everything §6.3 needs to size one child's lease.
///
/// Deliberately a struct of already-read facts rather than a trait with async
/// readers, like [`crate::sources::World`] and for the same reason: the rule is
/// a decision and decisions test without a database.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaseRequest {
    pub session: Uuid,
    pub item: Uuid,
    /// The one unit every quantity in this request is in. Declared rather than
    /// inferred from the first `Meter` that turns up, so a request that mixes
    /// two units is refused rather than silently adopting whichever was read
    /// first.
    pub unit: SpendUnit,
    /// The workflow's `estimate_cost` on this tier.
    pub estimate: Observed<Meter>,
    /// The tier's `max_usd_per_task`, or a statement that the tier declares no
    /// per-task cap.
    pub per_task: Observed<Bound>,
    /// §7 row 2's day budget, and what the session has already spent against it
    /// today. `spent_today` is read only when `day` is a [`Bound::Cap`].
    pub day: Observed<Bound>,
    pub spent_today: Observed<Meter>,
    /// The session's total budget and what it has spent against it.
    pub total: Observed<Bound>,
    pub spent_total: Observed<Meter>,
    /// The leases of this session's children that are still running.
    ///
    /// §12.4: *"three concurrent children cannot together exceed"* the day
    /// budget. That is this field: each child's whole lease is committed the
    /// moment it is issued, not as it is drawn down, because a fourth child
    /// sized against what the first three have *so far* spent is a fourth child
    /// that can push the session past the cap.
    pub in_flight: Vec<Lease>,
}

/// Size a child's lease (AGENT-014 §6.3).
///
/// # Errors
///
/// Refuses a mismatched unit, an unread quantity, an exhausted cap and a lease
/// that would come out at zero. See [`LeaseRefusal`].
pub fn lease(req: &LeaseRequest) -> Result<Lease, LeaseRefusal> {
    let unit = req.unit;
    let want =
        check_meter(&req.estimate, "the workflow's cost estimate", unit)?.amount * LEASE_HEADROOM;

    let mut unbounded = Vec::new();
    // `(kind, headroom)` pairs, in `CapKind::ALL`'s order, which is the order a
    // tie is broken in.
    let mut candidates = vec![(CapKind::Estimate, want)];

    let committed = in_flight_total(&req.in_flight, unit)?;

    for (kind, bound, spent) in [
        (CapKind::PerTask, &req.per_task, None),
        (
            CapKind::Day,
            &req.day,
            Some((&req.spent_today, "the session's spend today")),
        ),
        (
            CapKind::Total,
            &req.total,
            Some((&req.spent_total, "the session's total spend")),
        ),
    ] {
        let bound = match bound {
            Observed::Is(b) => b,
            Observed::Unknown { why } => {
                return Err(LeaseRefusal::NotRead {
                    what: format!("{} ({why})", kind.in_prose()),
                })
            }
        };
        let cap = match bound {
            Bound::Cap(m) => {
                check_unit(*m, kind.in_prose(), unit)?;
                *m
            }
            Bound::NotDenominated { .. } => {
                unbounded.push(kind);
                continue;
            }
        };
        // A per-task cap is a ceiling on one child and has nothing spent
        // against it; the other two are ceilings on the session.
        let already = match spent {
            None => 0.0,
            Some((observed, what)) => check_meter(observed, what, unit)?.amount + committed,
        };
        let headroom = cap.amount - already;
        if headroom <= 0.0 {
            return Err(LeaseRefusal::Exhausted {
                which: kind,
                cap: cap.in_prose(),
                spent: Meter::new(already.max(0.0), unit)
                    .map_err(|e| LeaseRefusal::Unusable(e.to_string()))?
                    .in_prose(),
                in_flight: Meter::new(committed, unit)
                    .map_err(|e| LeaseRefusal::Unusable(e.to_string()))?
                    .in_prose(),
            });
        }
        candidates.push((kind, headroom));
    }

    // §6.3 is a rule about a ceiling. If every ceiling turned out to be a
    // declared absence, the only thing left is the estimate, and an estimate is
    // not a ceiling — it is this module's own guess at what the child wants.
    if unbounded.len() == CapKind::ALL.len() - 1 {
        return Err(LeaseRefusal::NothingBounds {
            missing: unbounded
                .iter()
                .map(|c| c.in_prose())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }

    let (capped_by, size) = candidates
        .iter()
        .copied()
        .reduce(|a, b| if b.1 < a.1 { b } else { a })
        .expect("the estimate is always a candidate");
    let size = Meter::new(size, unit).map_err(|e| LeaseRefusal::Unusable(e.to_string()))?;
    if size.amount <= 0.0 {
        return Err(LeaseRefusal::WouldBeZero {
            size: size.in_prose(),
        });
    }
    Ok(Lease {
        session: req.session,
        item: req.item,
        size,
        capped_by,
        unbounded_by: unbounded,
    })
}

fn check_unit(m: Meter, what: &str, unit: SpendUnit) -> Result<(), LeaseRefusal> {
    if m.unit == unit {
        return Ok(());
    }
    Err(LeaseRefusal::UnitMismatch {
        what: what.to_string(),
        wanted: unit.in_prose(),
        got: m.unit.in_prose(),
    })
}

fn check_meter(
    observed: &Observed<Meter>,
    what: &str,
    unit: SpendUnit,
) -> Result<Meter, LeaseRefusal> {
    match observed {
        Observed::Is(m) => {
            check_unit(*m, what, unit)?;
            Ok(*m)
        }
        Observed::Unknown { why } => Err(LeaseRefusal::NotRead {
            what: format!("{what} ({why})"),
        }),
    }
}

fn in_flight_total(leases: &[Lease], unit: SpendUnit) -> Result<f64, LeaseRefusal> {
    let mut total = 0.0;
    for l in leases {
        check_unit(l.size, "a lease held by a child still running", unit)?;
        total += l.size.amount;
    }
    Ok(total)
}

// ── The session row's own budgets, read as bounds ───────────────────────────
//
// `SessionBudget` (§2.1) holds three numbers and they are not in one unit:
// `usd_total` and `usd_per_day` are dollars, `trials_total` is trials. Which
// of them bounds a lease therefore depends on what the lease is metered in,
// and for two of the four combinations the answer is *nothing does*. These two
// functions are the one place that mapping is written, so the driver does not
// have to decide it at the call site and get it different each time.

/// §2.1's day budget, as a bound on a lease metered in `unit`.
///
/// # Errors
///
/// Refuses a stored budget that is not a finite non-negative number. The column
/// is a `jsonb` and nothing between the charter form and here would otherwise
/// stop a `NaN`.
pub fn day_bound(
    budget: &crate::charter::SessionBudget,
    unit: SpendUnit,
) -> Result<Bound, MeterError> {
    match unit {
        SpendUnit::Usd => Ok(Bound::Cap(Meter::usd(budget.usd_per_day)?)),
        // There is no trials-per-day figure anywhere in this platform.
        // `usd_per_day` is the only per-day number and it is in dollars, which
        // cannot cap a trial count without a per-run price — the thing
        // `llm_usage.cost_usd` does not provide.
        SpendUnit::Trials => Ok(Bound::NotDenominated {
            wants: "a per-day budget in trials; `SessionBudget` has `usd_per_day` only, and \
                    converting it needs a per-run price this platform does not hold",
        }),
    }
}

/// §2.1's total budget, as a bound on a lease metered in `unit`.
///
/// **`trials_total = 0` is read as "nobody said", not as "no trials".** That
/// is what the column actually means — `routes::desk_charter::translate`
/// stores a `0` for a blank `max_trials` field and the desk renders it as *no
/// cap recorded* — and the two readings are a mile apart: as a cap it would
/// exhaust the session before its first child, and as an absence it correctly
/// leaves the total unbounded in trials and says so on every lease.
///
/// # Errors
///
/// Refuses a stored budget that is not a finite non-negative number.
pub fn total_bound(
    budget: &crate::charter::SessionBudget,
    unit: SpendUnit,
) -> Result<Bound, MeterError> {
    match unit {
        SpendUnit::Usd => Ok(Bound::Cap(Meter::usd(budget.usd_total)?)),
        SpendUnit::Trials if budget.trials_total > 0 => {
            Ok(Bound::Cap(Meter::trials(budget.trials_total as f64)?))
        }
        SpendUnit::Trials => Ok(Bound::NotDenominated {
            wants: "`research_sessions.budget.trials_total`, which §2.1 fills from the tier's \
                    `max_trials_per_task` × `max_runs`; a stored 0 is the blank form field, not \
                    a cap of none",
        }),
    }
}

// ── The day budget (§7 row 2) ───────────────────────────────────────────────

/// Where the session stands against its day budget.
///
/// Four states, and the last two are the module header's table again: a day
/// budget nobody read is not a day budget with room in it, and a day budget
/// that does not exist in this unit is not one either — but they lead to
/// different behaviour, so they are different words.
#[derive(Debug, Clone, PartialEq)]
pub enum DayBudget {
    Remaining(Meter),
    Exhausted { cap: Meter, spent: Meter },
    NotDenominated { wants: &'static str },
    Unreadable { why: String },
}

impl DayBudget {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            DayBudget::Remaining(_) => "remaining",
            DayBudget::Exhausted { .. } => "exhausted",
            DayBudget::NotDenominated { .. } => "not_denominated",
            DayBudget::Unreadable { .. } => "unreadable",
        }
    }

    /// Whether the driver may claim on this alone.
    ///
    /// [`DayBudget::NotDenominated`] answers **yes**, and that is a decision
    /// rather than an oversight: today no day budget exists in trials, and a
    /// session that refused to claim until one did would be a continuous
    /// research feature that never ran. What stops it being silent is that
    /// [`lease`] records the same gap on every lease it issues, so the fact
    /// that nothing bounded the day rides on the `run_started` event and into
    /// the digest rather than living in this comment.
    #[must_use]
    pub fn may_claim(&self) -> bool {
        matches!(
            self,
            DayBudget::Remaining(_) | DayBudget::NotDenominated { .. }
        )
    }
}

/// Read the day budget the way [`lease`] reads it, so the gate and the lease
/// cannot disagree (§7 row 2).
#[must_use]
pub fn day_budget(
    cap: &Observed<Bound>,
    spent_today: &Observed<Meter>,
    in_flight: &[Lease],
    unit: SpendUnit,
) -> DayBudget {
    let bound = match cap {
        Observed::Is(b) => b,
        Observed::Unknown { why } => {
            return DayBudget::Unreadable {
                why: format!("the day budget was not read: {why}"),
            }
        }
    };
    let cap = match bound {
        Bound::Cap(m) => *m,
        Bound::NotDenominated { wants } => return DayBudget::NotDenominated { wants },
    };
    if cap.unit != unit {
        return DayBudget::Unreadable {
            why: format!(
                "the day budget is in {} and the session meters runs in {}, and there is no \
                 conversion between them on this platform",
                cap.unit.in_prose(),
                unit.in_prose()
            ),
        };
    }
    let spent = match check_meter(spent_today, "the session's spend today", unit) {
        Ok(m) => m,
        Err(e) => return DayBudget::Unreadable { why: e.to_string() },
    };
    let committed: f64 = in_flight.iter().map(|l| l.size.amount).sum();
    let used = spent.amount + committed;
    let left = cap.amount - used;
    if left <= 0.0 {
        return DayBudget::Exhausted {
            cap,
            spent: Meter::new(used.max(0.0), unit).unwrap_or(cap),
        };
    }
    Meter::new(left, unit).map_or(
        DayBudget::Unreadable {
            why: "the remaining day budget is not a finite number".to_string(),
        },
        DayBudget::Remaining,
    )
}

// ── When the session is finished (§7 row 3) ─────────────────────────────────
//
// `StopWhen` has been a vocabulary with **no reader** since it was written:
// five words, a `Serialize`, and nothing anywhere that evaluates one. §7 row 3
// — *"Total budget / `max_runs` / `stop_when` met → `complete` after the final
// digest"* — is the rule, and it is two rules rather than one: the first two
// are the platform's and hold whatever the charter says, and the third is the
// charter's own list.
//
// Row 3 is **not** row 4. Row 4 is the drained agenda, which
// [`crate::replenish::on_drain`] decides because a drained agenda under
// `replenish = true` is not an ending at all. A charter that also lists
// `agenda_drained` is asking for the same outcome by a second route, and
// `stop` reads the drain it is handed rather than deciding it again.

/// One reason the session is finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reached {
    /// The charter condition, or `None` for the two the platform holds
    /// regardless of what the charter lists.
    pub condition: Option<crate::charter::StopWhen>,
    pub word: &'static str,
    pub sentence: String,
}

/// A stop condition nobody can evaluate.
///
/// Carried rather than dropped: a charter listing a condition this platform
/// cannot measure is a charter whose author believes the session will stop and
/// it will not. That belongs in the digest, not in a silence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unevaluable {
    pub condition: crate::charter::StopWhen,
    pub word: &'static str,
    pub missing: String,
}

/// What §7 row 3 says about the session right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StopVerdict {
    pub reached: Vec<Reached>,
    pub not_evaluable: Vec<Unevaluable>,
}

impl StopVerdict {
    /// Whether the session completes — *after the final digest*, which is the
    /// caller's ordering and not something this function can enforce.
    #[must_use]
    pub fn completes(&self) -> bool {
        !self.reached.is_empty()
    }

    /// One sentence per reason, for the digest.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.reached
            .iter()
            .map(|r| r.sentence.clone())
            .chain(self.not_evaluable.iter().map(|u| {
                format!(
                    "`{}` cannot be evaluated on this platform, so it will never stop this \
                     session: {}",
                    u.word, u.missing
                )
            }))
            .collect()
    }
}

/// Everything §7 row 3 reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Stopping {
    /// `Folded::runs_started`.
    pub runs_started: i64,
    /// `SessionBudget::max_runs`. A value at or below zero is **"nobody
    /// said"**, not "stop at once" — the same reading `total_bound` gives
    /// `trials_total = 0`, and for the same reason: it is what a blank form
    /// field stores.
    pub max_runs: i32,
    pub unit: SpendUnit,
    pub total: Observed<Bound>,
    pub spent_total: Observed<Meter>,
    /// What [`crate::replenish::on_drain`] decided, if the agenda drained.
    pub agenda_drained: bool,
    /// Whether [`crate::stall::detect`] returned a diagnosis.
    pub stalled: bool,
    /// How many deployable strategies the session has found, for
    /// [`crate::charter::StopWhen::DeployableFound`].
    pub deployable_found: Observed<u32>,
}

/// Is the session finished? (AGENT-014 §7 row 3.)
#[must_use]
pub fn stop(conditions: &[crate::charter::StopWhen], s: &Stopping) -> StopVerdict {
    use crate::charter::StopWhen;

    let mut reached = Vec::new();
    let mut not_evaluable = Vec::new();

    // The two the platform holds whatever the charter lists.
    if s.max_runs > 0 && s.runs_started >= i64::from(s.max_runs) {
        reached.push(Reached {
            condition: None,
            word: "max_runs",
            sentence: format!(
                "{} runs have started and the session's `max_runs` is {}",
                s.runs_started, s.max_runs
            ),
        });
    }
    let budget = total_budget_met(s);

    if let Met::Yes { because } = &budget {
        reached.push(Reached {
            condition: None,
            word: "total_budget",
            sentence: because.clone(),
        });
    }

    for c in conditions {
        let met = match c {
            StopWhen::Budget => budget.clone(),
            // `Schedule` is `{windows: [{days, start, end, tz}]}` and a window
            // recurs weekly forever. There is no end date anywhere in the
            // object, so this condition has nothing to compare against — it is
            // reported rather than quietly never firing.
            StopWhen::ScheduleEnd => Met::NotEvaluable {
                missing: "`charter::Schedule` has weekly windows and no end date, so there is \
                          no instant a schedule ends at"
                    .to_string(),
            },
            StopWhen::AgendaDrained => {
                if s.agenda_drained {
                    Met::Yes {
                        because: "the agenda drained and the charter asks to stop there"
                            .to_string(),
                    }
                } else {
                    Met::No
                }
            }
            StopWhen::Stall => {
                if s.stalled {
                    Met::Yes {
                        because: "the stall detector returned a diagnosis and the charter asks \
                                  to stop rather than wait for a human"
                            .to_string(),
                    }
                } else {
                    Met::No
                }
            }
            StopWhen::DeployableFound { k } => match s.deployable_found.get() {
                Some(found) if found >= k => Met::Yes {
                    because: format!(
                        "{found} deployable strategies found; the charter asked for {k}"
                    ),
                },
                Some(_) => Met::No,
                None => Met::NotEvaluable {
                    missing: match &s.deployable_found {
                        Observed::Unknown { why } => why.clone(),
                        Observed::Is(_) => unreachable!("`get` answered None"),
                    },
                },
            },
        };
        match met {
            Met::Yes { because } => {
                // `total_budget` is already in the list when the charter also
                // names `budget`; one reason, not two.
                if !reached.iter().any(|r| r.sentence == because) {
                    reached.push(Reached {
                        condition: Some(*c),
                        word: stop_word(*c),
                        sentence: because,
                    });
                }
            }
            Met::No => {}
            Met::NotEvaluable { missing } => not_evaluable.push(Unevaluable {
                condition: *c,
                word: stop_word(*c),
                missing,
            }),
        }
    }

    StopVerdict {
        reached,
        not_evaluable,
    }
}

/// Whether one condition holds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Met {
    Yes {
        because: String,
    },
    No,
    /// Nothing on this platform can answer it. **Never folded into
    /// [`Met::No`]:** a condition that cannot fire and one that has not fired
    /// look identical on an agenda and completely different to somebody
    /// wondering why a session is still running.
    NotEvaluable {
        missing: String,
    },
}

fn total_budget_met(s: &Stopping) -> Met {
    let bound = match &s.total {
        Observed::Is(b) => b,
        Observed::Unknown { why } => {
            return Met::NotEvaluable {
                missing: format!("the session's total budget was not read: {why}"),
            }
        }
    };
    let cap = match bound {
        Bound::Cap(m) if m.unit == s.unit => *m,
        Bound::Cap(m) => {
            return Met::NotEvaluable {
                missing: format!(
                    "the total budget is in {} and the session meters runs in {}",
                    m.unit.in_prose(),
                    s.unit.in_prose()
                ),
            }
        }
        Bound::NotDenominated { wants } => {
            return Met::NotEvaluable {
                missing: (*wants).to_string(),
            }
        }
    };
    match check_meter(&s.spent_total, "the session's total spend", s.unit) {
        Ok(spent) if spent.amount >= cap.amount => Met::Yes {
            because: format!(
                "the session has spent {} of a total budget of {}",
                spent.in_prose(),
                cap.in_prose()
            ),
        },
        Ok(_) => Met::No,
        Err(e) => Met::NotEvaluable {
            missing: e.to_string(),
        },
    }
}

/// The word a `stop_when` condition records.
#[must_use]
pub fn stop_word(c: crate::charter::StopWhen) -> &'static str {
    use crate::charter::StopWhen;
    match c {
        StopWhen::Budget => "budget",
        StopWhen::ScheduleEnd => "schedule_end",
        StopWhen::AgendaDrained => "agenda_drained",
        StopWhen::Stall => "stall",
        StopWhen::DeployableFound { .. } => "n_deployable_found",
    }
}

// ── What the driver does this iteration (§7 rows 1–2, 8–10) ─────────────────

/// When live children drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainAt {
    /// The boundary has already passed, or the reason is not a calendar one.
    Now,
    /// The window closes then — the instant `run_started` records, so a child
    /// knows its own boundary without re-reading the calendar.
    At(DateTime<Utc>),
}

/// Why nothing is claimed.
#[derive(Debug, Clone, PartialEq)]
pub enum HoldReason {
    /// §7 row 1.
    WindowClosed,
    /// §7 row 2.
    DayBudgetExhausted,
    /// The day budget could not be read. **Not** a pause: a failed read is a
    /// failed iteration, the same argument [`crate::couplings::Verdict::CannotDecide`]
    /// makes, and appending a pause would leave a human to clear something
    /// nobody caused.
    DayBudgetUnreadable { why: String },
    /// §7 rows 8–10.
    Paused(Vec<PauseReason>),
    /// A coupling could not be read.
    CannotDecide { missing: Vec<String> },
    /// The calendar parses but its next boundary is past the horizon.
    ScheduleIndeterminate,
    /// `max_concurrent_runs` is used up.
    NoSlots,
    /// The agenda has nothing claimable. Whether that is `complete` or a
    /// replenish is [`crate::replenish::on_drain`]'s decision, not this one.
    NothingClaimable,
}

impl HoldReason {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            HoldReason::WindowClosed => "window_closed",
            HoldReason::DayBudgetExhausted => "day_budget_exhausted",
            HoldReason::DayBudgetUnreadable { .. } => "day_budget_unreadable",
            HoldReason::Paused(_) => "paused",
            HoldReason::CannotDecide { .. } => "cannot_decide",
            HoldReason::ScheduleIndeterminate => "schedule_indeterminate",
            HoldReason::NoSlots => "no_slots",
            HoldReason::NothingClaimable => "nothing_claimable",
        }
    }

    /// Whether this is a state the log records, or a failed iteration.
    ///
    /// The same asymmetry [`crate::couplings`] draws and for the same reason:
    /// a pause a human has to clear is worth appending; an iteration that could
    /// not read something is not.
    #[must_use]
    pub fn is_a_transition(&self) -> bool {
        matches!(
            self,
            HoldReason::WindowClosed | HoldReason::DayBudgetExhausted | HoldReason::Paused(_)
        )
    }
}

/// One thing the driver does, in the order [`plan`] returns them.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// §7 row 1: live children finish the current step and suspend.
    Drain { at: DrainAt, children: Vec<Uuid> },
    /// §7 row 1: *"the next window resumes suspended children **before**
    /// claiming anything new"*. That ordering is the whole reason this is a
    /// list rather than a struct of flags.
    Resume(Vec<Uuid>),
    /// The driver may start this many children.
    Claim { slots: usize },
    /// Nothing is claimed.
    Hold {
        why: HoldReason,
        re_arms_at: Option<DateTime<Utc>>,
    },
}

impl Step {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Step::Drain { .. } => "drain",
            Step::Resume(_) => "resume",
            Step::Claim { .. } => "claim",
            Step::Hold { .. } => "hold",
        }
    }
}

/// Everything §7 rows 1–2 and 8–10 need, already read.
///
/// `state` is absent on purpose: §3's loop gates on it before any of this, and
/// a second copy of "is this session running" here would be a second place for
/// a stalled session to be allowed to claim.
#[derive(Debug, Clone, PartialEq)]
pub struct Situation {
    pub openness: Openness,
    pub couplings: Verdict,
    pub day_budget: DayBudget,
    /// Children suspended at a previous boundary, oldest first.
    pub suspended: Vec<Uuid>,
    /// Children running right now.
    pub live: Vec<Uuid>,
    pub max_concurrent: i16,
    /// How many agenda items are claimable.
    pub claimable: usize,
}

/// What the driver does this iteration, in order (AGENT-014 §7).
///
/// The order of the checks is severity, and it is not arbitrary. The couplings
/// come first because they are about the world rather than about the session —
/// a kill switch tripping outranks a window that happens to be open. The day
/// budget comes before the calendar because §7 row 2 says it behaves *"as
/// above"* and then resumes on the next **day's** window, which is a longer
/// wait than the next window: reporting `window_closed` for a session that is
/// actually out of budget would re-arm it on a boundary that changes nothing.
#[must_use]
pub fn plan(s: &Situation) -> Vec<Step> {
    let mut steps = Vec::new();

    match &s.couplings {
        Verdict::Paused(reasons) => {
            // §7 rows 8–10 do not all drain: the kill switch and the defect
            // breaker suspend live children, and a degraded tier lets the ones
            // in flight finish.
            if !s.live.is_empty() && reasons.iter().any(PauseReason::drains_live_children) {
                steps.push(Step::Drain {
                    at: DrainAt::Now,
                    children: s.live.clone(),
                });
            }
            steps.push(Step::Hold {
                why: HoldReason::Paused(reasons.clone()),
                re_arms_at: None,
            });
            return steps;
        }
        Verdict::CannotDecide { missing } => {
            // No drain. See `HoldReason::DayBudgetUnreadable`.
            steps.push(Step::Hold {
                why: HoldReason::CannotDecide {
                    missing: missing.clone(),
                },
                re_arms_at: None,
            });
            return steps;
        }
        Verdict::Clear => {}
    }

    match &s.day_budget {
        DayBudget::Exhausted { .. } => {
            if !s.live.is_empty() {
                steps.push(Step::Drain {
                    at: DrainAt::Now,
                    children: s.live.clone(),
                });
            }
            steps.push(Step::Hold {
                why: HoldReason::DayBudgetExhausted,
                // §7 row 2: the **next day's** window, which the caller gets
                // from `OpenHours::next_open_on_a_later_day` — a different
                // question from `next_open_after`, and the calendar answers it
                // rather than this module guessing a midnight.
                re_arms_at: None,
            });
            return steps;
        }
        DayBudget::Unreadable { why } => {
            steps.push(Step::Hold {
                why: HoldReason::DayBudgetUnreadable { why: why.clone() },
                re_arms_at: None,
            });
            return steps;
        }
        DayBudget::Remaining(_) | DayBudget::NotDenominated { .. } => {}
    }

    match s.openness {
        Openness::Closed { opens_at } => {
            if !s.live.is_empty() {
                steps.push(Step::Drain {
                    at: DrainAt::Now,
                    children: s.live.clone(),
                });
            }
            steps.push(Step::Hold {
                why: HoldReason::WindowClosed,
                re_arms_at: Some(opens_at),
            });
            return steps;
        }
        Openness::Indeterminate => {
            steps.push(Step::Hold {
                why: HoldReason::ScheduleIndeterminate,
                re_arms_at: None,
            });
            return steps;
        }
        Openness::Open { closes_at } => {
            if !s.live.is_empty() {
                steps.push(Step::Drain {
                    at: DrainAt::At(closes_at),
                    children: s.live.clone(),
                });
            }
        }
        Openness::AlwaysOpen => {}
    }

    if !s.suspended.is_empty() {
        steps.push(Step::Resume(s.suspended.clone()));
    }

    let concurrency = usize::try_from(s.max_concurrent.max(0)).unwrap_or(0);
    // A resumed child occupies a slot the moment it resumes, which is what
    // "resumes them before claiming anything new" costs: the session that comes
    // back to two suspended children and a concurrency of two claims nothing
    // this iteration, and that is correct.
    let taken = s.live.len() + s.suspended.len();
    let free = concurrency.saturating_sub(taken);
    let slots = free.min(s.claimable);
    if slots > 0 {
        steps.push(Step::Claim { slots });
    } else {
        steps.push(Step::Hold {
            why: if free == 0 {
                HoldReason::NoSlots
            } else {
                HoldReason::NothingClaimable
            },
            re_arms_at: s.openness.re_arms_at(),
        });
    }
    steps
}

// ── The brief (§6.2) ────────────────────────────────────────────────────────

/// A standing constraint, as the `user_preference` row the brief reads
/// (AGENT-007 §3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preference {
    pub scope: String,
    pub text: String,
}

/// The scope a session's standing constraints are written under (§2.2).
///
/// One spelling, here, because the writer and the reader are in different
/// crates and a second `format!` is a second spelling waiting to happen.
#[must_use]
pub fn preference_scope(session: Uuid) -> String {
    format!("session:{session}")
}

/// A sibling run, as §6.2's *"recent verdicts in scope"* needs to see it.
#[derive(Debug, Clone, PartialEq)]
pub struct SiblingRun {
    pub item: Uuid,
    pub run: Uuid,
    /// What the run was about, which is what "in scope" is decided on.
    pub subject: String,
    pub question: String,
    /// The verdict word, from the run drivers' own vocabulary.
    pub verdict: String,
}

/// What a child is told before it starts (§6.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Brief {
    /// The charter's standing constraints, at top rank.
    pub preferences: Vec<Preference>,
    /// The session's last five verdicts in scope, one line each.
    pub recent: Vec<String>,
    /// Why this item is running now.
    pub why_now: String,
}

/// Everything [`brief`] reads.
#[derive(Debug, Clone, PartialEq)]
pub struct BriefInput<'a> {
    pub session: Uuid,
    pub charter: &'a Charter,
    pub item: &'a AgendaItem,
    /// The session's finished runs, **oldest first** — the same ordering
    /// [`crate::couplings::Signals::children`] takes, so a caller holding one
    /// log does not have to reverse it for one reader and not the other.
    pub siblings: &'a [SiblingRun],
}

/// Compose a child's brief (AGENT-014 §6.2).
#[must_use]
pub fn brief(input: &BriefInput) -> Brief {
    let scope = preference_scope(input.session);
    let preferences = input
        .charter
        .standing_constraints
        .iter()
        .map(|text| Preference {
            scope: scope.clone(),
            text: text.clone(),
        })
        .collect();

    let mut recent: Vec<String> = input
        .siblings
        .iter()
        .rev()
        .filter(|s| s.item != input.item.id)
        .filter(|s| input.charter.scope.admits(&s.subject))
        .take(RECENT_VERDICTS)
        .map(|s| format!("`{}` — {}: {}", s.run, s.verdict, s.question))
        .collect();
    recent.shrink_to_fit();

    Brief {
        preferences,
        recent,
        why_now: why_now(input.item),
    }
}

/// §6.2's third ingredient: *"the item's `source`"*.
///
/// The spec's example sentence is *"you are running because finding f_31a is 94
/// days stale"*. The **94** is not written here and will not be: no source
/// carries an age, and a number invented for a sentence is still a number
/// somebody will quote. The reference is carried instead, which is what the
/// child needs to go and look.
#[must_use]
pub fn why_now(item: &AgendaItem) -> String {
    let source = item.sources.first().copied().unwrap_or(Source::CharterSeed);
    let refs = item.source_refs.join(", ");
    let cite = if refs.is_empty() {
        // Not silence: a source with no reference is a source the child cannot
        // follow, and saying so is how it stops being invisible.
        " (no reference was recorded, so there is nothing here to open)".to_string()
    } else {
        format!(" ({refs})")
    };
    let because = match source {
        Source::CharterSeed => "the charter names this question as a seed".to_string(),
        Source::User => "a human put this question on the agenda".to_string(),
        Source::OpenQuestion => "an open question in scope is unanswered".to_string(),
        Source::OpenQuestionBlocking => {
            "an open question in scope is marked blocking, so something else is waiting on it"
                .to_string()
        }
        Source::RetryConditionMet => {
            "a negative result's retry condition now evaluates true, so what blocked it \
             may not any more"
                .to_string()
        }
        Source::Contradiction => {
            "two findings in scope contradict each other and nothing has resolved them".to_string()
        }
        Source::StaleFinding => {
            "an established finding in scope is past its `valid_until` and wants re-verifying"
                .to_string()
        }
        Source::Observation => {
            "the world changed in scope — a regime flip, new coverage, or a monitor event"
                .to_string()
        }
        Source::RefinementFollowup => {
            "a finished run in this session suggested this as its next move".to_string()
        }
        Source::ProposalFromRun => {
            "a finished run in this session proposed this question".to_string()
        }
        Source::Replenish => {
            "the agenda drained and a survey over the charter proposed this".to_string()
        }
    };
    let mut sentence = format!("you are running because {because}{cite}");
    if item.sources.len() > 1 {
        let others: Vec<&str> = item.sources[1..].iter().map(|s| s.as_str()).collect();
        sentence.push_str(&format!(
            ". {} other source{} wanted the same question: {}",
            others.len(),
            if others.len() == 1 { "" } else { "s" },
            others.join(", ")
        ));
    }
    sentence.push('.');
    sentence
}

// ── The redirecting steer (§6.4) ────────────────────────────────────────────

/// Why a redirect was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RedirectError {
    #[error(
        "a redirect has to say what the run is now for, and this one is blank. An item with an \
         empty question keys on nothing and would merge with every other empty one."
    )]
    Blank,
    #[error(
        "item is `{status}` and a finished item's question is the record of what ran. Rewriting \
         it would make the agenda disagree with the ledger about what question produced which \
         verdict."
    )]
    Terminal { status: &'static str },
    #[error("the steer does not redirect: the question is already `{question}`")]
    NoChange { question: String },
}

/// What a redirecting steer changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Redirect {
    pub item: Uuid,
    pub from: String,
    pub to: String,
    pub from_key: String,
    pub to_key: String,
}

/// §6.4: a steer that redirects the child's goal also rewrites the item's
/// question, *"so the agenda does not lie about what ran"*.
///
/// The **dedupe key moves with it**, and that is the part a caller would
/// forget: the key is the normalised question, layer one compares keys, and an
/// item left keyed on the question it is no longer asking would merge the old
/// question's next offer onto a card that has moved on — and would let the new
/// question be inserted a second time beside it.
///
/// # Errors
///
/// Refuses a blank question, a terminal item and a steer that changes nothing.
pub fn redirect(item: &mut AgendaItem, new_question: &str) -> Result<Redirect, RedirectError> {
    let new_question = new_question.trim();
    if new_question.is_empty() {
        return Err(RedirectError::Blank);
    }
    if item.status.is_terminal() {
        return Err(RedirectError::Terminal {
            status: item.status.word(),
        });
    }
    let to_key = dedupe_key(new_question, &item.subject);
    if to_key == item.dedupe_key && new_question == item.question {
        return Err(RedirectError::NoChange {
            question: item.question.clone(),
        });
    }
    let from = std::mem::replace(&mut item.question, new_question.to_string());
    let from_key = std::mem::replace(&mut item.dedupe_key, to_key.clone());
    // The score was computed for the old question. §5.4 recomputes when a
    // source changes a score; this changes the item itself, which is stronger.
    item.score = None;
    Ok(Redirect {
        item: item.id,
        from,
        to: new_question.to_string(),
        from_key,
        to_key,
    })
}

/// Whether a status is one a redirect can reach (§6.4: the human steers a
/// *live* child).
#[must_use]
pub fn is_steerable(status: &ItemStatus) -> bool {
    matches!(
        status,
        ItemStatus::Claimed { .. } | ItemStatus::Running { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::{ExplorationShare, Objective, Scope};
    use crate::couplings::{KillSwitch, ProfileHealth, Signals};

    /// Fixed, never `Uuid::new_v4()`. A previous track in this crate shipped a
    /// test that drew a fresh uuid inside a statistical assertion and failed
    /// about one run in seven.
    fn session() -> Uuid {
        Uuid::from_u128(0x5e55_2002)
    }

    fn item_id(n: u128) -> Uuid {
        Uuid::from_u128(0x1_0000 + n)
    }

    fn trials(n: f64) -> Meter {
        Meter::trials(n).expect("a finite non-negative trial count")
    }

    fn cap(n: f64) -> Observed<Bound> {
        Observed::Is(Bound::Cap(trials(n)))
    }

    fn a_request() -> LeaseRequest {
        LeaseRequest {
            session: session(),
            item: item_id(1),
            unit: SpendUnit::Trials,
            estimate: Observed::Is(trials(4.0)),
            per_task: cap(100.0),
            day: cap(60.0),
            spent_today: Observed::Is(trials(0.0)),
            total: cap(1000.0),
            spent_total: Observed::Is(trials(0.0)),
            in_flight: Vec::new(),
        }
    }

    // ── §6.3, the lease ─────────────────────────────────────────────────────

    #[test]
    fn a_lease_is_the_estimate_and_a_half() {
        let l = lease(&a_request()).expect("nothing binds below the estimate");
        assert!((l.size().amount() - 6.0).abs() < 1e-12, "{:?}", l.size());
        assert_eq!(l.capped_by(), CapKind::Estimate);
        assert_eq!(
            l.max_trials_per_task(),
            TrialCap::Trials {
                max_trials_per_task: 6
            }
        );
    }

    #[test]
    fn the_tightest_cap_is_the_one_the_lease_names() {
        for (which, req) in [
            (CapKind::PerTask, {
                let mut r = a_request();
                r.per_task = cap(2.0);
                r
            }),
            (CapKind::Day, {
                let mut r = a_request();
                r.day = cap(3.0);
                r
            }),
            (CapKind::Total, {
                let mut r = a_request();
                r.total = cap(1.0);
                r
            }),
        ] {
            let l = lease(&req).expect("there is still room for something");
            assert_eq!(l.capped_by(), which, "{:?}", l.size());
        }
    }

    #[test]
    fn a_lease_never_exceeds_the_remaining_day_budget() {
        // §12.4's first clause, swept rather than sampled: whatever the day cap
        // and whatever has been spent, the lease plus the spend never passes
        // the cap.
        for day in [1.0_f64, 5.0, 12.0, 60.0] {
            for spent in [0.0_f64, 0.5, 4.0, 11.0] {
                if spent >= day {
                    continue;
                }
                let mut r = a_request();
                r.day = cap(day);
                r.spent_today = Observed::Is(trials(spent));
                let l = lease(&r).expect("there is headroom left");
                assert!(
                    l.size().amount() + spent <= day + 1e-12,
                    "day {day}, spent {spent}, lease {:?}",
                    l.size()
                );
            }
        }
    }

    #[test]
    fn three_concurrent_children_cannot_together_exceed_the_day_budget() {
        // §12.4's second clause. Each lease is committed in full the moment it
        // is issued, so the third is sized against the first two.
        let mut r = a_request();
        r.day = cap(18.0);
        let mut issued: Vec<Lease> = Vec::new();
        for n in 1..=3u128 {
            r.item = item_id(n);
            r.in_flight = issued.clone();
            let l = lease(&r).expect("three leases of six fit in eighteen trials");
            issued.push(l);
        }
        let total: f64 = issued.iter().map(|l| l.size().amount()).sum();
        assert!(total <= 18.0 + 1e-12, "{total} over a cap of 18");
        assert_eq!(issued.len(), 3);

        // And the fourth is refused rather than squeezed to nothing.
        r.item = item_id(4);
        r.in_flight = issued;
        let e = lease(&r).expect_err("eighteen trials are committed");
        assert!(
            matches!(
                e,
                LeaseRefusal::Exhausted {
                    which: CapKind::Day,
                    ..
                }
            ),
            "{e}"
        );
        assert!(e.to_string().contains("children still running"), "{e}");
    }

    #[test]
    fn an_unread_quantity_refuses_the_lease_rather_than_defaulting_to_zero() {
        // The rule this crate is built on, at the one place where a zero would
        // look like prudence: "nothing spent today" is the *most* permissive
        // reading of a failed read, not the safest.
        for (what, mut r) in [
            ("estimate", {
                let mut r = a_request();
                r.estimate = Observed::unknown("no finished runs of this workflow");
                r
            }),
            ("spent today", {
                let mut r = a_request();
                r.spent_today = Observed::unknown("the usage table did not answer");
                r
            }),
            ("day", {
                let mut r = a_request();
                r.day = Observed::unknown("the session row was not read");
                r
            }),
        ] {
            r.item = item_id(9);
            let e = lease(&r).expect_err("{what} was not read");
            assert!(matches!(e, LeaseRefusal::NotRead { .. }), "{what}: {e:?}");
        }
    }

    #[test]
    fn a_bound_in_another_unit_is_refused_by_name_and_not_converted() {
        let mut r = a_request();
        r.day = Observed::Is(Bound::Cap(Meter::usd(25.0).expect("a day budget")));
        let e = lease(&r).expect_err("dollars do not cap trials");
        assert!(matches!(e, LeaseRefusal::UnitMismatch { .. }), "{e:?}");
        assert!(e.to_string().contains("trials"), "{e}");
        assert!(e.to_string().contains("USD"), "{e}");
        assert!(
            e.to_string().contains("auth session"),
            "the refusal says why there is no conversion: {e}"
        );
    }

    #[test]
    fn a_cap_the_platform_does_not_hold_is_recorded_rather_than_treated_as_infinite() {
        // This is today's platform: `SessionBudget` has `usd_per_day` and no
        // `trials_per_day`, so a lease metered in trials has no day bound.
        let mut r = a_request();
        r.day = Observed::Is(Bound::NotDenominated {
            wants: "research_sessions.trials_per_day",
        });
        let l = lease(&r).expect("the other two caps still apply");
        assert_eq!(l.unbounded_by(), &[CapKind::Day]);
        assert!(
            l.sentence().contains("not bounded by the day budget"),
            "{}",
            l.sentence()
        );
        assert!(
            l.event_payload()["unbounded_by"]
                .as_array()
                .expect("a list")
                .contains(&serde_json::Value::from("day")),
            "the gap rides on `run_started` rather than living in a comment"
        );
    }

    #[test]
    fn a_lease_with_no_real_ceiling_at_all_is_refused() {
        // The scenario the module header names: two bounds are already absent
        // on this platform, and a third absence would leave the child's spend
        // bounded by nothing but this module's own guess.
        let mut r = a_request();
        for (b, wants) in [
            (&mut r.per_task, "the tier's per-task cap"),
            (&mut r.day, "a per-day budget in trials"),
            (&mut r.total, "a total in trials"),
        ] {
            *b = Observed::Is(Bound::NotDenominated { wants });
        }
        let e = lease(&r).expect_err("an estimate is not a ceiling");
        assert!(matches!(e, LeaseRefusal::NothingBounds { .. }), "{e:?}");
        assert!(e.to_string().contains("per-task"), "the way out: {e}");
    }

    #[test]
    fn a_usd_lease_has_no_trial_count_and_says_why() {
        let mut r = a_request();
        r.unit = SpendUnit::Usd;
        r.estimate = Observed::Is(Meter::usd(2.0).expect("an estimate"));
        r.per_task = Observed::Is(Bound::Cap(Meter::usd(10.0).expect("a cap")));
        r.day = Observed::Is(Bound::Cap(Meter::usd(25.0).expect("a cap")));
        r.spent_today = Observed::Is(Meter::usd(0.0).expect("nothing yet"));
        r.total = Observed::Is(Bound::Cap(Meter::usd(750.0).expect("a cap")));
        r.spent_total = Observed::Is(Meter::usd(0.0).expect("nothing yet"));
        let l = lease(&r).expect("a priced platform would get here");
        match l.max_trials_per_task() {
            TrialCap::Unpriceable { because } => {
                assert!(because.contains("auth session"), "{because}");
            }
            other => panic!("a dollar lease cannot name a trial count: {other:?}"),
        }
    }

    #[test]
    fn a_meter_refuses_what_is_not_a_budget() {
        assert!(matches!(
            Meter::trials(f64::NAN),
            Err(MeterError::NotFinite { .. })
        ));
        assert!(matches!(
            Meter::trials(f64::INFINITY),
            Err(MeterError::NotFinite { .. })
        ));
        assert!(matches!(
            Meter::usd(-0.01),
            Err(MeterError::Negative { .. })
        ));
    }

    #[test]
    fn a_lease_that_would_buy_nothing_is_refused() {
        let mut r = a_request();
        r.estimate = Observed::Is(trials(0.0));
        let e = lease(&r).expect_err("a zero lease is not a lease");
        assert!(matches!(e, LeaseRefusal::WouldBeZero { .. }), "{e:?}");
    }

    // ── §2.1's budgets read as bounds ───────────────────────────────────────

    #[test]
    fn a_trials_total_of_zero_is_nobody_said_and_not_a_cap_of_none() {
        // `SessionBudget::default` stores 0 and `desk_charter::translate`
        // stores 0 for a blank form field. Read as a cap it would exhaust the
        // session before its first child ran.
        let b = crate::charter::SessionBudget::default();
        assert_eq!(b.trials_total, 0);
        assert!(matches!(
            total_bound(&b, SpendUnit::Trials).expect("a readable budget"),
            Bound::NotDenominated { .. }
        ));

        let mut filled = b.clone();
        filled.trials_total = 40;
        assert_eq!(
            total_bound(&filled, SpendUnit::Trials).expect("a readable budget"),
            Bound::Cap(trials(40.0))
        );
    }

    #[test]
    fn there_is_no_day_budget_in_trials_and_the_bound_says_which_column_is_missing() {
        let b = crate::charter::SessionBudget::default();
        match day_bound(&b, SpendUnit::Trials).expect("a readable budget") {
            Bound::NotDenominated { wants } => {
                assert!(wants.contains("usd_per_day"), "{wants}");
            }
            other => panic!("there is no trials-per-day figure on this platform: {other:?}"),
        }
        assert_eq!(
            day_bound(&b, SpendUnit::Usd).expect("a readable budget"),
            Bound::Cap(Meter::usd(crate::charter::DEFAULT_USD_PER_DAY).expect("a day budget"))
        );
    }

    #[test]
    fn a_stored_budget_that_is_not_a_number_is_refused_rather_than_leased_against() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let b = crate::charter::SessionBudget {
                usd_per_day: bad,
                ..crate::charter::SessionBudget::default()
            };
            assert!(day_bound(&b, SpendUnit::Usd).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_session_on_todays_platform_leases_with_two_of_three_bounds_named_as_missing() {
        // The end-to-end shape of the honest answer, so the report is a test
        // rather than a claim: a trials-metered lease against a real
        // `SessionBudget` is bounded by the tier's per-task cap alone, and says
        // so on the event.
        let b = crate::charter::SessionBudget::default();
        let r = LeaseRequest {
            session: session(),
            item: item_id(1),
            unit: SpendUnit::Trials,
            estimate: Observed::Is(trials(4.0)),
            per_task: cap(20.0),
            day: Observed::Is(day_bound(&b, SpendUnit::Trials).expect("readable")),
            spent_today: Observed::Is(trials(0.0)),
            total: Observed::Is(total_bound(&b, SpendUnit::Trials).expect("readable")),
            spent_total: Observed::Is(trials(0.0)),
            in_flight: Vec::new(),
        };
        let l = lease(&r).expect("the per-task cap is real");
        assert_eq!(l.unbounded_by(), &[CapKind::Day, CapKind::Total]);
        assert_eq!(l.capped_by(), CapKind::Estimate);
        assert!(
            l.sentence().contains("not bounded by"),
            "the human is told: {}",
            l.sentence()
        );
    }

    // ── §7 row 2, the day budget ────────────────────────────────────────────

    #[test]
    fn the_gate_and_the_lease_read_the_day_budget_the_same_way() {
        // The one thing that would make the gate a lie: a session the gate lets
        // claim and the lease then refuses for want of day budget, every
        // iteration, forever.
        for (cap_at, spent) in [(10.0_f64, 0.0_f64), (10.0, 9.0), (10.0, 10.0), (10.0, 11.0)] {
            let mut r = a_request();
            r.day = cap(cap_at);
            r.spent_today = Observed::Is(trials(spent));
            let gate = day_budget(&r.day, &r.spent_today, &r.in_flight, SpendUnit::Trials);
            let leased = lease(&r);
            assert_eq!(
                gate.may_claim(),
                !matches!(
                    leased,
                    Err(LeaseRefusal::Exhausted {
                        which: CapKind::Day,
                        ..
                    })
                ),
                "cap {cap_at}, spent {spent}: gate {gate:?}, lease {leased:?}"
            );
        }
    }

    #[test]
    fn an_unreadable_day_budget_is_not_a_full_one() {
        let d = day_budget(
            &Observed::unknown("the row was not read"),
            &Observed::Is(trials(0.0)),
            &[],
            SpendUnit::Trials,
        );
        assert!(matches!(d, DayBudget::Unreadable { .. }), "{d:?}");
        assert!(!d.may_claim());
    }

    #[test]
    fn a_day_budget_in_the_wrong_unit_is_unreadable_rather_than_applied() {
        let d = day_budget(
            &Observed::Is(Bound::Cap(Meter::usd(25.0).expect("a day budget"))),
            &Observed::Is(trials(0.0)),
            &[],
            SpendUnit::Trials,
        );
        assert!(matches!(d, DayBudget::Unreadable { .. }), "{d:?}");
    }

    // ── §7 row 3, when the session is finished ──────────────────────────────

    fn a_stopping() -> Stopping {
        Stopping {
            runs_started: 3,
            max_runs: 60,
            unit: SpendUnit::Trials,
            total: cap(1000.0),
            spent_total: Observed::Is(trials(12.0)),
            agenda_drained: false,
            stalled: false,
            deployable_found: Observed::Is(0),
        }
    }

    #[test]
    fn a_session_under_every_cap_is_not_finished() {
        let v = stop(&[], &a_stopping());
        assert!(!v.completes(), "{v:?}");
        assert!(v.not_evaluable.is_empty());
    }

    #[test]
    fn max_runs_and_the_total_budget_stop_it_whatever_the_charter_lists() {
        let mut s = a_stopping();
        s.runs_started = 60;
        let v = stop(&[], &s);
        assert!(v.completes());
        assert_eq!(v.reached[0].word, "max_runs");
        assert_eq!(v.reached[0].condition, None);

        let mut s = a_stopping();
        s.spent_total = Observed::Is(trials(1000.0));
        let v = stop(&[], &s);
        assert!(v.completes());
        assert_eq!(v.reached[0].word, "total_budget");
    }

    #[test]
    fn a_max_runs_of_zero_is_nobody_said_and_not_stop_at_once() {
        // The same trap as `trials_total = 0`, and the consequence is worse:
        // read as a cap it completes the session before its first run.
        let mut s = a_stopping();
        s.max_runs = 0;
        s.runs_started = 0;
        assert!(!stop(&[], &s).completes());
    }

    #[test]
    fn schedule_end_is_reported_as_unevaluable_rather_than_never_firing() {
        // `charter::Schedule` is weekly windows with no end date, so this
        // condition has nothing to compare against. A charter that lists it
        // believes the session will stop, and it will not.
        use crate::charter::StopWhen;
        let v = stop(&[StopWhen::ScheduleEnd], &a_stopping());
        assert!(!v.completes());
        assert_eq!(v.not_evaluable.len(), 1);
        assert_eq!(v.not_evaluable[0].word, "schedule_end");
        assert!(
            v.lines()[0].contains("never stop this session"),
            "{:?}",
            v.lines()
        );
    }

    #[test]
    fn the_charter_conditions_each_read_their_own_fact() {
        use crate::charter::StopWhen;
        let mut s = a_stopping();
        s.agenda_drained = true;
        assert_eq!(
            stop(&[StopWhen::AgendaDrained], &s).reached[0].word,
            "agenda_drained"
        );

        let mut s = a_stopping();
        s.stalled = true;
        assert_eq!(stop(&[StopWhen::Stall], &s).reached[0].word, "stall");

        let mut s = a_stopping();
        s.deployable_found = Observed::Is(2);
        assert!(stop(&[StopWhen::DeployableFound { k: 2 }], &s).completes());
        assert!(!stop(&[StopWhen::DeployableFound { k: 3 }], &s).completes());
    }

    #[test]
    fn an_unread_deployable_count_does_not_complete_and_does_not_pass_quietly() {
        use crate::charter::StopWhen;
        let mut s = a_stopping();
        s.deployable_found = Observed::unknown("the ledger was not queried");
        let v = stop(&[StopWhen::DeployableFound { k: 1 }], &s);
        assert!(!v.completes(), "an unread count is not a count of zero");
        assert_eq!(v.not_evaluable.len(), 1);
        assert!(v.not_evaluable[0].missing.contains("not queried"));
    }

    #[test]
    fn a_charter_naming_budget_does_not_stop_the_session_twice() {
        use crate::charter::StopWhen;
        let mut s = a_stopping();
        s.spent_total = Observed::Is(trials(1000.0));
        let v = stop(&[StopWhen::Budget], &s);
        assert_eq!(v.reached.len(), 1, "one reason, not two: {v:?}");
    }

    #[test]
    fn every_stop_word_is_its_own() {
        use crate::charter::StopWhen;
        let all = [
            StopWhen::Budget,
            StopWhen::ScheduleEnd,
            StopWhen::AgendaDrained,
            StopWhen::Stall,
            StopWhen::DeployableFound { k: 1 },
        ];
        let mut words: Vec<&str> = all.iter().copied().map(stop_word).collect();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), all.len(), "two conditions share a word");
    }

    #[test]
    fn an_undenominated_total_cannot_stop_a_session_and_says_so() {
        use crate::charter::StopWhen;
        let mut s = a_stopping();
        s.total = Observed::Is(
            total_bound(&crate::charter::SessionBudget::default(), SpendUnit::Trials)
                .expect("readable"),
        );
        let v = stop(&[StopWhen::Budget], &s);
        assert!(!v.completes());
        assert_eq!(v.not_evaluable.len(), 1);
        assert!(
            v.not_evaluable[0].missing.contains("trials_total"),
            "{}",
            v.not_evaluable[0].missing
        );
    }

    // ── §7 rows 1–2 and 8–10, the plan ──────────────────────────────────────

    fn a_situation() -> Situation {
        Situation {
            openness: Openness::AlwaysOpen,
            couplings: Verdict::Clear,
            day_budget: DayBudget::Remaining(trials(20.0)),
            suspended: Vec::new(),
            live: Vec::new(),
            max_concurrent: 2,
            claimable: 3,
        }
    }

    fn index_of(steps: &[Step], word: &str) -> Option<usize> {
        steps.iter().position(|s| s.as_str() == word)
    }

    #[test]
    fn the_next_window_resumes_before_it_claims_anything_new() {
        // §7 row 1's second clause and §12.5's second clause, asserted as an
        // ordering rather than as a pair of booleans a caller could read in
        // either order.
        let mut s = a_situation();
        s.max_concurrent = 3;
        s.suspended = vec![item_id(1)];
        let steps = plan(&s);
        let resume = index_of(&steps, "resume").expect("a suspended child is resumed");
        let claim = index_of(&steps, "claim").expect("and there is room for one more");
        assert!(resume < claim, "{steps:?}");
        assert_eq!(steps[claim], Step::Claim { slots: 2 });
    }

    #[test]
    fn a_resumed_child_occupies_a_slot() {
        let mut s = a_situation();
        s.max_concurrent = 2;
        s.suspended = vec![item_id(1), item_id(2)];
        let steps = plan(&s);
        assert_eq!(index_of(&steps, "claim"), None, "{steps:?}");
        assert!(
            matches!(
                steps.last(),
                Some(Step::Hold {
                    why: HoldReason::NoSlots,
                    ..
                })
            ),
            "{steps:?}"
        );
    }

    #[test]
    fn a_closed_window_drains_the_live_children_and_re_arms_on_the_opening() {
        let opens = DateTime::parse_from_rfc3339("2026-09-23T13:00:00Z")
            .expect("a fixed instant")
            .with_timezone(&Utc);
        let mut s = a_situation();
        s.openness = Openness::Closed { opens_at: opens };
        s.live = vec![item_id(7)];
        let steps = plan(&s);
        assert_eq!(
            steps[0],
            Step::Drain {
                at: DrainAt::Now,
                children: vec![item_id(7)]
            }
        );
        assert_eq!(
            steps[1],
            Step::Hold {
                why: HoldReason::WindowClosed,
                re_arms_at: Some(opens)
            }
        );
        assert_eq!(index_of(&steps, "claim"), None);
    }

    #[test]
    fn an_open_window_hands_the_live_children_their_boundary() {
        let closes = DateTime::parse_from_rfc3339("2026-09-22T17:00:00Z")
            .expect("a fixed instant")
            .with_timezone(&Utc);
        let mut s = a_situation();
        s.openness = Openness::Open { closes_at: closes };
        s.live = vec![item_id(7)];
        let steps = plan(&s);
        assert_eq!(
            steps[0],
            Step::Drain {
                at: DrainAt::At(closes),
                children: vec![item_id(7)]
            }
        );
        assert!(index_of(&steps, "claim").is_some(), "{steps:?}");
    }

    #[test]
    fn an_indeterminate_calendar_holds_rather_than_claiming() {
        // The calendar's own word for "I could not find the boundary". A driver
        // that read it as open would run outside its fence at the one moment
        // the calendar was confused.
        let mut s = a_situation();
        s.openness = Openness::Indeterminate;
        let steps = plan(&s);
        assert_eq!(index_of(&steps, "claim"), None, "{steps:?}");
    }

    #[test]
    fn the_kill_switch_drains_and_a_degraded_tier_lets_children_finish() {
        // §7 rows 8 and 9 differ in exactly this, and the difference is a
        // method on `PauseReason` rather than a branch written here.
        let mut s = a_situation();
        s.live = vec![item_id(7)];
        s.couplings = Verdict::Paused(vec![PauseReason::KillSwitch]);
        assert_eq!(plan(&s)[0].as_str(), "drain");

        s.couplings = Verdict::Paused(vec![PauseReason::Tier {
            profile: "opus".into(),
            health: "degraded".into(),
        }]);
        let steps = plan(&s);
        assert_eq!(index_of(&steps, "drain"), None, "{steps:?}");
        assert_eq!(steps[0].as_str(), "hold");
    }

    #[test]
    fn a_coupling_that_could_not_be_read_holds_without_draining_or_appending() {
        let mut s = a_situation();
        s.live = vec![item_id(7)];
        s.couplings = crate::couplings::evaluate(&Signals {
            kill_switch: Observed::unknown("monitor.events did not answer"),
            profile: "opus".into(),
            health: Observed::Is(ProfileHealth::Measured),
            children: Vec::new(),
        });
        let steps = plan(&s);
        assert_eq!(index_of(&steps, "drain"), None, "{steps:?}");
        match &steps[0] {
            Step::Hold { why, .. } => {
                assert!(matches!(why, HoldReason::CannotDecide { .. }), "{why:?}");
                assert!(
                    !why.is_a_transition(),
                    "a failed read is not a pause a human has to clear"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_exhausted_day_budget_stops_claims_and_is_a_transition() {
        let mut s = a_situation();
        s.live = vec![item_id(7)];
        s.day_budget = DayBudget::Exhausted {
            cap: trials(10.0),
            spent: trials(10.0),
        };
        let steps = plan(&s);
        assert_eq!(steps[0].as_str(), "drain");
        match &steps[1] {
            Step::Hold { why, .. } => {
                assert_eq!(why.as_str(), "day_budget_exhausted");
                assert!(why.is_a_transition());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_undenominated_day_budget_still_claims_and_the_lease_carries_the_gap() {
        // The two halves of today's honest answer, pinned together: the gate
        // permits, and the lease records. Either alone would be wrong.
        let mut s = a_situation();
        s.day_budget = DayBudget::NotDenominated {
            wants: "research_sessions.trials_per_day",
        };
        assert!(index_of(&plan(&s), "claim").is_some());

        let mut r = a_request();
        r.day = Observed::Is(Bound::NotDenominated {
            wants: "research_sessions.trials_per_day",
        });
        assert_eq!(
            lease(&r).expect("still leasable").unbounded_by(),
            &[CapKind::Day]
        );
    }

    #[test]
    fn nothing_claimable_and_no_slots_are_different_words() {
        let mut s = a_situation();
        s.claimable = 0;
        assert!(matches!(
            plan(&s).last(),
            Some(Step::Hold {
                why: HoldReason::NothingClaimable,
                ..
            })
        ));
        s.claimable = 3;
        s.live = vec![item_id(1), item_id(2)];
        assert!(matches!(
            plan(&s).last(),
            Some(Step::Hold {
                why: HoldReason::NoSlots,
                ..
            })
        ));
    }

    #[test]
    fn every_plan_ends_in_exactly_one_claim_or_one_hold() {
        // A plan with neither is a driver that does nothing and says nothing;
        // a plan with both is two readers disagreeing about whether it claimed.
        let openness = [
            Openness::AlwaysOpen,
            Openness::Open {
                closes_at: Utc::now(),
            },
            Openness::Closed {
                opens_at: Utc::now(),
            },
            Openness::Indeterminate,
        ];
        let couplings = [
            Verdict::Clear,
            Verdict::Paused(vec![PauseReason::KillSwitch]),
            Verdict::CannotDecide {
                missing: vec!["the kill switch".into()],
            },
        ];
        let budgets = [
            DayBudget::Remaining(trials(5.0)),
            DayBudget::Exhausted {
                cap: trials(1.0),
                spent: trials(1.0),
            },
            DayBudget::NotDenominated { wants: "a column" },
            DayBudget::Unreadable {
                why: "no read".into(),
            },
        ];
        let mut checked = 0;
        for o in &openness {
            for c in &couplings {
                for b in &budgets {
                    for live in [0usize, 1] {
                        for suspended in [0usize, 1] {
                            for claimable in [0usize, 2] {
                                let s = Situation {
                                    openness: *o,
                                    couplings: c.clone(),
                                    day_budget: b.clone(),
                                    suspended: (0..suspended)
                                        .map(|n| item_id(100 + n as u128))
                                        .collect(),
                                    live: (0..live).map(|n| item_id(200 + n as u128)).collect(),
                                    max_concurrent: 2,
                                    claimable,
                                };
                                let steps = plan(&s);
                                let terminal = steps
                                    .iter()
                                    .filter(|x| matches!(x, Step::Claim { .. } | Step::Hold { .. }))
                                    .count();
                                assert_eq!(terminal, 1, "{s:?} -> {steps:?}");
                                assert!(
                                    matches!(
                                        steps.last(),
                                        Some(Step::Claim { .. } | Step::Hold { .. })
                                    ),
                                    "and it is last: {steps:?}"
                                );
                                if let (Some(r), Some(cl)) =
                                    (index_of(&steps, "resume"), index_of(&steps, "claim"))
                                {
                                    assert!(r < cl, "resume before claim: {steps:?}");
                                }
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(checked >= 384, "the sweep swept {checked} situations");
    }

    // ── §6.2, the brief ─────────────────────────────────────────────────────

    fn a_charter(subjects: &[&str], constraints: &[&str]) -> Charter {
        Charter {
            scope: Scope {
                instruments: subjects.iter().map(|s| (*s).to_string()).collect(),
                ..Scope::default()
            },
            objective: Objective::BuildKnowledge {
                about: subjects.iter().map(|s| (*s).to_string()).collect(),
            },
            seeds: vec![],
            permitted_workflows: vec![],
            exclusions: vec![],
            standing_constraints: constraints.iter().map(|c| (*c).to_string()).collect(),
            exploration_share: ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        }
    }

    fn sibling(n: u128, subject: &str, verdict: &str) -> SiblingRun {
        SiblingRun {
            item: item_id(500 + n),
            run: Uuid::from_u128(0x9000 + n),
            subject: subject.to_string(),
            question: format!("question {n}"),
            verdict: verdict.to_string(),
        }
    }

    #[test]
    fn the_brief_carries_the_last_five_verdicts_in_scope_newest_first() {
        let charter = a_charter(&["btc"], &["never trade on weekends"]);
        let item = AgendaItem::new(session(), "does x work?", "btc", Source::CharterSeed);
        let siblings: Vec<SiblingRun> = (1..=8).map(|n| sibling(n, "btc", "completed")).collect();
        let b = brief(&BriefInput {
            session: session(),
            charter: &charter,
            item: &item,
            siblings: &siblings,
        });
        assert_eq!(b.recent.len(), RECENT_VERDICTS);
        assert!(
            b.recent[0].contains("question 8"),
            "newest first: {:?}",
            b.recent
        );
        assert!(b.recent[4].contains("question 4"), "{:?}", b.recent);
        assert_eq!(
            b.preferences,
            vec![Preference {
                scope: format!("session:{}", session()),
                text: "never trade on weekends".into()
            }]
        );
    }

    #[test]
    fn a_sibling_out_of_scope_is_not_in_the_brief() {
        let charter = a_charter(&["btc"], &[]);
        let item = AgendaItem::new(session(), "does x work?", "btc", Source::CharterSeed);
        let siblings = vec![sibling(1, "eth", "completed"), sibling(2, "btc", "failed")];
        let b = brief(&BriefInput {
            session: session(),
            charter: &charter,
            item: &item,
            siblings: &siblings,
        });
        assert_eq!(b.recent.len(), 1, "{:?}", b.recent);
        assert!(b.recent[0].contains("question 2"));
    }

    #[test]
    fn the_item_does_not_tell_itself_what_it_found() {
        let charter = a_charter(&["btc"], &[]);
        let item = AgendaItem::new(session(), "does x work?", "btc", Source::CharterSeed);
        let mut mine = sibling(1, "btc", "completed");
        mine.item = item.id;
        let b = brief(&BriefInput {
            session: session(),
            charter: &charter,
            item: &item,
            siblings: &[mine],
        });
        assert!(b.recent.is_empty(), "{:?}", b.recent);
    }

    #[test]
    fn why_now_says_which_source_and_cites_it_and_never_invents_an_age() {
        let mut item = AgendaItem::new(
            session(),
            "is f_31a still true?",
            "btc",
            Source::StaleFinding,
        )
        .with_source_ref("f_31a");
        let s = why_now(&item);
        assert!(s.contains("valid_until"), "{s}");
        assert!(s.contains("f_31a"), "{s}");
        assert!(
            !s.chars()
                .any(|c| c.is_ascii_digit() && c != '3' && c != '1'),
            "the spec's example says `94 days stale` and no source carries an age: {s}"
        );

        item.sources.push(Source::OpenQuestionBlocking);
        let s = why_now(&item);
        assert!(s.contains("1 other source"), "{s}");
    }

    #[test]
    fn a_source_with_no_reference_says_so_rather_than_citing_nothing() {
        let item = AgendaItem::new(session(), "q", "btc", Source::Contradiction);
        let s = why_now(&item);
        assert!(s.contains("no reference was recorded"), "{s}");
    }

    #[test]
    fn every_source_has_a_sentence_of_its_own() {
        // A `_ =>` arm here would give three sources the same sentence and the
        // child would be told the wrong reason it is running.
        let mut seen: Vec<String> = Vec::new();
        for src in Source::ALL {
            let item = AgendaItem::new(session(), "q", "btc", *src).with_source_ref("r_1");
            let s = why_now(&item);
            assert!(
                !seen.contains(&s),
                "{src:?} reuses another source's sentence"
            );
            seen.push(s);
        }
        assert_eq!(seen.len(), Source::ALL.len());
    }

    // ── §6.4, the redirecting steer ─────────────────────────────────────────

    #[test]
    fn a_redirect_moves_the_dedupe_key_with_the_question() {
        let mut item = AgendaItem::new(session(), "does x work?", "btc", Source::User);
        item.status = ItemStatus::Running {
            run_id: Uuid::from_u128(0xabc),
        };
        item.score = Some(crate::agenda::Score::combine(1.0, 1.0, 1.0, 1.0));
        let before = item.dedupe_key.clone();
        let r = redirect(&mut item, "does y work?").expect("a live item redirects");
        assert_eq!(item.question, "does y work?");
        assert_ne!(item.dedupe_key, before);
        assert_eq!(item.dedupe_key, r.to_key);
        assert_eq!(r.from_key, before);
        assert!(
            item.score.is_none(),
            "the score was the old question's and is not this one's"
        );
    }

    #[test]
    fn the_new_key_is_the_one_layer_one_would_compute() {
        let mut item = AgendaItem::new(session(), "does x work?", "btc", Source::User);
        item.status = ItemStatus::Running {
            run_id: Uuid::from_u128(0xabc),
        };
        redirect(&mut item, "  Does Y Work?  ").expect("a live item redirects");
        let fresh = AgendaItem::new(session(), "Does Y Work?", "btc", Source::User);
        assert_eq!(
            item.dedupe_key, fresh.dedupe_key,
            "a redirected item and a fresh offer of the same sentence must collide"
        );
    }

    #[test]
    fn a_finished_item_is_not_rewritten() {
        let mut item = AgendaItem::new(session(), "does x work?", "btc", Source::User);
        item.status = ItemStatus::Done {
            verdict: "completed".into(),
        };
        let e = redirect(&mut item, "does y work?").expect_err("the record stands");
        assert!(matches!(e, RedirectError::Terminal { .. }), "{e}");
        assert_eq!(item.question, "does x work?");
    }

    #[test]
    fn a_blank_redirect_and_a_no_op_redirect_are_both_refused() {
        let mut item = AgendaItem::new(session(), "does x work?", "btc", Source::User);
        item.status = ItemStatus::Running {
            run_id: Uuid::from_u128(0xabc),
        };
        assert!(matches!(
            redirect(&mut item, "   "),
            Err(RedirectError::Blank)
        ));
        assert!(matches!(
            redirect(&mut item, "does x work?"),
            Err(RedirectError::NoChange { .. })
        ));
        assert_eq!(item.question, "does x work?");
    }

    #[test]
    fn only_a_live_child_is_steerable() {
        assert!(is_steerable(&ItemStatus::Running {
            run_id: Uuid::nil()
        }));
        assert!(is_steerable(&ItemStatus::Claimed {
            run_id: Uuid::nil()
        }));
        assert!(!is_steerable(&ItemStatus::Queued));
        assert!(!is_steerable(&ItemStatus::Done {
            verdict: "completed".into()
        }));
    }

    #[test]
    fn the_kill_switch_still_refuses_a_ring_two_proposal_while_a_session_is_held() {
        // Not this module's rule, and it is asserted here because `plan` is
        // where a caller decides what to do next and could reasonably think a
        // held session is a quiet one. §7 row 8 says no proposal into ring 2
        // may be *created* while the switch is tripped, which is a separate
        // refusal that does not go through the plan at all.
        let signals = Signals {
            kill_switch: Observed::Is(KillSwitch::Tripped),
            profile: "opus".into(),
            health: Observed::Is(ProfileHealth::Measured),
            children: Vec::new(),
        };
        assert!(crate::couplings::refuses_ring(&signals, crate::harvest::Ring::Paper).is_some());
    }
}
