-- 0139 — the unit a digest's spend is in, and a digest written once
-- (AGENT-014 §2.4, §7.1; Set S tasks S-11.8, S-11.12, wired by
-- `api::session_driver`).
--
-- ## What this deliberately does not add
--
-- A table for the stall diagnosis. `0138`'s `session_events.payload_json` is
-- JSONB, append-only (`trg_session_events_no_change`), and
-- `research_session::stall::Diagnosis::event_payload` writes the whole
-- diagnosis into it — which rule, every item, every suggested amendment, the
-- window and the run counts. A second home for the same object would be a
-- second answer to "why did this session stall", and the one a reader reached
-- for first would be whichever the last writer remembered to fill. `0138`'s
-- `session_digests` is likewise already the right shape for the digest body.
--
-- So this migration adds exactly two things that are not there, and both are
-- refusals rather than storage.
--
-- ## 1. A digest names the unit its numbers are in
--
-- `digest::BudgetSummary.spent` and its cost-per-knowledge-unit line are read
-- by a human beside a day budget denominated in **dollars**
-- (`charter::SessionBudget.usd_per_day`). The number the platform can actually
-- measure per run is **trials**: `agent_runs.trials_spent` exists and there is
-- no per-run USD anywhere — `llm_usage.cost_usd` keys on the auth session, not
-- on the run, which is the same reason `select.rs` scores cost in trials.
--
-- The stall detector survives that ambiguity because cost drift is a ratio and
-- the unit cancels. A digest does not: "3.00 spent over 2 runs" beside a
-- "$25/day" bar reads as dollars, and a reader who believes it is reading
-- dollars when they are reading trials is reading a number nobody measured.
--
-- `spend_unit` is therefore NOT NULL with **no default**. Every writer of a
-- digest has to say which unit it counted in, and the CHECK is the list of
-- units that exist. When per-run USD lands, the writer passes `usd` and old
-- rows stay honest about having been trials — which is what a default of
-- `'trials'` would have quietly destroyed.
--
-- ## 2. A period gets one digest
--
-- The driver writes a digest every `review_every` and again on stall, complete
-- and stop. A driver killed between writing the digest and appending
-- `digest_written` re-derives the same period from `Folded::last_digest_seq`
-- and writes it again — so without this index the human would read the same
-- stretch of the session twice and could not tell which of the two was the one
-- they were told about.
--
-- The index is on `(session_id, to_seq)` because `to_seq` is the period's
-- identity: `from_seq` is the last digest's `to_seq + 1` and is therefore
-- derived from it. With `sessions::write_digest`'s `ON CONFLICT DO NOTHING`
-- and the v5 `digest_id` derived from the same pair, writing a digest is
-- idempotent on the pair the driver can always recompute.

-- ── the unit ────────────────────────────────────────────────────────────────
-- `0138` put an append-only trigger on this table, and it refuses UPDATE as
-- well as DELETE. Back-filling the column is an UPDATE, so the trigger comes
-- off for the length of the back-fill and goes back on at the end. The three
-- statements are one migration and therefore one transaction: there is no
-- window in which a session digest is mutable to anybody else.
DROP TRIGGER IF EXISTS trg_session_digests_no_change ON session_digests;

ALTER TABLE session_digests
    ADD COLUMN IF NOT EXISTS spend_unit TEXT;

-- Every row written before this column existed was written by a build whose
-- only measurable per-run cost was trials. Saying so is not a default; it is
-- the fact about those rows.
UPDATE session_digests SET spend_unit = 'trials' WHERE spend_unit IS NULL;

ALTER TABLE session_digests
    ALTER COLUMN spend_unit SET NOT NULL;

ALTER TABLE session_digests
    ADD CONSTRAINT a_digest_names_the_unit_it_counted_in CHECK (
        spend_unit IN ('trials', 'usd')
    );

COMMENT ON COLUMN session_digests.spend_unit IS
    'What `body->budget->spent` and `cost_per_knowledge_unit` are denominated '
    'in. `trials` today, because `agent_runs.trials_spent` is the only per-run '
    'cost this platform records — `llm_usage.cost_usd` keys on the auth session '
    'rather than the run. NOT NULL with no default so a writer has to say which '
    'it counted, and so a row written in trials cannot later be read as dollars.';

CREATE TRIGGER trg_session_digests_no_change
    BEFORE UPDATE OR DELETE ON session_digests
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── one digest per period ───────────────────────────────────────────────────
CREATE UNIQUE INDEX IF NOT EXISTS session_digests_period
    ON session_digests (session_id, to_seq);
