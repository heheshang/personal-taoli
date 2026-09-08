
CREATE TABLE IF NOT EXISTS schema_migrations (
    version BIGINT PRIMARY KEY,
    applied_at_ms BIGINT NOT NULL CHECK (applied_at_ms >= 0)
);

CREATE TABLE IF NOT EXISTS paper_balances (
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    asset TEXT NOT NULL CHECK (asset <> ''),
    observed_total NUMERIC NOT NULL CHECK (observed_total >= 0),
    observed_free NUMERIC NOT NULL CHECK (observed_free >= 0),
    local_reserved NUMERIC NOT NULL DEFAULT 0 CHECK (local_reserved >= 0),
    observed_at_ms BIGINT NOT NULL CHECK (observed_at_ms > 0),
    PRIMARY KEY (account_id, venue, asset),
    CHECK (observed_free <= observed_total),
    CHECK (local_reserved <= observed_free)
);

CREATE TABLE IF NOT EXISTS execution_plans (
    plan_id TEXT PRIMARY KEY CHECK (plan_id <> ''),
    request_id TEXT NOT NULL UNIQUE CHECK (request_id <> ''),
    request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    instrument_id TEXT NOT NULL CHECK (instrument_id <> ''),
    opportunity_id TEXT NOT NULL CHECK (opportunity_id <> ''),
    strategy_config_version TEXT NOT NULL CHECK (strategy_config_version <> ''),
    target_quantity NUMERIC NOT NULL CHECK (target_quantity > 0),
    max_unmatched_exposure NUMERIC NOT NULL CHECK (max_unmatched_exposure >= 0),
    state TEXT NOT NULL CHECK (state IN ('RESERVED', 'ABORTED', 'COMPLETED', 'MANUAL_REQUIRED')),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms >= 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS execution_plans_one_active_domain
    ON execution_plans (account_id, instrument_id)
    WHERE state = 'RESERVED';

CREATE TABLE IF NOT EXISTS risk_decisions (
    plan_id TEXT PRIMARY KEY REFERENCES execution_plans(plan_id) ON DELETE CASCADE,
    approved BOOLEAN NOT NULL CHECK (approved),
    input_digest TEXT NOT NULL CHECK (length(input_digest) = 64),
    decided_at_ms BIGINT NOT NULL CHECK (decided_at_ms >= 0)
);

CREATE TABLE IF NOT EXISTS order_intents (
    intent_id TEXT PRIMARY KEY CHECK (intent_id <> ''),
    plan_id TEXT NOT NULL REFERENCES execution_plans(plan_id) ON DELETE CASCADE,
    leg_id TEXT NOT NULL CHECK (leg_id <> ''),
    attempt_id TEXT NOT NULL UNIQUE CHECK (attempt_id <> ''),
    client_order_id TEXT NOT NULL UNIQUE CHECK (client_order_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    side TEXT NOT NULL CHECK (side IN ('BUY', 'SELL')),
    quantity NUMERIC NOT NULL CHECK (quantity > 0),
    limit_price NUMERIC NOT NULL CHECK (limit_price > 0),
    submission_status TEXT NOT NULL CHECK (submission_status = 'NOT_SENT'),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms >= 0),
    UNIQUE (plan_id, leg_id)
);

CREATE TABLE IF NOT EXISTS fund_reservations (
    reservation_id TEXT PRIMARY KEY CHECK (reservation_id <> ''),
    plan_id TEXT NOT NULL REFERENCES execution_plans(plan_id) ON DELETE CASCADE,
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    asset TEXT NOT NULL CHECK (asset <> ''),
    amount NUMERIC NOT NULL CHECK (amount > 0),
    state TEXT NOT NULL CHECK (state = 'LOCAL_RESERVED'),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms >= 0),
    UNIQUE (plan_id, venue, asset)
);

CREATE TABLE IF NOT EXISTS audit_events (
    event_id TEXT PRIMARY KEY CHECK (event_id <> ''),
    aggregate_id TEXT NOT NULL CHECK (aggregate_id <> ''),
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version > 0),
    event_type TEXT NOT NULL CHECK (event_type <> ''),
    correlation_id TEXT NOT NULL CHECK (correlation_id <> ''),
    causation_id TEXT NOT NULL CHECK (causation_id <> ''),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms >= 0),
    payload JSONB NOT NULL,
    UNIQUE (aggregate_id, aggregate_version)
);

CREATE OR REPLACE FUNCTION reject_audit_event_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'audit_events are immutable';
END;
$$;

DROP TRIGGER IF EXISTS audit_events_immutable ON audit_events;
CREATE TRIGGER audit_events_immutable
BEFORE UPDATE OR DELETE ON audit_events
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

INSERT INTO schema_migrations(version, applied_at_ms)
VALUES (1, (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT)
ON CONFLICT (version) DO NOTHING;
