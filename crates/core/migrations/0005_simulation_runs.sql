-- G-01 simulation dashboard projection: read-only report rows for the
-- simulated arbitrage run.  Projection is redundant by design; the single
-- source of truth for PAPER facts remains the existing B-series tables.
CREATE TABLE IF NOT EXISTS simulation_runs (
    run_id TEXT PRIMARY KEY CHECK (run_id <> ''),
    executed_at_ms BIGINT NOT NULL CHECK (executed_at_ms > 0),
    evaluated_at_ms BIGINT NOT NULL CHECK (evaluated_at_ms >= 0),
    symbol TEXT NOT NULL CHECK (symbol <> ''),
    buy_venue TEXT NOT NULL CHECK (buy_venue <> ''),
    sell_venue TEXT NOT NULL CHECK (sell_venue <> ''),
    direction TEXT NOT NULL CHECK (direction IN ('a', 'b')),
    quantity NUMERIC NOT NULL CHECK (quantity > 0),
    scenario TEXT NOT NULL CHECK (scenario IN
        ('NORMAL', 'DEPTH_SHORTFALL', 'COMPETED_AWAY', 'REJECTED')),
    rejection_reason TEXT,
    planning_warnings JSONB NOT NULL,
    plan_id TEXT NOT NULL CHECK (plan_id <> ''),
    account_id TEXT NOT NULL CHECK (account_id <> ''),
    instrument_id TEXT NOT NULL CHECK (instrument_id <> ''),
    buy_intent_id TEXT NOT NULL CHECK (buy_intent_id <> ''),
    sell_intent_id TEXT NOT NULL CHECK (sell_intent_id <> ''),
    seed BIGINT NOT NULL,
    decision_to_submit_ms BIGINT NOT NULL,
    fill_latency_ms BIGINT NOT NULL,
    bought_quantity NUMERIC NOT NULL CHECK (bought_quantity >= 0),
    sold_quantity NUMERIC NOT NULL CHECK (sold_quantity >= 0),
    buy_avg_price NUMERIC NOT NULL CHECK (buy_avg_price >= 0),
    sell_avg_price NUMERIC NOT NULL CHECK (sell_avg_price >= 0),
    buy_fee NUMERIC NOT NULL CHECK (buy_fee >= 0),
    sell_fee NUMERIC NOT NULL CHECK (sell_fee >= 0),
    scanned_net_profit NUMERIC NOT NULL,
    simulated_net_profit NUMERIC NOT NULL,
    adverse_move_bps NUMERIC NOT NULL,
    competitor_take_bps NUMERIC NOT NULL,
    compensation_decision TEXT NOT NULL CHECK (compensation_decision IN
        ('NO_ACTION', 'COMPENSATION_PLANNED', 'MANUAL_REQUIRED')),
    execution_state TEXT NOT NULL CHECK (execution_state IN
        ('PLANNED', 'RUNNING', 'COMPENSATION_PLANNED', 'COMPLETED', 'MANUAL_REQUIRED')),
    unmatched_quantity NUMERIC NOT NULL CHECK (unmatched_quantity >= 0),
    unmatched_exposure NUMERIC NOT NULL CHECK (unmatched_exposure >= 0),
    estimated_compensation_cost NUMERIC NOT NULL CHECK (estimated_compensation_cost >= 0),
    unknown_submit_tried BOOLEAN NOT NULL DEFAULT FALSE,
    query_found BOOLEAN NOT NULL DEFAULT FALSE,
    idempotent_replay BOOLEAN NOT NULL DEFAULT FALSE,
    skipped_frames BIGINT NOT NULL DEFAULT 0,
    -- Safety boundary: simulation never makes external order calls.  The
    -- CHECK hard-codes zero so any future writer violates the schema.
    external_order_calls BIGINT NOT NULL DEFAULT 0 CHECK (external_order_calls >= 0),
    report JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS simulation_runs_clock_idx
    ON simulation_runs (executed_at_ms DESC);
CREATE INDEX IF NOT EXISTS simulation_runs_symbol_idx
    ON simulation_runs (symbol, executed_at_ms DESC);
CREATE INDEX IF NOT EXISTS simulation_runs_scenario_idx
    ON simulation_runs (scenario, executed_at_ms DESC);

DROP TRIGGER IF EXISTS simulation_runs_immutable ON simulation_runs;
CREATE TRIGGER simulation_runs_immutable
BEFORE UPDATE OR DELETE ON simulation_runs
FOR EACH ROW EXECUTE FUNCTION reject_audit_event_mutation();
