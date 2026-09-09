use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::task::JoinHandle;
use tokio_postgres::Client;

use crate::{
    market::unix_timestamp_ms,
    paper::{close_connection, connect, validate_id, verify_schema},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DifferenceCategory {
    Delayed,
    Missing,
    Conflict,
    ExplainedAdjustment,
}
impl DifferenceCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delayed => "DELAYED",
            Self::Missing => "MISSING",
            Self::Conflict => "CONFLICT",
            Self::ExplainedAdjustment => "EXPLAINED_ADJUSTMENT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceInput {
    pub snapshot_id: String,
    pub account_id: String,
    pub venue: String,
    pub asset: String,
    pub total: Decimal,
    pub free: Decimal,
    pub locked: Decimal,
    pub observed_at_ms: u64,
    pub source: String,
    pub completeness: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationDifference {
    pub difference_id: String,
    pub asset: String,
    pub category: DifferenceCategory,
    pub local_amount: Decimal,
    pub external_amount: Decimal,
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationReport {
    pub run_id: String,
    pub status: String,
    pub differences: Vec<ReconciliationDifference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationSmokeReport {
    pub schema_version: i64,
    pub matched_snapshot: bool,
    pub missing_difference_classified: bool,
    pub conflict_difference_classified: bool,
    pub external_order_calls: usize,
}

pub struct ReconciliationCore {
    client: Client,
    connection: JoinHandle<()>,
}
impl ReconciliationCore {
    pub async fn acquire(database_url: &str) -> Result<Self> {
        let (client, connection) = connect(database_url).await?;
        verify_schema(&client).await?;
        Ok(Self { client, connection })
    }
    pub async fn record_snapshot(&mut self, snapshot: BalanceInput) -> Result<()> {
        validate_snapshot(&snapshot)?;
        self.client.execute("INSERT INTO balance_snapshots(snapshot_id,account_id,venue,asset,total,free,locked,observed_at_ms,source,completeness) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(snapshot_id) DO NOTHING", &[&snapshot.snapshot_id, &snapshot.account_id, &snapshot.venue, &snapshot.asset, &snapshot.total, &snapshot.free, &snapshot.locked, &to_i64(snapshot.observed_at_ms)?, &snapshot.source, &snapshot.completeness]).await.context("failed to persist balance snapshot")?;
        Ok(())
    }
    pub async fn reconcile(
        &mut self,
        run_id: &str,
        account_id: &str,
        venue: &str,
        local: &BTreeMap<String, Decimal>,
        external: &BTreeMap<String, Decimal>,
        now_ms: u64,
    ) -> Result<ReconciliationReport> {
        validate_id("run_id", run_id)?;
        validate_id("account_id", account_id)?;
        validate_id("venue", venue)?;
        if now_ms == 0 {
            bail!("reconciliation timestamp must be positive");
        }
        let tx = self
            .client
            .transaction()
            .await
            .context("failed to begin reconciliation")?;
        tx.execute("INSERT INTO reconciliation_runs(run_id,account_id,venue,status,started_at_ms) VALUES($1,$2,$3,'RUNNING',$4)", &[&run_id, &account_id, &venue, &to_i64(now_ms)?]).await.context("failed to create reconciliation run")?;
        let mut assets = local
            .keys()
            .chain(external.keys())
            .cloned()
            .collect::<Vec<_>>();
        assets.sort();
        assets.dedup();
        let mut differences = Vec::new();
        for asset in assets {
            let local_amount = local.get(&asset).copied().unwrap_or(Decimal::ZERO);
            let external_amount = external.get(&asset).copied().unwrap_or(Decimal::ZERO);
            if local_amount == external_amount {
                continue;
            }
            let category = if !local.contains_key(&asset) || !external.contains_key(&asset) {
                DifferenceCategory::Missing
            } else if local_amount.abs() < external_amount.abs() + Decimal::new(1, 8)
                && local_amount.abs() > external_amount.abs() - Decimal::new(1, 8)
            {
                DifferenceCategory::Delayed
            } else {
                DifferenceCategory::Conflict
            };
            let difference_id = format!("{run_id}:{asset}");
            tx.execute("INSERT INTO reconciliation_differences(difference_id,run_id,asset,category,local_amount,external_amount,explanation,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&difference_id, &run_id, &asset, &category.as_str(), &local_amount, &external_amount, &Option::<String>::None, &to_i64(now_ms)?]).await.context("failed to persist reconciliation difference")?;
            differences.push(ReconciliationDifference {
                difference_id,
                asset,
                category,
                local_amount,
                external_amount,
                explanation: None,
            });
        }
        let status = if differences.is_empty() {
            "MATCHED"
        } else {
            "ATTENTION_REQUIRED"
        };
        tx.execute(
            "UPDATE reconciliation_runs SET status=$2,finished_at_ms=$3 WHERE run_id=$1",
            &[&run_id, &status, &to_i64(now_ms)?],
        )
        .await
        .context("failed to finish reconciliation")?;
        tx.commit()
            .await
            .context("failed to commit reconciliation")?;
        Ok(ReconciliationReport {
            run_id: run_id.to_owned(),
            status: status.to_owned(),
            differences,
        })
    }
    pub async fn disconnect(self) {
        close_connection(self.client, self.connection).await;
    }
}

pub async fn run_reconciliation_smoke(database_url: &str) -> Result<ReconciliationSmokeReport> {
    crate::paper::migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let mut core = ReconciliationCore::acquire(database_url).await?;
    let account = format!("recon-account-{stamp}");
    let venue = "paper-a".to_owned();
    let now = unix_timestamp_ms()?;
    core.record_snapshot(BalanceInput {
        snapshot_id: format!("snapshot-{stamp}"),
        account_id: account.clone(),
        venue: venue.clone(),
        asset: "BTC".to_owned(),
        total: Decimal::ONE,
        free: Decimal::new(9, 1),
        locked: Decimal::new(1, 1),
        observed_at_ms: now,
        source: "smoke".to_owned(),
        completeness: "COMPLETE".to_owned(),
    })
    .await?;
    let mut matched = BTreeMap::new();
    matched.insert("BTC".to_owned(), Decimal::ONE);
    let report = core
        .reconcile(
            &format!("run-matched-{stamp}"),
            &account,
            &venue,
            &matched,
            &matched,
            now + 1,
        )
        .await?;
    let mut external = matched.clone();
    external.insert("USDT".to_owned(), Decimal::ONE);
    let missing = core
        .reconcile(
            &format!("run-missing-{stamp}"),
            &account,
            &venue,
            &matched,
            &external,
            now + 2,
        )
        .await?;
    let mut conflict_external = BTreeMap::new();
    conflict_external.insert("BTC".to_owned(), Decimal::new(2, 0));
    let conflict = core
        .reconcile(
            &format!("run-conflict-{stamp}"),
            &account,
            &venue,
            &matched,
            &conflict_external,
            now + 3,
        )
        .await?;
    core.disconnect().await;
    Ok(ReconciliationSmokeReport {
        schema_version: 4,
        matched_snapshot: report.status == "MATCHED",
        missing_difference_classified: missing
            .differences
            .iter()
            .any(|d| d.category == DifferenceCategory::Missing),
        conflict_difference_classified: conflict
            .differences
            .iter()
            .any(|d| d.category == DifferenceCategory::Conflict),
        external_order_calls: 0,
    })
}

fn validate_snapshot(snapshot: &BalanceInput) -> Result<()> {
    for (name, value) in [
        ("snapshot_id", snapshot.snapshot_id.as_str()),
        ("account_id", snapshot.account_id.as_str()),
        ("venue", snapshot.venue.as_str()),
        ("asset", snapshot.asset.as_str()),
        ("source", snapshot.source.as_str()),
    ] {
        validate_id(name, value)?;
    }
    if snapshot.observed_at_ms == 0
        || snapshot.total < Decimal::ZERO
        || snapshot.free < Decimal::ZERO
        || snapshot.locked < Decimal::ZERO
        || snapshot.free + snapshot.locked > snapshot.total
        || !matches!(
            snapshot.completeness.as_str(),
            "COMPLETE" | "PARTIAL" | "STALE"
        )
    {
        bail!("invalid balance snapshot");
    }
    Ok(())
}
fn to_i64(value: u64) -> Result<i64> {
    value
        .try_into()
        .context("timestamp exceeds PostgreSQL BIGINT")
}
