-- 0068 — the user's own knowledge (AGENT-007 §3.4; Set S task S-4.6).
--
-- Answers to `ask_user`, and turns that begin "remember that" or "from now on",
-- become entries the agent is handed at the top of every brief in scope.
--
-- §3.4 calls this the cheapest large gain in understanding, and the reason is
-- unglamorous: the agent stops re-asking a question the user has answered, and
-- stops re-deciding something the user already decided. Neither is a reasoning
-- improvement. Both are the difference between a colleague and an intern.
--
-- This is a VIEW over `knowledge.memory_entry`, not a second table. A preference
-- is a memory entry of kind `user_preference` — it is retrieved by the same
-- ranker, archived by the same retention job, and cited by the same log. A
-- parallel table would need its own retrieval path, and the second retrieval
-- path is the one that quietly stops being maintained.

CREATE OR REPLACE VIEW knowledge.user_preference AS
SELECT
  entry_id,
  tenant_id,
  user_id,
  -- global | instrument | strategy. `extra->>'scope'` is where §3.4 puts it.
  COALESCE(extra->>'scope', 'global') AS scope,
  subject,
  claim   AS statement,
  created_at,
  archived
FROM knowledge.memory_entry
WHERE kind = 'user_preference';

GRANT SELECT ON knowledge.user_preference TO app_role, agent_role;

-- Deleting a preference is the user withdrawing an instruction, and it must
-- actually take effect on the next brief — §3.4's second half, and S-4.6's
-- second acceptance.
--
-- It archives rather than deletes, for the same reason everything else here
-- does: a finding recorded *because* the user said "always use 1h bars" cites
-- that preference, and a hard delete would break the chain that explains why the
-- finding is about 1h bars at all. Archived is out of retrieval and out of every
-- brief, which is the whole of what the user asked for.
CREATE OR REPLACE FUNCTION knowledge.forget_preference(p_entry_id UUID, p_user_id UUID)
RETURNS BOOLEAN AS $$
DECLARE hit INT;
BEGIN
  UPDATE knowledge.memory_entry
     SET archived = TRUE, stale = TRUE, updated_at = now()
   WHERE entry_id = p_entry_id
     AND user_id  = p_user_id
     AND kind     = 'user_preference'
     AND NOT archived;
  GET DIAGNOSTICS hit = ROW_COUNT;
  RETURN hit > 0;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

REVOKE ALL ON FUNCTION knowledge.forget_preference(UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION knowledge.forget_preference(UUID, UUID) TO app_role;
-- Deliberately NOT granted to agent_role. A preference the agent can delete is a
-- preference the agent can decline to follow, and P1's whole argument is that
-- the model does not get to edit what it is told.
