CREATE TABLE IF NOT EXISTS ledger_events (
    event_id TEXT PRIMARY KEY CHECK (event_id <> ''),
    business_key TEXT NOT NULL UNIQUE CHECK (business_key <> ''),
    event_type TEXT NOT NULL CHECK (event_type <> ''),
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms > 0),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms > 0),
    payload JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS ledger_lines (
    line_id BIGSERIAL PRIMARY KEY,
    event_id TEXT NOT NULL REFERENCES ledger_events(event_id),
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    asset TEXT NOT NULL CHECK (asset <> ''),
    direction TEXT NOT NULL CHECK (direction IN ('DEBIT', 'CREDIT')),
    amount NUMERIC NOT NULL CHECK (amount > 0),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms > 0)
);

CREATE INDEX IF NOT EXISTS ledger_lines_account_asset
    ON ledger_lines(account_id, venue, asset, line_id);

CREATE TABLE IF NOT EXISTS balance_snapshots (
    snapshot_id TEXT PRIMARY KEY CHECK (snapshot_id <> ''),
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    asset TEXT NOT NULL CHECK (asset <> ''),
    total NUMERIC NOT NULL CHECK (total >= 0),
    free NUMERIC NOT NULL CHECK (free >= 0),
    locked NUMERIC NOT NULL CHECK (locked >= 0),
    observed_at_ms BIGINT NOT NULL CHECK (observed_at_ms > 0),
    source TEXT NOT NULL CHECK (source <> ''),
    completeness TEXT NOT NULL CHECK (completeness IN ('COMPLETE', 'PARTIAL', 'STALE')),
    UNIQUE(account_id, venue, asset, observed_at_ms)
);

CREATE TABLE IF NOT EXISTS reconciliation_runs (
    run_id TEXT PRIMARY KEY CHECK (run_id <> ''),
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    status TEXT NOT NULL CHECK (status IN ('RUNNING', 'MATCHED', 'ATTENTION_REQUIRED')),
    started_at_ms BIGINT NOT NULL CHECK (started_at_ms > 0),
    finished_at_ms BIGINT,
    UNIQUE(account_id, venue, started_at_ms)
);

CREATE TABLE IF NOT EXISTS reconciliation_differences (
    difference_id TEXT PRIMARY KEY CHECK (difference_id <> ''),
    run_id TEXT NOT NULL REFERENCES reconciliation_runs(run_id),
    asset TEXT NOT NULL CHECK (asset <> ''),
    category TEXT NOT NULL CHECK (category IN ('DELAYED', 'MISSING', 'CONFLICT', 'EXPLAINED_ADJUSTMENT')),
    local_amount NUMERIC NOT NULL CHECK (local_amount >= 0),
    external_amount NUMERIC NOT NULL CHECK (external_amount >= 0),
    explanation TEXT,
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms > 0)
);

CREATE TABLE IF NOT EXISTS control_commands (
    command_id TEXT PRIMARY KEY CHECK (command_id <> ''),
    request_id TEXT NOT NULL UNIQUE CHECK (request_id <> ''),
    actor TEXT NOT NULL CHECK (actor <> ''),
    action TEXT NOT NULL CHECK (action IN ('PAUSE', 'CANCEL_ORDERS', 'REDUCE_EXPOSURE', 'RESUME', 'HALT')),
    scope JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('RECEIVED', 'EXECUTING', 'SUCCEEDED', 'FAILED', 'PARTIAL', 'EXPIRED')),
    expires_at_ms BIGINT NOT NULL CHECK (expires_at_ms > 0),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms > 0),
    result JSONB
);

CREATE TABLE IF NOT EXISTS control_audit_logs (
    audit_id BIGSERIAL PRIMARY KEY,
    command_id TEXT NOT NULL REFERENCES control_commands(command_id),
    actor TEXT NOT NULL CHECK (actor <> ''),
    action TEXT NOT NULL CHECK (action <> ''),
    before_state JSONB,
    after_state JSONB,
    result JSONB,
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms > 0)
);

DROP TRIGGER IF EXISTS ledger_events_immutable ON ledger_events;
CREATE TRIGGER ledger_events_immutable
BEFORE UPDATE OR DELETE ON ledger_events
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

DROP TRIGGER IF EXISTS ledger_lines_immutable ON ledger_lines;
CREATE TRIGGER ledger_lines_immutable
BEFORE UPDATE OR DELETE ON ledger_lines
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

DROP TRIGGER IF EXISTS balance_snapshots_immutable ON balance_snapshots;
CREATE TRIGGER balance_snapshots_immutable
BEFORE UPDATE OR DELETE ON balance_snapshots
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

DROP TRIGGER IF EXISTS reconciliation_differences_immutable ON reconciliation_differences;
CREATE TRIGGER reconciliation_differences_immutable
BEFORE UPDATE OR DELETE ON reconciliation_differences
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

INSERT INTO schema_migrations(version, applied_at_ms)
VALUES (4, (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT)
ON CONFLICT (version) DO NOTHING;
