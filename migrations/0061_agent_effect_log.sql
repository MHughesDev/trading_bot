-- The effect log IS the run (AGENT-010 §5; Set S task S-2.5).
--
-- `agent_messages` records what the UI needs to draw a timeline: an assistant
-- message, a tool call, a result. It is a *rendering* of the run, and it cannot be
-- replayed — the kinds are presentational (`status`, `final`), the payloads are
-- whatever each writer felt like putting in a JSONB column, and nothing distinguishes
-- an effect the loop PRODUCED from an input it CONSUMED.
--
-- That distinction is the whole of resume. A run that crashed between deciding to
-- call a tool and calling it must not call it twice; a run that crashed after the
-- call must not lose the result. Neither is answerable from a transcript, and both
-- are answerable from an ordered log of effects and inputs.
--
-- So: one row per `Effect` the loop emitted and one per `Input` it was fed, keyed on
-- the **variant name** rather than on a presentational kind, in the order they
-- happened. `agent_messages` keeps being written beside this (§13 step 1) so the UI
-- keeps working while the two coexist; it is a view of the run, not the run.
--
-- ## The name
--
-- AGENT-010 §5 and Set S task S-2.5 both call this table `agent_events`. **That name
-- was already taken**, by migration 0036: an SSE event stream keyed on `session_id`,
-- which the UI reads to draw a live session. Two tables with one name is not a
-- collision anyone would notice until a query joined the wrong one, so this is
-- `agent_effect_log` — which is also what it is. The spec's name is recorded here
-- rather than silently substituted.

CREATE TABLE IF NOT EXISTS agent_effect_log (
    run_id       UUID NOT NULL REFERENCES agent_runs(run_id) ON DELETE CASCADE,

    -- Monotonic within a run, assigned by the writer. Part of the primary key rather
    -- than a surrogate id, because the ORDER is the content: an effect log whose
    -- rows cannot be put back in sequence is a set of facts about a run and not the
    -- run itself.
    seq          INT NOT NULL,

    -- 'effect' | 'input' | 'state'. Which side of the loop this row is.
    --
    -- An effect is something the loop decided to do and has not done yet. An input
    -- is the world answering. A state row is the working state at a step boundary,
    -- so a replay can be checked against what the run actually believed rather than
    -- only against what it did.
    direction    TEXT NOT NULL CHECK (direction IN ('effect', 'input', 'state')),

    -- The variant name: 'CallModel', 'ExecuteTool', 'ModelReply', 'ToolResult'.
    --
    -- Typed on purpose. `agent_messages.kind` is a presentational vocabulary that
    -- cannot be replayed, and a log keyed on it can tell you a tool was called but
    -- not which effect the loop was serving when it happened.
    kind         TEXT NOT NULL CHECK (kind <> ''),

    payload_json JSONB NOT NULL,

    -- The harness that wrote it. A replay by a different version is not a replay,
    -- and S-2.6 refuses one rather than producing a run that looks resumed.
    harness_version TEXT NOT NULL CHECK (harness_version <> ''),

    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (run_id, seq)
);

-- The read every resume does: the whole log for one run, in order.
CREATE INDEX IF NOT EXISTS agent_effect_log_run_seq_idx
    ON agent_effect_log (run_id, seq);

-- The read the replay checker does: find runs whose log ends on an effect with no
-- answering input, which is exactly the set that was interrupted mid-flight.
CREATE INDEX IF NOT EXISTS agent_effect_log_effects_idx
    ON agent_effect_log (run_id, seq DESC)
    WHERE direction = 'effect';

-- An effect log is evidence about what a run did. Editing one makes every later
-- claim about that run unfalsifiable, so the same rule as every other evidence
-- table on this platform: append only.
DROP TRIGGER IF EXISTS trg_agent_effect_log_immutable ON agent_effect_log;
CREATE TRIGGER trg_agent_effect_log_immutable
    BEFORE UPDATE OR DELETE ON agent_effect_log
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

COMMENT ON TABLE agent_effect_log IS
    'AGENT-010 §5. One row per Effect the loop emitted and per Input it consumed, in '
    'order, keyed on the variant name. Append-only. This is the run; agent_messages '
    'is a rendering of it for the UI and is written beside it during the migration.';

COMMENT ON COLUMN agent_effect_log.direction IS
    'Which side of the loop: an effect is a decision not yet acted on, an input is '
    'the world answering, a state row is what the run believed at a step boundary. '
    'The distinction is what makes resume possible — a run that crashed between '
    'deciding to write and writing must not write twice.';
