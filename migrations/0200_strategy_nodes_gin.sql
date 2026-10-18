-- 0200 - the used-by lookup is an index, not a scan (Set H H-4.5).
--
-- GET /api/models/{id}/used-by asks which strategy definitions have a node whose
-- model_ref names a model. That is a JSONB containment on the node list; this
-- index serves it, so the cost is the matches rather than every definition.

CREATE INDEX IF NOT EXISTS strategy_definitions_nodes_gin
    ON strategy_definitions USING GIN ((definition_json->'nodes') jsonb_path_ops);
