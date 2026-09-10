
CREATE TABLE IF NOT EXISTS execution_facts (
    plan_id TEXT PRIMARY KEY REFERENCES execution_plans(plan_id) ON DELETE CASCADE,
    target_quantity NUMERIC NOT NULL CHECK (target_quantity > 0),
    max_unmatched_exposure NUMERIC NOT NULL CHECK (max_unmatched_exposure >= 0),
    compensation_budget NUMERIC NOT NULL CHECK (compensation_budget >= 0),
    state TEXT NOT NULL CHECK (state IN ('PLANNED', 'RUNNING', 'COMPENSATION_PLANNED', 'COMPLETED', 'MANUAL_REQUIRED')),
    leg_a_filled NUMERIC NOT NULL CHECK (leg_a_filled >= 0),
    leg_b_filled NUMERIC NOT NULL CHECK (leg_b_filled >= 0),
    unmatched_quantity NUMERIC NOT NULL CHECK (unmatched_quantity >= 0),
    unmatched_exposure NUMERIC NOT NULL CHECK (unmatched_exposure >= 0),
    compensation_spent NUMERIC NOT NULL DEFAULT 0 CHECK (compensation_spent >= 0),
    last_version BIGINT NOT NULL CHECK (last_version >= 0),
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE TABLE IF NOT EXISTS execution_events (
    event_id TEXT PRIMARY KEY CHECK (event_id <> ''),
    plan_id TEXT NOT NULL REFERENCES execution_facts(plan_id) ON DELETE CASCADE,
    event_version BIGINT NOT NULL CHECK (event_version > 0),
    event_type TEXT NOT NULL CHECK (event_type IN ('EVALUATED', 'COMPENSATION_DECIDED', 'MANUAL_ESCALATED', 'COMPLETED')),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms >= 0),
    payload JSONB NOT NULL,
    UNIQUE (plan_id, event_version)
);

CREATE TABLE IF NOT EXISTS compensation_decisions (
    decision_id TEXT PRIMARY KEY CHECK (decision_id <> ''),
    plan_id TEXT NOT NULL REFERENCES execution_facts(plan_id) ON DELETE CASCADE,
    event_version BIGINT NOT NULL CHECK (event_version > 0),
    decision TEXT NOT NULL CHECK (decision IN ('NO_ACTION', 'COMPENSATION_PLANNED', 'MANUAL_REQUIRED')),
    quantity NUMERIC NOT NULL CHECK (quantity >= 0),
    estimated_cost NUMERIC NOT NULL CHECK (estimated_cost >= 0),
    reason TEXT NOT NULL CHECK (reason <> ''),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms >= 0),
    UNIQUE (plan_id, event_version)
);

DROP TRIGGER IF EXISTS execution_events_immutable ON execution_events;
CREATE TRIGGER execution_events_immutable
BEFORE UPDATE OR DELETE ON execution_events
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

DROP TRIGGER IF EXISTS compensation_decisions_immutable ON compensation_decisions;
CREATE TRIGGER compensation_decisions_immutable
BEFORE UPDATE OR DELETE ON compensation_decisions
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();
