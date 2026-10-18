-- Per-user acknowledge/dismiss state for the alert inbox (FB-RX-005).
-- A critical item still in force cannot be dismissed; the server enforces that,
-- this table only records what a user has acted on.
CREATE TABLE IF NOT EXISTS alert_inbox_state (
    user_id  UUID        NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    source   TEXT        NOT NULL CHECK (source IN ('monitor', 'price')),
    item_id  TEXT        NOT NULL,
    state    TEXT        NOT NULL CHECK (state IN ('acknowledged', 'dismissed')),
    acted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, source, item_id)
);

GRANT SELECT, INSERT, UPDATE ON alert_inbox_state TO app_role;
