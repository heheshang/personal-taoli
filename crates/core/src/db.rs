//! B-01 database access layer (DB-01: sqlx connection pool).
//!
//! One cached `PgPool` per database URL replaces the old fresh-connection-per-
//! operation path.  Two invariants come from pooling and are enforced here:
//!
//! 1. **No session advisory lock may outlive its borrower.**  A pooled backend
//!    survives release, so a leaked `pg_advisory_lock` would permanently block
//!    the next writer.  `DomainConnection::disconnect` unlocks explicitly and
//!    `after_release` unlocks again as a backstop for panic paths.
//! 2. **No transaction state may outlive its borrower.**  Read-only sessions
//!    therefore use a sqlx `Transaction` plus `SET TRANSACTION READ ONLY`
//!    (which sqlx rolls back on release) rather than a raw `BEGIN`, whose
//!    dangling state would poison the next borrower of that connection.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPoolOptions, Postgres};
use sqlx::{Executor, PgExecutor, PgPool, Row};
use tokio::sync::Mutex;

use crate::market::unix_timestamp_ms;

pub(crate) const SCHEMA_VERSION: i64 = 5;
const INITIAL_PAPER_MIGRATION: &str = include_str!("../migrations/0001_paper_core.sql");
const ORDER_FACTS_MIGRATION: &str = include_str!("../migrations/0002_order_facts.sql");
const DOUBLE_LEG_EXECUTION_MIGRATION: &str =
    include_str!("../migrations/0003_double_leg_execution.sql");
const ACCOUNTING_RECONCILIATION_CONTROL_MIGRATION: &str =
    include_str!("../migrations/0004_accounting_reconciliation_control.sql");
const SIMULATION_RUNS_MIGRATION: &str = include_str!("../migrations/0005_simulation_runs.sql");

/// Pool ceiling.  A single-writer domain holds one dedicated lock connection
/// plus the connections its transactions borrow, and the F-02 smoke holds
/// several domains at once, so the ceiling must exceed the concurrent-core
/// count without approaching PostgreSQL's own connection limit.
const MAX_CONNECTIONS: u32 = 16;
/// Fail closed rather than hang when the pool is exhausted.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Pools keyed by database URL.  `src-tauri` re-reads `TAOLI_DATABASE_URL` per
/// command and may rewrite it during setup, so one global pool would be wrong.
static POOLS: LazyLock<Mutex<HashMap<String, PgPool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns the cached pool for `database_url`, creating it on first use.
///
/// An absent or blank URL is a configuration error and fails closed.
pub(crate) async fn pool(database_url: &str) -> Result<PgPool> {
    if database_url.trim().is_empty() {
        bail!("TAOLI_DATABASE_URL must be non-empty to reach the B-01 PostgreSQL database");
    }
    // The lock is held across `connect` so two concurrent callers for the same
    // URL cannot each build a pool; pool creation is rare enough that this
    // serialization costs nothing.
    let mut pools = POOLS.lock().await;
    if let Some(existing) = pools.get(database_url) {
        return Ok(existing.clone());
    }
    let created = PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        // A released backend may still hold a session advisory lock taken by a
        // writer that panicked before `disconnect`.  Clear it before the
        // connection can be handed to another borrower.
        .after_release(|connection, _metadata| {
            Box::pin(async move {
                connection
                    .execute("SELECT pg_advisory_unlock_all()")
                    .await?;
                Ok(true)
            })
        })
        .connect(database_url)
        .await
        .context("failed to connect to the B-01 PostgreSQL database")?;
    pools.insert(database_url.to_owned(), created.clone());
    Ok(created)
}

/// Applies every B-series migration under one advisory lock, then verifies
/// the resulting schema version.  Idempotent per migration file.
pub(crate) async fn migrate(database_url: &str) -> Result<()> {
    let pool = pool(database_url).await?;
    migrate_pool(&pool).await
}

/// `migrate` against an already-resolved pool.
pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()> {
    // The whole migration set runs on one borrowed connection so the advisory
    // lock and every `raw_sql` batch share a single session.
    let mut connection = pool
        .acquire()
        .await
        .context("failed to acquire the B-01 migration connection")?;
    let migration_lock = advisory_key("schema-migration", "b01-paper-core");
    connection
        .execute(sqlx::query("SELECT pg_advisory_lock($1)").bind(migration_lock))
        .await
        .context("failed to acquire B-01 migration lock")?;
    for (label, migration) in [
        ("B-01", INITIAL_PAPER_MIGRATION),
        ("B-02", ORDER_FACTS_MIGRATION),
        ("B-03", DOUBLE_LEG_EXECUTION_MIGRATION),
        ("B-04", ACCOUNTING_RECONCILIATION_CONTROL_MIGRATION),
        ("G-01", SIMULATION_RUNS_MIGRATION),
    ] {
        connection
            .execute(sqlx::raw_sql(migration))
            .await
            .with_context(|| format!("failed to apply {label} database migration"))?;
    }
    let verified = verify_schema(&mut *connection).await;
    connection
        .execute(sqlx::query("SELECT pg_advisory_unlock($1)").bind(migration_lock))
        .await
        .context("failed to release B-01 migration lock")?;
    verified?;
    Ok(())
}

/// Rejects any schema whose applied version is not exactly `SCHEMA_VERSION`.
///
/// Generic over the executor so callers can verify through a pool, a borrowed
/// connection, or an open transaction.
pub(crate) async fn verify_schema<'e, E>(executor: E) -> Result<()>
where
    E: PgExecutor<'e>,
{
    let row = executor
        .fetch_one(sqlx::query("SELECT MAX(version) FROM schema_migrations"))
        .await
        .context("B-01 schema is not initialized")?;
    let version: Option<i64> = row.try_get(0).context("B-01 schema is not initialized")?;
    if version != Some(SCHEMA_VERSION) {
        bail!(
            "unsupported B-01 schema version: expected {}, found {:?}",
            SCHEMA_VERSION,
            version
        );
    }
    Ok(())
}

pub(crate) fn validate_id(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value != value.trim() || value.contains('\0') {
        bail!("{name} must be non-empty, trimmed and contain no NUL byte");
    }
    Ok(())
}

/// Deterministic 64-bit advisory key from a namespace and value; distinct
/// namespaces never collide because of the embedded separator.
pub(crate) fn advisory_key(namespace: &str, value: &str) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    i64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"))
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(crate) fn db_time() -> Result<i64> {
    to_i64(unix_timestamp_ms()?)
}

pub(crate) fn to_i64(value: u64) -> Result<i64> {
    value
        .try_into()
        .context("timestamp exceeds PostgreSQL BIGINT")
}

pub(crate) fn to_u64(value: i64) -> Result<u64> {
    value.try_into().context("database timestamp is negative")
}

/// Database connection with advisory lock for single-writer domains.
///
/// Encapsulates the common pattern of:
/// 1. Resolving the pool for the database URL
/// 2. Verifying schema
/// 3. Acquiring an advisory lock for the execution domain on a dedicated
///    connection, which keeps the session lock alive for the whole domain
/// 4. Releasing that lock and returning the connection to the pool
///
/// Queries go through `pool`, which is an executor behind a shared reference,
/// so read methods keep their `&self` receivers.
pub(crate) struct DomainConnection {
    pub pool: PgPool,
    /// Session that owns the domain advisory lock.  Never used for queries.
    lock_connection: PoolConnection<Postgres>,
    pub account_id: String,
    pub instrument_id: String,
}

impl DomainConnection {
    /// Acquires a domain lock for the given account and instrument.
    ///
    /// This is the shared implementation used by PaperCore, OrderCore, and ExecutionCore.
    pub async fn acquire(
        database_url: &str,
        account_id: impl Into<String>,
        instrument_id: impl Into<String>,
        domain: &str,
    ) -> Result<Self> {
        let account_id = account_id.into();
        let instrument_id = instrument_id.into();
        validate_id("account_id", &account_id)?;
        validate_id("instrument_id", &instrument_id)?;
        let pool = pool(database_url).await?;
        verify_schema(&pool).await?;
        let lock_key = advisory_key(
            "execution-domain",
            &format!("{account_id}\0{instrument_id}"),
        );
        let mut lock_connection = pool
            .acquire()
            .await
            .context(format!("failed to acquire {domain} single-writer lock"))?;
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *lock_connection)
            .await
            .context(format!("failed to acquire {domain} single-writer lock"))?;
        if !acquired {
            // `try` failed, so this session holds nothing; releasing it is safe.
            drop(lock_connection);
            bail!(
                "{domain} execution domain already has a writer: account={} instrument={}",
                account_id,
                instrument_id
            );
        }
        Ok(Self {
            pool,
            lock_connection,
            account_id,
            instrument_id,
        })
    }

    /// Releases the domain lock and returns the lock session to the pool.
    pub async fn disconnect(mut self) {
        // Explicit unlock first: `after_release` is only a backstop, and a
        // leaked session lock would block every later writer of this domain.
        let _ = sqlx::query("SELECT pg_advisory_unlock_all()")
            .execute(&mut *self.lock_connection)
            .await;
        drop(self.lock_connection);
    }
}
