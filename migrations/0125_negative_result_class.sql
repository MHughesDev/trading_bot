-- 0125 — a negative result's class is the funnel's, not the model's
-- (AGENT-007 §3.2; Set S task S-4.4).
--
-- ## The failure this prevents
--
-- S-4.4's acceptance: "a negative result cannot be created with a class the
-- funnel did not produce". The sentence that follows in the plan says why it
-- matters — this is the mechanism that stops `capacity_insufficient` being used
-- to keep a dead idea alive. A model that may choose the class may always choose
-- the one whose `retry_condition` is "when the platform is bigger", and a
-- negative result that can be re-opened by naming it differently is not a
-- negative result.
--
-- ## Rust already makes it unforgeable; this makes it unbypassable
--
-- `memory::negative::derive_negative_result` has no class parameter: the class
-- comes from `FailureClass::from_source`, which reads an event the funnel
-- emitted. There is no path from a string the model chose to the stored class.
--
-- That is a property of one function, and it holds until the second writer. The
-- CHECK below is the same rule where a writer cannot route around it — the same
-- argument `0122`'s header makes about provenance, and the reason S-4.4's
-- acceptance is phrased as "cannot be created" rather than "is not created".
--
-- ## Why a CHECK over JSONB and not a column
--
-- The class is a kind-specific field: only `negative_result` has one, and §2.2
-- puts those in `extra`. Lifting it to a column would give the other six kinds a
-- null they can never fill, and a `CHECK (kind <> 'negative_result' OR ...)` is
-- the same rule with a column nobody reads.
--
-- The constraint is written so it says nothing at all about the other kinds:
-- `extra ? 'failure_class'` is false for them, so the implication is vacuously
-- true and a finding is never asked about a field it does not have.

ALTER TABLE agent_memory.entry
  ADD CONSTRAINT a_failure_class_is_one_the_funnel_produces CHECK (
    NOT (extra ? 'failure_class')
    OR extra->>'failure_class' IN (
      -- `memory::negative::FailureClass`, all ten.
      'weak_effect',
      'capacity_insufficient',
      'costs_dominate',
      'replication_failed',
      'insufficient_data',
      'no_better_than_null',
      -- AGENT-013 §8.2 added these four.
      'leakage',
      'protocol_unstable',
      'fragile_parameters',
      'regime_specific'
    )
  );

-- A negative result without a ticket is a refusal nobody can trace back to the
-- gate that issued it — and the ticket is what `retry_condition` is checked
-- against when somebody asks whether the idea may be re-opened.
ALTER TABLE agent_memory.entry
  ADD CONSTRAINT a_negative_result_names_its_ticket CHECK (
    kind <> 'negative_result'
    OR (extra ? 'ticket_id' AND btrim(extra->>'ticket_id') <> '')
  );

-- Every negative result carries a class. The pair of constraints is deliberate:
-- the one above says a class must be a real one, and this says there must be a
-- class. Without both, an entry with no `failure_class` at all satisfies the
-- first vacuously.
ALTER TABLE agent_memory.entry
  ADD CONSTRAINT a_negative_result_carries_a_class CHECK (
    kind <> 'negative_result' OR extra ? 'failure_class'
  );
