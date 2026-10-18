//! Persisting `harness::memory::Entry` (AGENT-007 §2.2; Set S task S-4.3).
//!
//! The harness computes and, until `0122_semantic_memory.sql`, nothing stored
//! the result. This is the join: one place that turns an `Entry` into a row and
//! a row back into an `Entry`.
//!
//! # It validates before it writes, and the database validates again
//!
//! [`MemoryStore::put`] calls [`Entry::validate`] first. Not because the CHECKs
//! are insufficient — they are the enforcement, and they are what makes the
//! acceptance true — but because a constraint violation surfaces as a
//! `sqlx::Error` naming a constraint, and `EntryError::ClaimTooLong(412)` is the
//! message the caller can act on. The database is the guard; this is the
//! error message.
//!
//! The order matters the other way round too: if the two ever disagree, the
//! write fails at the database and the caller sees a constraint name rather than
//! silent acceptance. That is the correct direction for them to disagree in, and
//! `crates/invariants/tests/schema_mirrors_memory_types.rs` exists to stop them
//! disagreeing at all.
//!
//! # Why `extra` is JSONB and the shared fields are columns
//!
//! §2.2's table is a shared left-hand column and a per-kind right-hand one. The
//! shared half is what anything queries on — subject, kind, confidence, window,
//! staleness — so it is columns with indexes. The per-kind half shares nothing
//! between an `instrument_profile`'s coverage map and an `open_question`'s
//! blocking flag, so seven sparse column sets would be six nulls per row and a
//! migration every time a kind grows a field.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use harness::memory::conflict::{corroborate, decide_write, link_conflict, WriteAction};
use harness::memory::{
    Confidence, DataWindow, Direction, Entry, EntryError, EvidenceClass, Kind, Measurement,
    Provenance,
};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("{0}")]
    Invalid(#[from] EntryError),
    #[error("the store rejected the entry: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A row whose text columns hold something no enum accepts. Only reachable
    /// if a writer bypassed both the CHECK and this module.
    #[error("row {id} carries {field} = {value:?}, which is not a value this type has")]
    Unreadable {
        id: String,
        field: &'static str,
        value: String,
    },
}

/// The `extra.directive` of a preference that skips the Research offer (S-10.2).
pub const ALWAYS_START_DIRECTIVE: &str = "always_start_research";

/// What [`MemoryStore::put_checked`] did with a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// A row was written (or replaced), linked to these contradicting ids.
    Inserted { conflicts_with: Vec<String> },
    /// No row was written: this stored entry's corroboration count rose.
    Corroborated { into: String },
}

/// What one [`MemoryStore::sweep_retention`] pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct SweepReport {
    pub examined: u64,
    pub archived_now: u64,
    pub staled_now: u64,
}

pub struct MemoryStore {
    pool: PgPool,
}

/// The window's ends as dates.
///
/// `DataWindow` carries ISO strings because the harness has no chrono dependency
/// in that module; the column is a `DATE`. Parsing here rather than storing text
/// is deliberate — the near-duplicate check (§2.2.1) asks about *overlapping*
/// windows, and overlap over text is a string comparison that happens to work
/// for ISO-8601 and silently stops working for anything else.
fn window_dates(w: &DataWindow) -> Result<(NaiveDate, NaiveDate), EntryError> {
    let start =
        NaiveDate::parse_from_str(&w.start, "%Y-%m-%d").map_err(|_| EntryError::NoWindow)?;
    let end = NaiveDate::parse_from_str(&w.end, "%Y-%m-%d").map_err(|_| EntryError::NoWindow)?;
    Ok((start, end))
}

fn direction_str(d: Direction) -> &'static str {
    match d {
        Direction::Positive => "positive",
        Direction::Negative => "negative",
        Direction::Null => "null",
    }
}

fn kind_from(s: &str, id: &str) -> Result<Kind, MemoryError> {
    Ok(match s {
        "finding" => Kind::Finding,
        "negative_result" => Kind::NegativeResult,
        "instrument_profile" => Kind::InstrumentProfile,
        "signal_registry" => Kind::SignalRegistry,
        "platform_defect" => Kind::PlatformDefect,
        "user_preference" => Kind::UserPreference,
        "open_question" => Kind::OpenQuestion,
        other => {
            return Err(MemoryError::Unreadable {
                id: id.into(),
                field: "kind",
                value: other.into(),
            })
        }
    })
}

fn confidence_from(s: &str, id: &str) -> Result<Confidence, MemoryError> {
    Ok(match s {
        "established" => Confidence::Established,
        "suggestive" => Confidence::Suggestive,
        "refuted" => Confidence::Refuted,
        other => {
            return Err(MemoryError::Unreadable {
                id: id.into(),
                field: "confidence",
                value: other.into(),
            })
        }
    })
}

fn evidence_class_from(s: &str, id: &str) -> Result<EvidenceClass, MemoryError> {
    Ok(match s {
        "direct_stat" => EvidenceClass::DirectStat,
        "derived" => EvidenceClass::Derived,
        "model_summary" => EvidenceClass::ModelSummary,
        "human" => EvidenceClass::Human,
        other => {
            return Err(MemoryError::Unreadable {
                id: id.into(),
                field: "evidence_class",
                value: other.into(),
            })
        }
    })
}

fn direction_from(s: &str, id: &str) -> Result<Direction, MemoryError> {
    Ok(match s {
        "positive" => Direction::Positive,
        "negative" => Direction::Negative,
        "null" => Direction::Null,
        other => {
            return Err(MemoryError::Unreadable {
                id: id.into(),
                field: "direction",
                value: other.into(),
            })
        }
    })
}

impl MemoryStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert or replace one entry.
    ///
    /// # Errors
    /// [`MemoryError::Invalid`] when `Entry::validate` refuses it — the same
    /// refusal the database would give, with a message naming the field;
    /// [`MemoryError::Sqlx`] for anything the database refuses that this did not.
    pub async fn put(&self, user_id: Uuid, entry: &Entry) -> Result<(), MemoryError> {
        self.put_checked(user_id, entry).await.map(|_| ())
    }

    /// [`Self::put`], and says what the write did (S-4.15, S-4.16).
    ///
    /// This is the one door a finding goes through, so it is where the
    /// memory-poisoning guard lives. For a measured `finding` it asks
    /// [`harness::memory::conflict::decide_write`] what the existing live
    /// findings on the subject say:
    ///
    /// * a **contradiction** (opposing direction, disjoint intervals, overlapping
    ///   horizon, both `established`) inserts the row linked both ways in
    ///   `conflicts_with`, in one transaction, and downgrades neither;
    /// * a **near-duplicate** inserts nothing: the stored row's corroboration
    ///   count rises and it gains the writing run's id;
    /// * anything else inserts.
    ///
    /// Everything else (non-findings, findings with no measurement, a re-write
    /// of an id that already exists) is the plain upsert it always was.
    ///
    /// # Errors
    /// As [`Self::put`].
    pub async fn put_checked(
        &self,
        user_id: Uuid,
        entry: &Entry,
    ) -> Result<WriteOutcome, MemoryError> {
        entry.validate()?;
        if entry.kind != Kind::Finding || entry.measurement.is_none() {
            Self::write(&self.pool, user_id, entry).await?;
            return Ok(WriteOutcome::Inserted {
                conflicts_with: entry.conflicts_with.clone(),
            });
        }
        // Including tainted entries: a contradiction with a finding derived from a
        // fetched page is still a contradiction, and the guard must not be blind to it.
        let existing = self
            .for_subject_including_tainted(user_id, &entry.subject, 200)
            .await?;
        match decide_write(entry, &existing) {
            WriteAction::Corroborate { entry_id } => {
                let Some(mut stored) = existing.into_iter().find(|e| e.id == entry_id) else {
                    // decide_write only names ids it was given.
                    Self::write(&self.pool, user_id, entry).await?;
                    return Ok(WriteOutcome::Inserted {
                        conflicts_with: entry.conflicts_with.clone(),
                    });
                };
                corroborate(&mut stored, entry);
                Self::write(&self.pool, user_id, &stored).await?;
                Ok(WriteOutcome::Corroborated { into: entry_id })
            }
            WriteAction::Insert { conflicts_with } => {
                if conflicts_with.is_empty() {
                    Self::write(&self.pool, user_id, entry).await?;
                    return Ok(WriteOutcome::Inserted {
                        conflicts_with: entry.conflicts_with.clone(),
                    });
                }
                let mut new = entry.clone();
                let mut tx = self.pool.begin().await?;
                let mut linked = Vec::new();
                for mut stored in existing
                    .into_iter()
                    .filter(|e| conflicts_with.contains(&e.id))
                {
                    link_conflict(&mut new, &mut stored);
                    linked.push(stored);
                }
                Self::write(&mut *tx, user_id, &new).await?;
                for stored in &linked {
                    Self::write(&mut *tx, user_id, stored).await?;
                }
                tx.commit().await?;
                Ok(WriteOutcome::Inserted {
                    conflicts_with: new.conflicts_with,
                })
            }
        }
    }

    /// The one INSERT, over any executor.
    ///
    /// Generic so [`Self::supersede`] can run it inside a transaction without a
    /// second copy of twenty-nine bindings — and a second copy is exactly how the
    /// two paths would drift, with only one of them ever exercised by a test.
    async fn write<'e, E>(executor: E, user_id: Uuid, entry: &Entry) -> Result<(), MemoryError>
    where
        E: sqlx::PgExecutor<'e>,
    {
        entry.validate()?;
        let (start, end) = window_dates(&entry.data_window)?;
        let valid_until = entry
            .valid_until
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        let evidence_ids = serde_json::to_value(&entry.evidence_ids).unwrap_or(Value::Null);
        // `Entry::extra` is `#[serde(default)]`, so an entry that carries no
        // kind-specific fields arrives as JSON `null` — and the column requires
        // an object. Every `Entry::finding(...)` therefore failed to insert
        // until this line, which is a defect a unit test could not see: the
        // round-trip fixture set `extra` explicitly and passed.
        //
        // Normalised here rather than relaxed in the schema. "No kind-specific
        // fields" is `{}`; storing `null` as a second way to say the same thing
        // would be a distinction every reader has to handle and none can act on.
        let extra = if entry.extra.is_object() {
            entry.extra.clone()
        } else {
            serde_json::json!({})
        };
        // Attribution. §2.2 makes `created_by` a shared field of every kind —
        // `{agent_model | user_id}` — and the schema requires it. The harness
        // never sets it: `Entry::finding` leaves it empty and nothing in
        // `crates/harness` fills it, so a harness-written entry could not be
        // stored at all.
        //
        // The store supplies it, because the store is the only place that knows
        // the user. Filling it here rather than relaxing the CHECK: an entry
        // nobody is attributed to is the one §2.2 forbids, and the fix is to
        // attribute it, not to stop asking. A caller that sets it — the
        // preference route stamps the user explicitly — keeps its own value.
        let created_by = if entry.created_by.trim().is_empty() {
            user_id.to_string()
        } else {
            entry.created_by.clone()
        };
        let m = entry.measurement.as_ref();

        sqlx::query(
            "INSERT INTO agent_memory.entry (
               entry_id, user_id, kind, subject, claim, evidence_ids,
               quantity, direction, value, se, n,
               confidence, evidence_class, window_start, window_end, valid_until,
               prov_run_id, prov_tool, prov_tool_version, prov_template_hash,
               created_by, superseded_by, conflicts_with, stale, archived,
               corroborations, corroborated_by, cited_by_chain, extra
             ) VALUES (
               $1,$2,$3,$4,$5,$6,
               $7,$8,$9,$10,$11,
               $12,$13,$14,$15,$16,
               $17,$18,$19,$20,
               $21,$22,$23,$24,$25,
               $26,$27,$28,$29
             )
             ON CONFLICT (entry_id) DO UPDATE SET
               claim = EXCLUDED.claim,
               evidence_ids = EXCLUDED.evidence_ids,
               confidence = EXCLUDED.confidence,
               evidence_class = EXCLUDED.evidence_class,
               valid_until = EXCLUDED.valid_until,
               superseded_by = EXCLUDED.superseded_by,
               conflicts_with = EXCLUDED.conflicts_with,
               stale = EXCLUDED.stale,
               archived = EXCLUDED.archived,
               corroborations = EXCLUDED.corroborations,
               corroborated_by = EXCLUDED.corroborated_by,
               cited_by_chain = EXCLUDED.cited_by_chain,
               extra = EXCLUDED.extra,
               updated_at = now()",
        )
        .bind(&entry.id)
        .bind(user_id)
        .bind(entry.kind.as_str())
        .bind(&entry.subject)
        .bind(&entry.claim)
        .bind(evidence_ids)
        .bind(m.map(|m| m.quantity.clone()))
        .bind(m.map(|m| direction_str(m.direction)))
        .bind(m.map(|m| m.value))
        .bind(m.and_then(|m| m.se))
        .bind(m.and_then(|m| m.n).and_then(|n| i64::try_from(n).ok()))
        .bind(entry.confidence.as_str())
        .bind(evidence_class_str(entry.evidence_class))
        .bind(start)
        .bind(end)
        .bind(valid_until)
        .bind(&entry.provenance.run_id)
        .bind(&entry.provenance.tool)
        .bind(&entry.provenance.tool_version)
        .bind(&entry.provenance.template_hash)
        .bind(&created_by)
        .bind(entry.superseded_by.as_deref())
        .bind(&entry.conflicts_with)
        .bind(entry.stale)
        .bind(entry.archived)
        .bind(i32::try_from(entry.corroborations).unwrap_or(i32::MAX))
        .bind(&entry.corroborated_by)
        .bind(entry.cited_by_chain)
        .bind(extra)
        .execute(executor)
        .await?;
        Ok(())
    }

    /// Replace an entry with a newer one, **keeping the old row** (§3.3, S-4.7).
    ///
    /// The distinction the acceptance turns on: a superseded profile is
    /// *retained*, not overwritten. `put` would overwrite — it is
    /// `ON CONFLICT DO UPDATE` — which is right for a correction and wrong for a
    /// succession. A weekly profiler that overwrote would leave no way to ask
    /// what the instrument looked like last month, and "the profile changed" is
    /// often the finding.
    ///
    /// So the old row stays exactly as it was and gains a forward pointer. The
    /// reads above exclude anything superseded, so the *current* view shows one
    /// profile per instrument while the history stays queryable — retained and
    /// shown are different things, and collapsing them would mean the brief
    /// listing every profile the instrument has ever had.
    ///
    /// Both writes go in one transaction. Half of a succession is worse than
    /// neither: a new profile with the old one still current shows two, and an
    /// old one marked superseded by a row that failed to write shows none.
    ///
    /// # Errors
    /// [`MemoryError::Invalid`] if the replacement does not validate;
    /// [`MemoryError::Sqlx`] if either write fails — in which case neither
    /// lands.
    pub async fn supersede(
        &self,
        user_id: Uuid,
        old_id: &str,
        replacement: &Entry,
    ) -> Result<(), MemoryError> {
        replacement.validate()?;
        let mut tx = self.pool.begin().await?;

        // The new row first: if it fails, nothing has been marked superseded.
        Self::write(&mut *tx, user_id, replacement).await?;

        sqlx::query(
            "UPDATE agent_memory.entry SET superseded_by = $1, updated_at = now()               WHERE entry_id = $2 AND user_id = $3",
        )
        .bind(&replacement.id)
        .bind(old_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Apply the retention rule to every entry of every user (S-4.16, §2.4).
    ///
    /// `today` is an ISO date, supplied rather than read: the rule is a pure
    /// function of it, and a sweep whose answer depended on when the test ran
    /// could not be asserted. `last_cited` is the latest `cited_at` of any
    /// retrieval of the entry, or its creation day when it has never been cited.
    ///
    /// Archiving sets a flag and nothing else. No row is deleted, so a dossier
    /// citing an archived entry still resolves it through [`Self::get`].
    ///
    /// # Errors
    /// [`MemoryError::Sqlx`] if the read or an update fails;
    /// [`MemoryError::Unreadable`] for a row no enum accepts.
    pub async fn sweep_retention(&self, today: &str) -> Result<SweepReport, MemoryError> {
        let rows = sqlx::query(
            "SELECT e.*, e.user_id AS owner, e.created_at::date::text AS created_day,                     (SELECT max(r.cited_at)::date::text FROM memory_retrievals r                       WHERE r.entry_id = e.entry_id AND r.cited) AS last_cited_day                FROM agent_memory.entry e",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut report = SweepReport::default();
        for row in &rows {
            let owner: Uuid = row.get("owner");
            let created: String = row.get("created_day");
            let last_cited: Option<String> = row.try_get("last_cited_day").ok().flatten();
            let mut entry = row_to_entry(row)?;
            report.examined += 1;
            let before = (entry.stale, entry.archived);
            let decision = harness::memory::retention::retention(
                &entry,
                today,
                last_cited.as_deref().unwrap_or(&created),
            );
            harness::memory::retention::apply(&mut entry, decision);
            if before == (entry.stale, entry.archived) {
                continue;
            }
            sqlx::query(
                "UPDATE agent_memory.entry SET stale = $1, archived = $2, updated_at = now()                   WHERE user_id = $3 AND entry_id = $4",
            )
            .bind(entry.stale)
            .bind(entry.archived)
            .bind(owner)
            .bind(&entry.id)
            .execute(&self.pool)
            .await?;
            if entry.archived && !before.1 {
                report.archived_now += 1;
            } else if entry.stale && !before.0 {
                report.staled_now += 1;
            }
        }
        Ok(report)
    }

    /// The monthly store report for one user (§2.4's last paragraph, S-4.16):
    /// store size, archive rate, and the share of retrievals that were cited.
    ///
    /// Counts archived rows too, because the archive rate is a share of the
    /// whole store.
    ///
    /// # Errors
    /// As [`Self::sweep_retention`].
    pub async fn store_report(
        &self,
        user_id: Uuid,
    ) -> Result<harness::memory::retention::StoreReport, MemoryError> {
        let rows = sqlx::query("SELECT * FROM agent_memory.entry WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        let entries = rows
            .iter()
            .map(row_to_entry)
            .collect::<Result<Vec<_>, _>>()?;
        let counts = sqlx::query(
            "SELECT count(*)::bigint AS shown, count(*) FILTER (WHERE r.cited)::bigint AS cited                FROM memory_retrievals r JOIN agent_memory.entry e ON e.entry_id = r.entry_id               WHERE e.user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        let shown: i64 = counts.try_get("shown").unwrap_or(0);
        let cited: i64 = counts.try_get("cited").unwrap_or(0);
        Ok(harness::memory::retention::store_report(
            &entries,
            usize::try_from(shown).unwrap_or(0),
            usize::try_from(cited).unwrap_or(0),
        ))
    }

    /// Write the monthly store report for every user who has entries and no report
    /// for `today`'s month yet. Returns how many were written.
    ///
    /// Size, archive rate and cited share, as [`Self::store_report`] computes them,
    /// stored once per `(user, month)` and never edited: the month-over-month series
    /// is what shows a store degrading, and a row that could be rewritten would let a
    /// bad month be tidied away. `ON CONFLICT DO NOTHING` makes the daily call that
    /// finds the month already reported a no-op.
    ///
    /// # Errors
    /// As [`Self::store_report`], or the insert failing.
    pub async fn publish_monthly_reports(&self, today: &str) -> Result<usize, MemoryError> {
        use chrono::Datelike;
        let day = NaiveDate::parse_from_str(today, "%Y-%m-%d").map_err(|_| {
            MemoryError::Unreadable {
                id: "monthly_report".into(),
                field: "today",
                value: today.to_string(),
            }
        })?;
        let month = day.with_day(1).unwrap_or(day);
        let users: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT e.user_id FROM agent_memory.entry e
              WHERE NOT EXISTS (SELECT 1 FROM agent_memory.monthly_report r
                                 WHERE r.user_id = e.user_id AND r.month = $1)",
        )
        .bind(month)
        .fetch_all(&self.pool)
        .await?;
        let mut written = 0;
        for user in users {
            let report = self.store_report(user).await?;
            let body = serde_json::to_value(&report).unwrap_or(Value::Null);
            let done = sqlx::query(
                "INSERT INTO agent_memory.monthly_report (user_id, month, report)
                 VALUES ($1, $2, $3) ON CONFLICT (user_id, month) DO NOTHING",
            )
            .bind(user)
            .bind(month)
            .bind(body)
            .execute(&self.pool)
            .await?;
            written += usize::try_from(done.rows_affected()).unwrap_or(0);
        }
        Ok(written)
    }

    /// One entry by id, superseded or not.
    ///
    /// The audit read: `by_kind` and `for_subject` show the current view, and
    /// this is how a caller follows a `superseded_by` pointer back through the
    /// history the current view hides.
    ///
    /// # Errors
    /// As [`Self::by_kind`].
    pub async fn get(&self, user_id: Uuid, entry_id: &str) -> Result<Option<Entry>, MemoryError> {
        let row =
            sqlx::query("SELECT * FROM agent_memory.entry WHERE user_id = $1 AND entry_id = $2")
                .bind(user_id)
                .bind(entry_id)
                .fetch_optional(&self.pool)
                .await?;
        row.as_ref().map(row_to_entry).transpose()
    }
    /// The live entries for one subject, newest first.
    ///
    /// Archived rows are excluded here rather than by the caller — the partial
    /// index is on the live set, and a caller that filtered afterwards would
    /// both miss the index and be a caller that can forget.
    ///
    /// # Errors
    /// [`MemoryError::Sqlx`] if the query fails; [`MemoryError::Unreadable`] for
    /// a row whose enum columns hold something no variant accepts.
    pub async fn for_subject(
        &self,
        user_id: Uuid,
        subject: &str,
        limit: i64,
    ) -> Result<Vec<Entry>, MemoryError> {
        self.subject_rows(user_id, subject, limit, false).await
    }

    /// [`Self::for_subject`] including entries derived from untrusted text (KM-10).
    /// The default retrieval excludes them; the conflict guard and the browser, which
    /// must see everything that exists, call this one.
    ///
    /// # Errors
    /// As [`Self::for_subject`].
    pub async fn for_subject_including_tainted(
        &self,
        user_id: Uuid,
        subject: &str,
        limit: i64,
    ) -> Result<Vec<Entry>, MemoryError> {
        self.subject_rows(user_id, subject, limit, true).await
    }

    async fn subject_rows(
        &self,
        user_id: Uuid,
        subject: &str,
        limit: i64,
        include_tainted: bool,
    ) -> Result<Vec<Entry>, MemoryError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_memory.entry \
              WHERE user_id = $1 AND subject = $2 AND NOT archived \
                AND superseded_by IS NULL \
                AND ($4 OR NOT COALESCE((extra ->> 'tainted')::boolean, false)) \
              ORDER BY created_at DESC LIMIT $3",
        )
        .bind(user_id)
        .bind(subject)
        .bind(limit)
        .bind(include_tainted)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_entry).collect()
    }

    /// Whether the user has told the platform to start Research without
    /// asking, for this subject (S-10.2: "a `user_preference` 'always start' per
    /// scope skips the offer").
    ///
    /// A live `user_preference` whose `extra.directive` is
    /// `always_start_research` and whose subject is `scope` (or `*` for every
    /// scope). Archived (deleted) preferences do not count, so deleting it
    /// restores the offer. The driver calls this where it would emit
    /// `Outcome::Offered`; the read is here so that decision is one query and
    /// not a scan of the preference list in the driver.
    ///
    /// # Errors
    /// As [`Self::by_kind`].
    pub async fn prefers_always_start(
        &self,
        user_id: Uuid,
        scope: &str,
    ) -> Result<bool, MemoryError> {
        let prefs = self.by_kind(user_id, Kind::UserPreference, 200).await?;
        Ok(prefs.iter().any(|e| {
            e.extra.get("directive").and_then(Value::as_str) == Some(ALWAYS_START_DIRECTIVE)
                && (e.subject == scope || e.subject == "*")
        }))
    }

    /// Entries of one kind, newest first.
    ///
    /// # Errors
    /// As [`Self::for_subject`].
    pub async fn by_kind(
        &self,
        user_id: Uuid,
        kind: Kind,
        limit: i64,
    ) -> Result<Vec<Entry>, MemoryError> {
        self.kind_rows(user_id, kind, limit, false).await
    }

    /// [`Self::by_kind`] including entries derived from untrusted text (KM-10).
    ///
    /// # Errors
    /// As [`Self::by_kind`].
    pub async fn by_kind_including_tainted(
        &self,
        user_id: Uuid,
        kind: Kind,
        limit: i64,
    ) -> Result<Vec<Entry>, MemoryError> {
        self.kind_rows(user_id, kind, limit, true).await
    }

    async fn kind_rows(
        &self,
        user_id: Uuid,
        kind: Kind,
        limit: i64,
        include_tainted: bool,
    ) -> Result<Vec<Entry>, MemoryError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_memory.entry \
              WHERE user_id = $1 AND kind = $2 AND NOT archived \
                AND superseded_by IS NULL \
                AND ($4 OR NOT COALESCE((extra ->> 'tainted')::boolean, false)) \
              ORDER BY created_at DESC LIMIT $3",
        )
        .bind(user_id)
        .bind(kind.as_str())
        .bind(limit)
        .bind(include_tainted)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_entry).collect()
    }
}

/// One retention sweep, as of `today` (an ISO date), logged; then the month's report
/// for every user who does not have one yet (S-4.16).
pub async fn run_retention_once(pool: &PgPool, today: &str) {
    let store = MemoryStore::new(pool.clone());
    match store.sweep_retention(today).await {
        Ok(r) => tracing::info!(
            examined = r.examined,
            archived = r.archived_now,
            staled = r.staled_now,
            "memory retention sweep"
        ),
        Err(e) => tracing::warn!(error = %e, "memory retention sweep failed"),
    }
    match store.publish_monthly_reports(today).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(reports = n, "memory store monthly report written"),
        Err(e) => tracing::warn!(error = %e, "memory store monthly report failed"),
    }
}

/// Spawn the daily retention loop (S-4.16): `memory::retention::retention` decided
/// what to do with an entry and nothing ever asked it, so no row ever archived.
/// The first sweep runs a minute after boot. Needs one line in the platform's
/// `main.rs`: `api::memory_store::spawn_retention_sweep(pg.clone());`.
pub fn spawn_retention_sweep(pool: PgPool) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            let today = chrono::Utc::now().date_naive().to_string();
            run_retention_once(&pool, &today).await;
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
}

fn evidence_class_str(c: EvidenceClass) -> &'static str {
    match c {
        EvidenceClass::DirectStat => "direct_stat",
        EvidenceClass::Derived => "derived",
        EvidenceClass::ModelSummary => "model_summary",
        EvidenceClass::Human => "human",
    }
}

fn row_to_entry(row: &sqlx::postgres::PgRow) -> Result<Entry, MemoryError> {
    let id: String = row.get("entry_id");
    let quantity: Option<String> = row.try_get("quantity").ok().flatten();
    let measurement = match quantity {
        Some(q) => {
            let dir: String = row.try_get("direction").ok().flatten().unwrap_or_default();
            let n: Option<i64> = row.try_get("n").ok().flatten();
            Some(Measurement {
                quantity: q,
                direction: direction_from(&dir, &id)?,
                value: row.try_get("value").ok().flatten().unwrap_or(0.0),
                se: row.try_get("se").ok().flatten(),
                n: n.and_then(|n| u64::try_from(n).ok()),
            })
        }
        None => None,
    };
    let kind_s: String = row.get("kind");
    let conf_s: String = row.get("confidence");
    let class_s: String = row.get("evidence_class");
    let start: NaiveDate = row.get("window_start");
    let end: NaiveDate = row.get("window_end");
    let valid_until: Option<NaiveDate> = row.try_get("valid_until").ok().flatten();
    let evidence_ids: Value = row.try_get("evidence_ids").unwrap_or(Value::Null);
    let corroborations: i32 = row.try_get("corroborations").unwrap_or(1);

    Ok(Entry {
        id: id.clone(),
        kind: kind_from(&kind_s, &id)?,
        subject: row.get("subject"),
        claim: row.get("claim"),
        evidence_ids: serde_json::from_value::<BTreeMap<String, String>>(evidence_ids)
            .unwrap_or_default(),
        measurement,
        confidence: confidence_from(&conf_s, &id)?,
        evidence_class: evidence_class_from(&class_s, &id)?,
        data_window: DataWindow::new(start.to_string(), end.to_string()),
        valid_until: valid_until.map(|d| d.to_string()),
        provenance: Provenance::new(
            row.get::<String, _>("prov_run_id"),
            row.get::<String, _>("prov_tool"),
            row.get::<String, _>("prov_tool_version"),
            row.get::<String, _>("prov_template_hash"),
        ),
        created_by: row.get("created_by"),
        superseded_by: row.try_get("superseded_by").ok().flatten(),
        conflicts_with: row.try_get("conflicts_with").unwrap_or_default(),
        stale: row.try_get("stale").unwrap_or(false),
        archived: row.try_get("archived").unwrap_or(false),
        corroborations: u32::try_from(corroborations).unwrap_or(1),
        corroborated_by: row.try_get("corroborated_by").unwrap_or_default(),
        cited_by_chain: row.try_get("cited_by_chain").unwrap_or(false),
        extra: row.try_get("extra").unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `Kind` round-trips through its string.
    ///
    /// The store writes `Kind::as_str` and reads it back through `kind_from`,
    /// and those are two separate matches. A kind added to one and not the other
    /// writes a row this module then cannot read — which is worse than failing,
    /// because the write succeeds.
    #[test]
    fn every_kind_survives_the_round_trip() {
        for k in [
            Kind::Finding,
            Kind::NegativeResult,
            Kind::InstrumentProfile,
            Kind::SignalRegistry,
            Kind::PlatformDefect,
            Kind::UserPreference,
            Kind::OpenQuestion,
        ] {
            let back = kind_from(k.as_str(), "t").expect("a kind this module wrote");
            assert_eq!(back, k, "`{}` does not read back", k.as_str());
        }
    }

    #[test]
    fn every_confidence_and_evidence_class_survives_the_round_trip() {
        for c in [
            Confidence::Established,
            Confidence::Suggestive,
            Confidence::Refuted,
        ] {
            assert_eq!(confidence_from(c.as_str(), "t").unwrap(), c);
        }
        for c in [
            EvidenceClass::DirectStat,
            EvidenceClass::Derived,
            EvidenceClass::ModelSummary,
            EvidenceClass::Human,
        ] {
            assert_eq!(evidence_class_from(evidence_class_str(c), "t").unwrap(), c);
        }
        for d in [Direction::Positive, Direction::Negative, Direction::Null] {
            assert_eq!(direction_from(direction_str(d), "t").unwrap(), d);
        }
    }

    /// An unreadable value is named rather than defaulted.
    ///
    /// Defaulting would make a corrupted row look like a `finding`, which is the
    /// kind whose confidence anything downstream trusts most.
    #[test]
    fn an_unknown_enum_value_is_an_error_and_not_a_default() {
        let e = kind_from("hunch", "f_1").expect_err("not a kind");
        match e {
            MemoryError::Unreadable { id, field, value } => {
                assert_eq!(id, "f_1");
                assert_eq!(field, "kind");
                assert_eq!(value, "hunch");
            }
            other => panic!("expected Unreadable, got {other}"),
        }
    }

    /// The window is parsed, not passed through as text.
    #[test]
    fn a_window_that_is_not_a_date_is_refused_before_the_database_sees_it() {
        let bad = DataWindow::new("last summer", "now");
        assert!(matches!(window_dates(&bad), Err(EntryError::NoWindow)));
        let good = DataWindow::new("2026-04-01", "2026-06-30");
        let (s, e) = window_dates(&good).expect("an ISO window parses");
        assert!(s < e);
    }
}
