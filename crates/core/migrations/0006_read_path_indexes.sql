-- 0006: read-path indexes for the G-01 dashboard and the run-detail projection.
--
-- These tables are append-only ledgers (immutable-event triggers), so the
-- dashboard's per-poll scans grow without bound.  Verified against the live
-- database with `SET enable_seqscan=off`: every query below still chose a
-- sequential scan, i.e. no existing index is usable for it.
--
-- `LIKE 'f02-%'` needs `text_pattern_ops`: the database collation is
-- en_US.utf8, and a default btree operator class cannot serve a LIKE prefix
-- scan outside the C locale.

CREATE INDEX IF NOT EXISTS audit_events_aggregate_id_pattern
    ON audit_events (aggregate_id text_pattern_ops);

CREATE INDEX IF NOT EXISTS audit_events_correlation_id_pattern
    ON audit_events (correlation_id text_pattern_ops);

CREATE INDEX IF NOT EXISTS audit_events_occurred_at
    ON audit_events (occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS trade_facts_intent_id
    ON trade_facts (intent_id, occurred_at_ms);

-- The overview loads the most recent 1200 snapshots regardless of account;
-- the existing unique key leads with account_id and cannot serve it.
CREATE INDEX IF NOT EXISTS balance_snapshots_source_observed_at
    ON balance_snapshots (source, observed_at_ms DESC);
