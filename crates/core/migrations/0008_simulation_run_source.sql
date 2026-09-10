-- 0008: distinguish synthetic smoke-probe runs from live market runs.
--
-- `simulation_runs` holds two very different kinds of artifact:
--   * SMOKE — `run_simulation_smoke` drives the engine over synthetic books
--     whose venue names are `s01-buy`..`s09-sell` and whose symbol label is a
--     fixture constant ("BTCUSDT"), not market data;
--   * LIVE  — the continuous observer feeds a real scanned opportunity.
--
-- Until now the only discriminator was the venue naming convention, so the
-- dashboard presented probe fixtures as if they were market simulation.  This
-- column makes the source explicit and queryable.
--
-- Backfill strategy: the default is attached as 'SMOKE' while the column is
-- created, so existing rows become probe rows without an UPDATE.  This is not
-- stylistic — `0005` installs `simulation_runs_immutable` (BEFORE UPDATE OR
-- DELETE), so an UPDATE backfill is rejected outright.  Dropping the trigger to
-- allow one would trade a permanent safety invariant for a one-off
-- convenience.  The default is then flipped to 'LIVE' so future inserts that
-- omit the column are live, matching how the engine now always binds `source`
-- explicitly.
--
-- Every pre-existing row is a smoke row: no live opportunity has ever been
-- admitted (all directions are rejected while credentials and venue
-- eligibility are unconfigured), which is also why the aggregate split in the
-- dashboard reads `live_runs = 0`.

ALTER TABLE simulation_runs
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'SMOKE'
    CHECK (source IN ('SMOKE', 'LIVE'));

ALTER TABLE simulation_runs
    ALTER COLUMN source SET DEFAULT 'LIVE';

CREATE INDEX IF NOT EXISTS simulation_runs_source_clock_idx
    ON simulation_runs (source, executed_at_ms DESC);
