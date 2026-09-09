-- Service tokens: long-lived bearer sessions for headless clients (MCP server,
-- internal agent runs). Same sessions table, distinguished by kind.

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS label TEXT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'web';
-- kind: 'web' (interactive login) | 'service' (minted API token)
