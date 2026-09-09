//! Shared PostgreSQL persistence scaffolding for the B-series PAPER modules.
//!
//! Owns the schema version contract, the applied migration set, the
//! connection lifecycle, schema verification, and the small validation /
//! hashing / timestamp helpers every persistence module uses.  Domain
//! modules (paper, order, execution, accounting, reconciliation, control)
//! depend on this module for infrastructure instead of hosting generic
//! plumbing inside a B-01 domain module.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, NoTls};

use crate::market::unix_timestamp_ms;

pub(crate) const SCHEMA_VERSION: i64 = 4;
const INITIAL_PAPER_MIGRATION: &str = include_str!("../migrations/0001_paper_core.sql");
const ORDER_FACTS_MIGRATION: &str = include_str!("../migrations/0002_order_facts.sql");
const DOUBLE_LEG_EXECUTION_MIGRATION: &str =
    include_str!("../migrations/0003_double_leg_execution.sql");
const ACCOUNTING_RECONCILIATION_CONTROL_MIGRATION: &str =
    include_str!("../migrations/0004_accounting_reconciliation_control.sql");

/// Applies every B-series migration under one advisory lock, then verifies
/// the resulting schema version.  Idempotent per migration file.
pub(crate) async fn migrate(database_url: &str) -> Result<()> {
    let (client, connection) = connect(database_url).await?;
    let migration_lock = advisory_key("schema-migration", "b01-paper-core");
    client
        .query_one("SELECT pg_advisory_lock($1)", &[&migration_lock])
        .await
        .context("failed to acquire B-01 migration lock")?;
    client
        .batch_execute(INITIAL_PAPER_MIGRATION)
        .await
        .context("failed to apply B-01 database migration")?;
    client
        .batch_execute(ORDER_FACTS_MIGRATION)
        .await
        .context("failed to apply B-02 database migration")?;
    client
        .batch_execute(DOUBLE_LEG_EXECUTION_MIGRATION)
        .await
        .context("failed to apply B-03 database migration")?;
    client
        .batch_execute(ACCOUNTING_RECONCILIATION_CONTROL_MIGRATION)
        .await
        .context("failed to apply B-04 database migration")?;
    verify_schema(&client).await?;
    close_connection(client, connection).await;
    Ok(())
}

pub(crate) async fn connect(database_url: &str) -> Result<(Client, JoinHandle<()>)> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .context("failed to connect to the B-01 PostgreSQL database")?;
    let task = tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("B-01 PostgreSQL connection closed: {error}");
        }
    });
    Ok((client, task))
}

pub(crate) async fn close_connection(client: Client, connection: JoinHandle<()>) {
    drop(client);
    connection.abort();
    let _ = connection.await;
}

pub(crate) async fn verify_schema(client: &Client) -> Result<()> {
    let version: Option<i64> = client
        .query_one("SELECT MAX(version) FROM schema_migrations", &[])
        .await
        .context("B-01 schema is not initialized")?
        .get(0);
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
/// 1. Connecting to PostgreSQL
/// 2. Verifying schema
/// 3. Acquiring an advisory lock for the execution domain
/// 4. Providing disconnect on drop
pub(crate) struct DomainConnection {
    pub client: Client,
    pub connection: JoinHandle<()>,
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
        let (client, connection) = connect(database_url).await?;
        verify_schema(&client).await?;
        let lock_key = advisory_key(
            "execution-domain",
            &format!("{account_id}\0{instrument_id}"),
        );
        let acquired: bool = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&lock_key])
            .await
            .context(format!("failed to acquire {domain} single-writer lock"))?
            .get(0);
        if !acquired {
            close_connection(client, connection).await;
            bail!(
                "{domain} execution domain already has a writer: account={} instrument={}",
                account_id,
                instrument_id
            );
        }
        Ok(Self {
            client,
            connection,
            account_id,
            instrument_id,
        })
    }

    /// Disconnects from the database.
    pub async fn disconnect(self) {
        close_connection(self.client, self.connection).await;
    }
}
