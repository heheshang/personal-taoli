-- 0007: drop the pre-sqlx migration bookkeeping table.
--
-- Applied state now lives in `_sqlx_migrations`, which sqlx owns and which
-- additionally records a SHA-384 checksum per file. `0001`-era databases were
-- seeded into it once by `db::migrate_pool` (`adopt_legacy_bookkeeping`), so
-- this table is no longer read or written by anything. Keeping a frozen copy
-- would leave a second, stale version ledger next to the authoritative one —
-- exactly the parallel-truth-source shape this repository forbids.
--
-- Irreversible on purpose: the dropped rows carried only (version,
-- applied_at_ms) bookkeeping, and the timestamp was carried into
-- `_sqlx_migrations.installed_on` before the drop.

DROP TABLE IF EXISTS schema_migrations;
