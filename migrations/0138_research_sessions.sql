-- 0138 — a Research conversation is a session, and a session is its log
-- (AGENT-014 §2, §3, §10; ADR-0037; Set S task S-11.1).
--
-- ## One shape, not two
--
-- Continuous research could have been a second object beside a bounded Research
-- run. It is not: a bounded run is `continuous = false` with one agenda item and
-- one child. That is why `question` and `charter` are both nullable and a CHECK
-- decides which one a row must have — the alternative was two tables whose
-- readers (the desk, the digest, the agenda) would each have to ask which kind
-- of thing they were looking at before they could render it.
--
-- ## No status column, on purpose
--
-- `research_sessions` has no `state`. The state is `fold(session_events)`, the
-- way a campaign's phase is `fold(mlops.campaign_event)` (ADR-P2-04, AT-59). A
-- column holding a second copy of it is precisely the thing that goes wrong when
-- a driver dies between writing the event and writing the column, and there is
-- then no way to tell which of the two is the session.
--
-- The 0052 discipline is reused whole:
--
--   1. events are ordered within a session by `seq`, assigned by the trigger so
--      no writer chooses its own position;
--   2. an event that *is* a transition carries the state it moves to, and an
--      illegal move is refused at INSERT against every writer, not only against
--      the driver.
--
-- Most kinds are not transitions — an item added, a selection made, a run
-- started — and they carry `state = NULL`. The trigger numbers them and says
-- nothing else about them, which is the honest extent of what a row-level
-- trigger can check. Rules over *pairs* of events (a finish with no start) need
-- the payload and live in `research_session::fold`, which reports them as
-- anomalies and stops the driver claiming.
--
-- ## The exploration floor is CHECKed where it is stored
--
-- `exploration_share` lives inside the charter document and the CHECK reads it
-- there rather than in a denormalised column. A second copy of a number whose
-- whole point is that it cannot be lowered is a second place to lower it.
-- `research_session::charter::ExplorationShare` refuses the same value in Rust,
-- including through `serde`, and
-- `crates/invariants/tests/exploration_floor_is_one_number.rs` reads this file
-- to hold the two equal.

-- ── the session ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS research_sessions (
    session_id       UUID PRIMARY KEY,
    user_id          UUID NOT NULL,
    -- The conversation IS the session; with the toggle on it is the journal.
    conversation_id  UUID NOT NULL REFERENCES agent_conversations(conversation_id) ON DELETE CASCADE,
    continuous       BOOLEAN NOT NULL DEFAULT FALSE,
    question         TEXT,
    charter          JSONB,
    budget           JSONB NOT NULL DEFAULT '{}'::jsonb,
    schedule         JSONB NOT NULL DEFAULT '{}'::jsonb,
    autonomy         TEXT NOT NULL DEFAULT 'notify',
    replenish        BOOLEAN NOT NULL DEFAULT FALSE,
    review_every     JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- The profile it runs on. Continuous requires a frontier profile
    -- (AGENT-014 §8); which names those is a platform fact and is checked in
    -- `api`, not here, because this database does not know what profiles exist.
    tier             TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_digest_at   TIMESTAMPTZ,

    -- Exactly one of the two, decided by the toggle.
    CONSTRAINT a_session_is_a_question_or_a_charter CHECK (
        (continuous AND charter IS NOT NULL AND question IS NULL)
        OR (NOT continuous AND question IS NOT NULL AND charter IS NULL)
    ),
    -- AGENT-014 §5.2, INV-21. `COALESCE` to a value below the floor rather than
    -- letting a missing key produce NULL: a CHECK that evaluates to NULL passes,
    -- so a charter written without the key would otherwise slip through the one
    -- constraint this table exists to hold.
    CONSTRAINT exploration_never_below_the_floor CHECK (
        charter IS NULL
        OR COALESCE((charter->>'exploration_share')::double precision, -1) >= 0.10
    ),
    CONSTRAINT exploration_is_a_share CHECK (
        charter IS NULL
        OR COALESCE((charter->>'exploration_share')::double precision, 0) <= 1.0
    ),
    -- AGENT-014 §2.1: 1..3. Concurrency beyond that is a way to spend a day
    -- budget in an hour.
    CONSTRAINT concurrency_is_one_to_three CHECK (
        COALESCE((schedule->>'max_concurrent_runs')::int, 1) BETWEEN 1 AND 3
    ),
    CONSTRAINT autonomy_is_one_of_three CHECK (
        autonomy IN ('supervised', 'notify', 'autonomous')
    ),
    CONSTRAINT a_session_names_its_tier CHECK (btrim(tier) <> '')
);

-- Every Research conversation has exactly one session, which is what makes
-- "look up the session for this conversation" a lookup rather than a search
-- with a tie-break.
CREATE UNIQUE INDEX IF NOT EXISTS research_sessions_conversation
    ON research_sessions (conversation_id);

CREATE INDEX IF NOT EXISTS research_sessions_user
    ON research_sessions (user_id, created_at DESC);

-- ── the log ─────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS session_events (
    event_id    BIGSERIAL PRIMARY KEY,
    session_id  UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    -- Assigned by the trigger. A writer that sets it is overwritten, which is
    -- the point: two concurrent appends cannot both claim the same position and
    -- neither has to know what the last one was.
    seq         BIGINT,
    kind        TEXT NOT NULL,
    -- The state this event moves the session to, or NULL when it is not a
    -- transition. `research_session::fold::EventKind::moves_to` is the writer's
    -- copy of the same mapping.
    state       TEXT,
    payload_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT an_event_has_a_kind CHECK (btrim(kind) <> ''),
    CONSTRAINT a_state_is_one_of_six CHECK (
        state IS NULL
        OR state IN ('draft', 'running', 'paused', 'stalled', 'complete', 'stopped')
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS session_events_seq
    ON session_events (session_id, seq);

-- The transition table (AGENT-014 §2.1, §7). Held equal to
-- `research_session::fold::transition_ok` by
-- `crates/invariants/tests/a_session_transition_table_has_one_definition.rs`,
-- which walks every ordered pair of the six state words and compares the two
-- tables cell by cell.
--
-- `stopped` is the only terminal state. `complete` is not: a drained agenda
-- completes, and a human adding an item starts it again.
CREATE OR REPLACE FUNCTION session_transition_ok(prev TEXT, next TEXT)
RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN next = prev      THEN FALSE
        WHEN prev = 'stopped' THEN FALSE
        WHEN prev = 'draft'    THEN next IN ('running', 'stopped')
        WHEN prev = 'running'  THEN next IN ('paused', 'stalled', 'complete', 'stopped')
        WHEN prev = 'paused'   THEN next IN ('running', 'complete', 'stopped')
        WHEN prev = 'stalled'  THEN next IN ('running', 'complete', 'stopped')
        WHEN prev = 'complete' THEN next IN ('running', 'stopped')
        ELSE FALSE
    END
$$;

CREATE OR REPLACE FUNCTION check_session_transition()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    prev_state TEXT;
    prev_seq   BIGINT;
BEGIN
    SELECT seq INTO prev_seq
    FROM session_events
    WHERE session_id = NEW.session_id
    ORDER BY seq DESC
    LIMIT 1;

    -- The last event that *was* a transition, which may be many events back.
    SELECT state INTO prev_state
    FROM session_events
    WHERE session_id = NEW.session_id AND state IS NOT NULL
    ORDER BY seq DESC
    LIMIT 1;

    IF NEW.state IS NOT NULL THEN
        IF prev_state IS NULL THEN
            IF NEW.state <> 'draft' THEN
                RAISE EXCEPTION
                    'a session opens in draft, not %; a log whose first state is anything else '
                    'describes a session nobody created',
                    NEW.state;
            END IF;
        ELSIF NOT session_transition_ok(prev_state, NEW.state) THEN
            RAISE EXCEPTION
                'session % cannot go from % to % (AGENT-014 2.1)',
                NEW.session_id, prev_state, NEW.state;
        END IF;
    ELSIF prev_state IS NULL THEN
        RAISE EXCEPTION
            'session % has no state yet and this event does not give it one: the log opens with '
            'session_created',
            NEW.session_id;
    END IF;

    NEW.seq := COALESCE(prev_seq + 1, 0);
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_session_transition ON session_events;
CREATE TRIGGER trg_session_transition BEFORE INSERT ON session_events
    FOR EACH ROW EXECUTE FUNCTION check_session_transition();

-- A log is a record of what happened. Editing one makes it a record of now.
DROP TRIGGER IF EXISTS trg_session_events_no_change ON session_events;
CREATE TRIGGER trg_session_events_no_change
    BEFORE UPDATE OR DELETE ON session_events
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── the agenda ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS agenda_items (
    item_id     UUID PRIMARY KEY,
    session_id  UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    question    TEXT NOT NULL,
    subject     TEXT NOT NULL DEFAULT '',
    workflow    TEXT NOT NULL,
    -- Every source that wanted this question, not the last one to say so. A
    -- question three sources want outranks one that one source wants
    -- (AGENT-014 §4), which only works if the list is kept.
    sources     JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_refs JSONB NOT NULL DEFAULT '[]'::jsonb,
    dedupe_key  TEXT NOT NULL,
    score       JSONB,
    status      TEXT NOT NULL DEFAULT 'proposed',
    status_detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    depends_on  JSONB NOT NULL DEFAULT '[]'::jsonb,
    attempts    INT NOT NULL DEFAULT 0,
    yield_ref   TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at  TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,

    CONSTRAINT an_item_asks_something CHECK (btrim(question) <> ''),
    CONSTRAINT an_item_names_a_workflow CHECK (btrim(workflow) <> ''),
    CONSTRAINT an_item_has_a_key CHECK (btrim(dedupe_key) <> ''),
    CONSTRAINT an_item_status_is_one_of_nine CHECK (
        status IN ('proposed', 'approved', 'queued', 'claimed', 'running',
                   'done', 'vetoed', 'superseded', 'expired')
    ),
    -- AGENT-014 §6.6: one retry, then it expires. A counter that can climb past
    -- the limit is a counter nothing enforces.
    CONSTRAINT an_item_is_retried_once CHECK (attempts BETWEEN 0 AND 2)
);

-- Layer one of the three-layer duplicate refusal (AGENT-014 P3): within a
-- session, one live item per key. Terminal items are excluded, because asking a
-- finished question again is replication and replication is something the
-- platform is for.
CREATE UNIQUE INDEX IF NOT EXISTS agenda_items_live_key
    ON agenda_items (session_id, dedupe_key)
    WHERE status NOT IN ('done', 'vetoed', 'superseded', 'expired');

CREATE INDEX IF NOT EXISTS agenda_items_session_status
    ON agenda_items (session_id, status, created_at);

-- ── the digest ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS session_digests (
    digest_id   UUID PRIMARY KEY,
    session_id  UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    -- The period it covers, as the log's own positions: a digest is over events,
    -- not over wall-clock, so two digests cannot overlap or leave a gap.
    from_seq    BIGINT NOT NULL,
    to_seq      BIGINT NOT NULL,
    body        JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_digest_covers_forward CHECK (to_seq >= from_seq)
);

CREATE INDEX IF NOT EXISTS session_digests_session
    ON session_digests (session_id, created_at DESC);

-- A digest is what the human reads instead of the log. Rewriting one changes
-- what they were told they read.
DROP TRIGGER IF EXISTS trg_session_digests_no_change ON session_digests;
CREATE TRIGGER trg_session_digests_no_change
    BEFORE UPDATE OR DELETE ON session_digests
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── the rail ────────────────────────────────────────────────────────────────
-- A child conversation is grouped under the journal it was started from
-- (AGENT-014 §9). It is also how the conversation front door tells a child of a
-- session apart from a conversation a human opened: a child already belongs to a
-- session and must not be given a second one, and the unique index on
-- `research_sessions.conversation_id` would otherwise refuse its first turn.
ALTER TABLE agent_conversations
    ADD COLUMN IF NOT EXISTS parent_conversation_id UUID
        REFERENCES agent_conversations(conversation_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS agent_conversations_parent
    ON agent_conversations (parent_conversation_id)
    WHERE parent_conversation_id IS NOT NULL;

-- ── the child's way back ────────────────────────────────────────────────────
-- AGENT-010 §4 amended: a run knows which session and which item it is for, so
-- "what did this session actually run" is a query and not a reconstruction from
-- conversation titles.
ALTER TABLE agent_runs
    ADD COLUMN IF NOT EXISTS session_id UUID,
    ADD COLUMN IF NOT EXISTS agenda_item_id UUID;

CREATE INDEX IF NOT EXISTS agent_runs_session
    ON agent_runs (session_id, agenda_item_id);
