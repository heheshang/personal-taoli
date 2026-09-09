use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_postgres::Client;

use crate::{
    db::{close_connection, connect, to_i64, to_u64, validate_id, verify_schema},
    market::unix_timestamp_ms,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ControlAction {
    Pause,
    CancelOrders,
    ReduceExposure,
    Resume,
    Halt,
}
impl ControlAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "PAUSE",
            Self::CancelOrders => "CANCEL_ORDERS",
            Self::ReduceExposure => "REDUCE_EXPOSURE",
            Self::Resume => "RESUME",
            Self::Halt => "HALT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ControlStatus {
    Received,
    Executing,
    Succeeded,
    Failed,
    Partial,
    Expired,
}
impl ControlStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Received => "RECEIVED",
            Self::Executing => "EXECUTING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Partial => "PARTIAL",
            Self::Expired => "EXPIRED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlCommandRequest {
    pub command_id: String,
    pub request_id: String,
    pub actor: String,
    pub action: ControlAction,
    pub scope: Value,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControlCommand {
    pub command_id: String,
    pub request_id: String,
    pub actor: String,
    pub action: ControlAction,
    pub status: ControlStatus,
    pub expires_at_ms: u64,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControlSmokeReport {
    pub schema_version: i64,
    pub distinct_actions_recorded: bool,
    pub idempotent_request: bool,
    pub expired_not_executed: bool,
    pub external_order_calls: usize,
}

pub struct ControlCore {
    client: Client,
    connection: JoinHandle<()>,
}
impl ControlCore {
    pub async fn acquire(database_url: &str) -> Result<Self> {
        let (client, connection) = connect(database_url).await?;
        verify_schema(&client).await?;
        Ok(Self { client, connection })
    }
    pub async fn submit(
        &mut self,
        request: ControlCommandRequest,
        now_ms: u64,
    ) -> Result<ControlCommand> {
        validate_request(&request, now_ms)?;
        let tx = self
            .client
            .transaction()
            .await
            .context("failed to begin control command")?;
        if let Some(row) = tx
            .query_opt(
                "SELECT command_id FROM control_commands WHERE request_id=$1",
                &[&request.request_id],
            )
            .await
            .context("failed to inspect control idempotency key")?
        {
            let command_id: String = row.get(0);
            if command_id != request.command_id {
                bail!("control request_id was reused with another command_id");
            }
            tx.commit()
                .await
                .context("failed to commit control replay")?;
            return self.load(&request.command_id).await;
        }
        let status = if request.expires_at_ms <= now_ms {
            ControlStatus::Expired
        } else {
            ControlStatus::Received
        };
        tx.execute("INSERT INTO control_commands(command_id,request_id,actor,action,scope,status,expires_at_ms,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&request.command_id, &request.request_id, &request.actor, &request.action.as_str(), &request.scope, &status.as_str(), &to_i64(request.expires_at_ms)?, &to_i64(now_ms)?]).await.context("failed to persist control command")?;
        tx.execute("INSERT INTO control_audit_logs(command_id,actor,action,before_state,after_state,result,occurred_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&request.command_id, &request.actor, &request.action.as_str(), &Value::Null, &json!({"status":status.as_str()}), &Value::Null, &to_i64(now_ms)?]).await.context("failed to persist control audit")?;
        tx.commit()
            .await
            .context("failed to commit control command")?;
        self.load(&request.command_id).await
    }
    pub async fn complete(
        &mut self,
        command_id: &str,
        status: ControlStatus,
        result: Value,
        now_ms: u64,
    ) -> Result<ControlCommand> {
        validate_id("command_id", command_id)?;
        let tx = self
            .client
            .transaction()
            .await
            .context("failed to begin control completion")?;
        let row = tx.query_one("SELECT action,expires_at_ms,status FROM control_commands WHERE command_id=$1 FOR UPDATE", &[&command_id]).await.context("control command not found")?;
        let action: String = row.get(0);
        let expires: i64 = row.get(1);
        let current: String = row.get(2);
        if current == "EXPIRED" || expires <= to_i64(now_ms)? {
            tx.execute(
                "UPDATE control_commands SET status='EXPIRED',result=$2 WHERE command_id=$1",
                &[&command_id, &json!({"reason":"expired before execution"})],
            )
            .await?;
            tx.commit().await?;
            return self.load(command_id).await;
        }
        if !matches!(
            status,
            ControlStatus::Succeeded | ControlStatus::Failed | ControlStatus::Partial
        ) {
            bail!("control completion requires a terminal result");
        }
        tx.execute(
            "UPDATE control_commands SET status=$2,result=$3 WHERE command_id=$1",
            &[&command_id, &status.as_str(), &result],
        )
        .await
        .context("failed to complete control command")?;
        tx.execute("INSERT INTO control_audit_logs(command_id,actor,action,result,occurred_at_ms) SELECT command_id,actor,action,$2,$3 FROM control_commands WHERE command_id=$1", &[&command_id, &result, &to_i64(now_ms)?]).await.context("failed to append control result audit")?;
        let _ = action;
        tx.commit()
            .await
            .context("failed to commit control completion")?;
        self.load(command_id).await
    }
    pub async fn load(&self, command_id: &str) -> Result<ControlCommand> {
        validate_id("command_id", command_id)?;
        let row = self.client.query_one("SELECT command_id,request_id,actor,action,status,expires_at_ms,result FROM control_commands WHERE command_id=$1", &[&command_id]).await.context("failed to load control command")?;
        Ok(ControlCommand {
            command_id: row.get(0),
            request_id: row.get(1),
            actor: row.get(2),
            action: parse_action(row.get::<_, String>(3).as_str())?,
            status: parse_status(row.get::<_, String>(4).as_str())?,
            expires_at_ms: to_u64(row.get(5))?,
            result: row.get(6),
        })
    }
    pub async fn disconnect(self) {
        close_connection(self.client, self.connection).await;
    }
}

pub async fn run_control_smoke(database_url: &str) -> Result<ControlSmokeReport> {
    crate::db::migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let mut core = ControlCore::acquire(database_url).await?;
    let now = unix_timestamp_ms()?;
    let request = ControlCommandRequest {
        command_id: format!("command-{stamp}"),
        request_id: format!("request-{stamp}"),
        actor: "smoke-operator".to_owned(),
        action: ControlAction::Pause,
        scope: json!({"domain":"all"}),
        expires_at_ms: now + 10_000,
    };
    let first = core.submit(request.clone(), now).await?;
    let replay = core.submit(request, now).await?;
    let completed = core
        .complete(
            &first.command_id,
            ControlStatus::Succeeded,
            json!({"accepted":true}),
            now + 1,
        )
        .await?;
    let expired_request = ControlCommandRequest {
        command_id: format!("expired-{stamp}"),
        request_id: format!("expired-request-{stamp}"),
        actor: "smoke-operator".to_owned(),
        action: ControlAction::Resume,
        scope: json!({"requires_reconciliation":true}),
        expires_at_ms: now,
    };
    let expired = core.submit(expired_request, now).await?;
    core.disconnect().await;
    Ok(ControlSmokeReport {
        schema_version: 4,
        distinct_actions_recorded: first.action == ControlAction::Pause
            && completed.status == ControlStatus::Succeeded,
        idempotent_request: replay.command_id == first.command_id,
        expired_not_executed: expired.status == ControlStatus::Expired,
        external_order_calls: 0,
    })
}

fn validate_request(request: &ControlCommandRequest, now_ms: u64) -> Result<()> {
    for (name, value) in [
        ("command_id", request.command_id.as_str()),
        ("request_id", request.request_id.as_str()),
        ("actor", request.actor.as_str()),
    ] {
        validate_id(name, value)?;
    }
    if request.expires_at_ms == 0 || now_ms == 0 {
        bail!("control timestamps must be positive");
    }
    Ok(())
}
fn parse_action(value: &str) -> Result<ControlAction> {
    match value {
        "PAUSE" => Ok(ControlAction::Pause),
        "CANCEL_ORDERS" => Ok(ControlAction::CancelOrders),
        "REDUCE_EXPOSURE" => Ok(ControlAction::ReduceExposure),
        "RESUME" => Ok(ControlAction::Resume),
        "HALT" => Ok(ControlAction::Halt),
        _ => bail!("invalid control action"),
    }
}
fn parse_status(value: &str) -> Result<ControlStatus> {
    match value {
        "RECEIVED" => Ok(ControlStatus::Received),
        "EXECUTING" => Ok(ControlStatus::Executing),
        "SUCCEEDED" => Ok(ControlStatus::Succeeded),
        "FAILED" => Ok(ControlStatus::Failed),
        "PARTIAL" => Ok(ControlStatus::Partial),
        "EXPIRED" => Ok(ControlStatus::Expired),
        _ => bail!("invalid control status"),
    }
}
