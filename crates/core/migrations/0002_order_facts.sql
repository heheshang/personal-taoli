CREATE TABLE IF NOT EXISTS order_facts (
    intent_id TEXT PRIMARY KEY REFERENCES order_intents(intent_id) ON DELETE CASCADE,
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    instrument_id TEXT NOT NULL CHECK (instrument_id <> ''),
    venue TEXT NOT NULL CHECK (venue <> ''),
    ordered_quantity NUMERIC NOT NULL CHECK (ordered_quantity > 0),
    submission_status TEXT NOT NULL CHECK (submission_status IN ('NOT_SENT', 'IN_FLIGHT', 'ACCEPTED', 'DEFINITELY_REJECTED', 'UNKNOWN')),
    cancel_status TEXT NOT NULL CHECK (cancel_status IN ('NONE', 'REQUESTED', 'UNKNOWN', 'CONFIRMED')),
    reconciliation_status TEXT NOT NULL CHECK (reconciliation_status IN ('PENDING', 'MATCHED', 'CONFLICT')),
    exchange_order_id TEXT,
    filled_quantity NUMERIC NOT NULL DEFAULT 0 CHECK (filled_quantity >= 0),
    last_fact_version BIGINT NOT NULL DEFAULT 0 CHECK (last_fact_version >= 0),
    last_error TEXT,
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms >= 0),
    CHECK (exchange_order_id IS NULL OR exchange_order_id <> ''),
    UNIQUE (venue, exchange_order_id)
);

CREATE INDEX IF NOT EXISTS order_facts_domain_status
    ON order_facts (account_id, instrument_id, submission_status);

CREATE TABLE IF NOT EXISTS order_action_facts (
    fact_id TEXT PRIMARY KEY CHECK (fact_id <> ''),
    intent_id TEXT NOT NULL REFERENCES order_facts(intent_id) ON DELETE CASCADE,
    fact_version BIGINT NOT NULL CHECK (fact_version > 0),
    action TEXT NOT NULL CHECK (action IN ('SUBMIT_STARTED', 'SUBMIT_ACCEPTED', 'SUBMIT_REJECTED', 'SUBMIT_UNKNOWN', 'QUERY_NOT_FOUND', 'QUERY_FOUND', 'CANCEL_REQUESTED', 'CANCEL_UNKNOWN', 'CANCEL_CONFIRMED', 'TRADE_RECORDED', 'TRADE_DUPLICATE', 'TRADE_CONFLICT')),
    request_digest TEXT CHECK (request_digest IS NULL OR length(request_digest) = 64),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms >= 0),
    payload JSONB NOT NULL,
    UNIQUE (intent_id, fact_version)
);

CREATE TABLE IF NOT EXISTS order_investigations (
    query_id TEXT PRIMARY KEY CHECK (query_id <> ''),
    intent_id TEXT NOT NULL REFERENCES order_facts(intent_id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN ('NOT_FOUND', 'FOUND')),
    visibility_deadline_ms BIGINT NOT NULL CHECK (visibility_deadline_ms >= 0),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms >= 0),
    payload JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS trade_facts (
    venue TEXT NOT NULL CHECK (venue <> ''),
    exchange_order_id TEXT NOT NULL CHECK (exchange_order_id <> ''),
    trade_id TEXT NOT NULL CHECK (trade_id <> ''),
    intent_id TEXT NOT NULL REFERENCES order_facts(intent_id) ON DELETE CASCADE,
    trade_digest TEXT NOT NULL CHECK (length(trade_digest) = 64),
    quantity NUMERIC NOT NULL CHECK (quantity > 0),
    price NUMERIC NOT NULL CHECK (price > 0),
    fee_asset TEXT NOT NULL CHECK (fee_asset <> ''),
    fee_amount NUMERIC NOT NULL CHECK (fee_amount >= 0),
    source_sequence BIGINT NOT NULL CHECK (source_sequence >= 0),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms >= 0),
    PRIMARY KEY (venue, exchange_order_id, trade_id)
);

DROP TRIGGER IF EXISTS order_action_facts_immutable ON order_action_facts;
CREATE TRIGGER order_action_facts_immutable
BEFORE UPDATE OR DELETE ON order_action_facts
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

DROP TRIGGER IF EXISTS order_investigations_immutable ON order_investigations;
CREATE TRIGGER order_investigations_immutable
BEFORE UPDATE OR DELETE ON order_investigations
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();

DROP TRIGGER IF EXISTS trade_facts_immutable ON trade_facts;
CREATE TRIGGER trade_facts_immutable
BEFORE UPDATE OR DELETE ON trade_facts
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();
