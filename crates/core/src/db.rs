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

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPoolOptions, Postgres};
use sqlx::{Executor, PgPool};
use tokio::sync::Mutex;

use crate::market::unix_timestamp_ms;

/// Every B-series migration, embedded from `migrations/` at compile time.
///
/// sqlx parses `<VERSION>_<DESCRIPTION>.sql`, orders by version, and records a
/// SHA-384 of each file in `_sqlx_migrations`; editing an already-applied file
/// is therefore a hard error instead of a silent no-op.  Adding a migration is
/// one new file — there is no list to keep in sync.
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Highest embedded migration version.
///
/// Derived from the file set rather than hand-maintained, so the reported
/// `schema_version` cannot drift from the migrations that were applied.
pub(crate) fn schema_version() -> i64 {
    MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

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

/// Database URLs whose migrations and schema check already succeeded in this
/// process.  Migrations are idempotent but expensive (six DDL batches behind
/// a session advisory lock) while the dashboard polls every 10 s, so the work
/// must happen once per process rather than once per call.
static READY: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

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
/// the resulting schema version.  Idempotent per migration file, and memoized
/// per process: the first call for a URL does the work, later calls are a hit.
pub(crate) async fn migrate(database_url: &str) -> Result<()> {
    ready_pool(database_url).await.map(|_| ())
}

/// Pool whose migrations have been applied.
///
/// The single entry point for callers that need "the schema is usable".  The
/// work is memoized per process because the dashboard polls every 10 s and the
/// migrator would otherwise re-read `_sqlx_migrations` and re-check every
/// checksum on each poll.
pub(crate) async fn ready_pool(database_url: &str) -> Result<PgPool> {
    let pool = pool(database_url).await?;
    if READY.lock().await.contains(database_url) {
        return Ok(pool);
    }
    // Two concurrent first callers can both pass the check above.  That is
    // safe rather than merely tolerated: seeding uses `ON CONFLICT DO NOTHING`
    // and `MIGRATOR.run` serializes on sqlx's database-wide advisory lock, so
    // the loser observes every version already applied and does nothing.
    migrate_pool(&pool).await?;
    READY.lock().await.insert(database_url.to_owned());
    Ok(pool)
}

/// Applies pending migrations through sqlx's migrator.
///
/// Applied state lives in `_sqlx_migrations` (version, description,
/// `installed_on`, `success`, SHA-384 `checksum`, `execution_time`); sqlx holds
/// its own database-wide advisory lock, wraps each file plus its bookkeeping in
/// one transaction, and refuses to proceed from a half-applied (`success =
/// false`) migration.
pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()> {
    adopt_legacy_bookkeeping(pool).await?;
    MIGRATOR
        .run(pool)
        .await
        .context("failed to apply database migrations")?;
    Ok(())
}

/// Seeds `_sqlx_migrations` from the pre-sqlx `schema_migrations` table.
///
/// Databases created before sqlx owned this file set record their applied
/// versions in `schema_migrations`, which `0007` drops.  Without this one-time
/// adoption the migrator would see an empty `_sqlx_migrations` and re-run all
/// files against a live database.  Runs only while `_sqlx_migrations` is empty.
///
/// The `CREATE TABLE` mirrors the migrator's own `ensure_migrations_table`, so
/// the seeded rows land in exactly the table `MIGRATOR.run` reads next.  A
/// mismatch would surface immediately as a migrator error rather than as
/// silently re-applied migrations.
async fn adopt_legacy_bookkeeping(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS _sqlx_migrations (
             version BIGINT PRIMARY KEY,
             description TEXT NOT NULL,
             installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
             success BOOLEAN NOT NULL,
             checksum BYTEA NOT NULL,
             execution_time BIGINT NOT NULL
         )",
    )
    .execute(pool)
    .await
    .context("failed to prepare the sqlx migration table")?;

    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .context("failed to inspect the sqlx migration table")?;
    if applied > 0 {
        return Ok(());
    }

    let legacy_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public.schema_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await
            .context("failed to inspect the legacy migration table")?;
    if !legacy_exists {
        return Ok(());
    }

    // Refuse a database carrying a migration this binary does not know, the
    // same protection the removed `verify_schema` gave: an older binary must
    // not migrate a newer schema.
    let legacy_versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .context("failed to read the legacy migration table")?;
    for version in &legacy_versions {
        if !MIGRATOR.version_exists(*version) {
            bail!(
                "unsupported B-01 schema version: expected at most {}, found {}",
                schema_version(),
                version
            );
        }
    }

    for migration in MIGRATOR.iter() {
        let recorded_at_ms: Option<i64> =
            sqlx::query_scalar("SELECT applied_at_ms FROM schema_migrations WHERE version = $1")
                .bind(migration.version)
                .fetch_optional(pool)
                .await
                .with_context(|| {
                    format!("failed to read legacy migration {}", migration.version)
                })?;
        let Some(applied_at_ms) = recorded_at_ms else {
            continue;
        };
        // `installed_on` keeps the original application time so dropping the
        // legacy table loses no history; `execution_time` is unknown for an
        // adopted row, which is the value sqlx itself uses before measuring.
        sqlx::query(
            "INSERT INTO _sqlx_migrations
                 (version, description, installed_on, success, checksum, execution_time)
             VALUES ($1, $2, to_timestamp($3::double precision / 1000.0), TRUE, $4, -1)
             ON CONFLICT (version) DO NOTHING",
        )
        .bind(migration.version)
        .bind(&*migration.description)
        .bind(applied_at_ms as f64)
        .bind(&*migration.checksum)
        .execute(pool)
        .await
        .with_context(|| format!("failed to adopt legacy migration {}", migration.version))?;
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
        // `ready_pool` rather than `pool`: acquiring a domain also guarantees
        // the schema is migrated, so a writer can never start against an
        // unmigrated database.
        let pool = ready_pool(database_url).await?;
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
