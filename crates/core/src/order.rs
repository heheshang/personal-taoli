use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_postgres::Row;

use crate::db::{DomainConnection, db_time, hex_digest, migrate, to_i64, validate_id};
use crate::market::unix_timestamp_ms;
use crate::paper::{
    OrderIntentInput, OrderSide, ReservationInput, ReservePlanRequest, set_paper_balance,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubmissionStatus {
    NotSent,
    InFlight,
    Accepted,
    DefinitelyRejected,
    Unknown,
}

impl SubmissionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotSent => "NOT_SENT",
            Self::InFlight => "IN_FLIGHT",
            Self::Accepted => "ACCEPTED",
            Self::DefinitelyRejected => "DEFINITELY_REJECTED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CancelStatus {
    None,
    Requested,
    Unknown,
    Confirmed,
}

impl CancelStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Requested => "REQUESTED",
            Self::Unknown => "UNKNOWN",
            Self::Confirmed => "CONFIRMED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReconciliationStatus {
    Pending,
    Matched,
    Conflict,
}

impl ReconciliationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Matched => "MATCHED",
            Self::Conflict => "CONFLICT",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderFactSnapshot {
    pub intent_id: String,
    pub account_id: String,
    pub instrument_id: String,
    pub venue: String,
    pub ordered_quantity: Decimal,
    pub submission_status: SubmissionStatus,
    pub cancel_status: CancelStatus,
    pub reconciliation_status: ReconciliationStatus,
    pub exchange_order_id: Option<String>,
    pub filled_quantity: Decimal,
    pub last_fact_version: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SubmitResult {
    Accepted { exchange_order_id: String },
    DefinitelyRejected { reason: String },
    Unknown { reason: String },
}

#[derive(Debug, Clone)]
pub enum QueryResult {
    NotFound {
        query_id: String,
        visibility_deadline_ms: u64,
    },
    Found {
        query_id: String,
        exchange_order_id: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum CancelResult {
    Unknown,
    Confirmed,
}

#[derive(Debug, Clone)]
pub struct TradeInput {
    pub venue: String,
    pub exchange_order_id: String,
    pub trade_id: String,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee_asset: String,
    pub fee_amount: Decimal,
    pub source_sequence: u64,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderFactsSmokeReport {
    pub schema_version: i64,
    pub submit_definitely_rejected: bool,
    pub submit_timeout_unknown: bool,
    pub query_not_found_preserved_unknown: bool,
    pub query_found_recovered: bool,
    pub duplicate_trade_ignored: bool,
    pub cancel_race_trade_preserved: bool,
    pub recovered_filled_quantity: Decimal,
    pub recovered_submission_status: SubmissionStatus,
    pub recovered_cancel_status: CancelStatus,
    pub external_order_calls: usize,
}

pub struct OrderCore {
    inner: DomainConnection,
}

impl OrderCore {
    pub async fn acquire(
        database_url: &str,
        account_id: impl Into<String>,
        instrument_id: impl Into<String>,
    ) -> Result<Self> {
        let inner =
            DomainConnection::acquire(database_url, account_id, instrument_id, "order").await?;
        Ok(Self { inner })
    }

    pub async fn ensure_intent(&mut self, intent_id: &str) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        validate_id("intent_id", &intent_id)?;
        let now = db_time()?;
        self.inner
            .client
            .execute(
                "INSERT INTO order_facts(intent_id,account_id,instrument_id,venue,ordered_quantity,submission_status,cancel_status,reconciliation_status,updated_at_ms) SELECT oi.intent_id,ep.account_id,ep.instrument_id,oi.venue,oi.quantity,'NOT_SENT','NONE','PENDING',$2 FROM order_intents oi JOIN execution_plans ep ON ep.plan_id=oi.plan_id WHERE oi.intent_id=$1 AND ep.account_id=$3 AND ep.instrument_id=$4 ON CONFLICT(intent_id) DO NOTHING",
                &[&intent_id, &now, &self.inner.account_id, &self.inner.instrument_id],
            )
            .await
            .context("failed to materialize B-02 order fact")?;
        self.load(&intent_id).await
    }

    pub async fn submit_started(
        &mut self,
        intent_id: &str,
        request_digest: &str,
    ) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        validate_id("intent_id", &intent_id)?;
        validate_digest(request_digest)?;
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin submit-start transaction")?;
        let current = lock_fact(&tx, &intent_id).await?;
        if current.submission_status != SubmissionStatus::NotSent {
            bail!("submit can only start from NOT_SENT");
        }
        let version = current.last_fact_version + 1;
        let now = db_time()?;
        tx.execute(
            "UPDATE order_facts SET submission_status='IN_FLIGHT',last_fact_version=$2,updated_at_ms=$3 WHERE intent_id=$1",
            &[&intent_id, &version, &now],
        ).await.context("failed to mark order IN_FLIGHT")?;
        append_action(
            &tx,
            &intent_id,
            version,
            "SUBMIT_STARTED",
            Some(request_digest),
            now,
            json!({"request_digest": request_digest}),
        )
        .await?;
        tx.commit()
            .await
            .context("failed to commit submit-start fact")?;
        self.load(&intent_id).await
    }
    pub async fn submit_result(
        &mut self,
        intent_id: &str,
        result: SubmitResult,
    ) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin submit-result transaction")?;
        let current = lock_fact(&tx, &intent_id).await?;
        if current.submission_status != SubmissionStatus::InFlight {
            bail!("submit result requires IN_FLIGHT");
        }
        let (status, action, payload, order_id, error) = match result {
            SubmitResult::Accepted { exchange_order_id } => {
                validate_id("exchange_order_id", &exchange_order_id)?;
                (
                    SubmissionStatus::Accepted,
                    "SUBMIT_ACCEPTED",
                    json!({"exchange_order_id": exchange_order_id}),
                    Some(exchange_order_id),
                    None,
                )
            }
            SubmitResult::DefinitelyRejected { reason } => {
                validate_id("rejection reason", &reason)?;
                (
                    SubmissionStatus::DefinitelyRejected,
                    "SUBMIT_REJECTED",
                    json!({"reason": reason}),
                    None,
                    Some(reason),
                )
            }
            SubmitResult::Unknown { reason } => {
                validate_id("unknown reason", &reason)?;
                (
                    SubmissionStatus::Unknown,
                    "SUBMIT_UNKNOWN",
                    json!({"reason": reason}),
                    None,
                    Some(reason),
                )
            }
        };
        let version = current.last_fact_version + 1;
        let now = db_time()?;
        tx.execute(
            "UPDATE order_facts SET submission_status=$2,exchange_order_id=COALESCE($3,exchange_order_id),last_error=$4,last_fact_version=$5,updated_at_ms=$6 WHERE intent_id=$1",
            &[&intent_id, &status.as_str(), &order_id, &error, &version, &now],
        ).await.context("failed to persist submit result")?;
        append_action(&tx, &intent_id, version, action, None, now, payload).await?;
        tx.commit()
            .await
            .context("failed to commit submit result")?;
        self.load(&intent_id).await
    }

    pub async fn query_result(
        &mut self,
        intent_id: &str,
        result: QueryResult,
    ) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin order investigation")?;
        let current = lock_fact(&tx, &intent_id).await?;
        if current.submission_status != SubmissionStatus::Unknown {
            bail!("order query requires UNKNOWN");
        }
        let (action, outcome, order_id, query_id, deadline) = match result {
            QueryResult::NotFound {
                query_id,
                visibility_deadline_ms,
            } => (
                "QUERY_NOT_FOUND",
                "NOT_FOUND",
                None,
                query_id,
                visibility_deadline_ms,
            ),
            QueryResult::Found {
                query_id,
                exchange_order_id,
            } => {
                validate_id("exchange_order_id", &exchange_order_id)?;
                ("QUERY_FOUND", "FOUND", Some(exchange_order_id), query_id, 0)
            }
        };
        validate_id("query_id", &query_id)?;
        let now = db_time()?;
        tx.execute(
            "INSERT INTO order_investigations(query_id,intent_id,outcome,visibility_deadline_ms,occurred_at_ms,payload) VALUES($1,$2,$3,$4,$5,$6)",
            &[
                &query_id,
                &intent_id,
                &outcome,
                &to_i64(deadline)?,
                &now,
                &json!({"outcome": outcome}),
            ],
        ).await.context("failed to persist order investigation")?;
        let version = current.last_fact_version + 1;
        let (status, payload) = if let Some(exchange_order_id) = &order_id {
            (
                SubmissionStatus::Accepted,
                json!({"exchange_order_id": exchange_order_id}),
            )
        } else {
            (
                SubmissionStatus::Unknown,
                json!({"query_id": query_id, "visibility_deadline_ms": deadline}),
            )
        };
        tx.execute(
            "UPDATE order_facts SET submission_status=$2,exchange_order_id=COALESCE($3,exchange_order_id),last_fact_version=$4,updated_at_ms=$5 WHERE intent_id=$1",
            &[&intent_id, &status.as_str(), &order_id, &version, &now],
        ).await.context("failed to persist query result")?;
        append_action(&tx, &intent_id, version, action, None, now, payload).await?;
        tx.commit()
            .await
            .context("failed to commit order investigation")?;
        self.load(&intent_id).await
    }

    pub async fn cancel_requested(&mut self, intent_id: &str) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        validate_id("intent_id", &intent_id)?;
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin cancel-request transaction")?;
        let current = lock_fact(&tx, &intent_id).await?;
        if current.submission_status != SubmissionStatus::Accepted
            || !matches!(
                current.cancel_status,
                CancelStatus::None | CancelStatus::Unknown
            )
        {
            bail!("cancel request requires an accepted order without confirmed cancellation");
        }
        let version = current.last_fact_version + 1;
        let now = db_time()?;
        tx.execute(
            "UPDATE order_facts SET cancel_status='REQUESTED',last_fact_version=$2,updated_at_ms=$3 WHERE intent_id=$1",
            &[&intent_id, &version, &now],
        )
        .await
        .context("failed to persist cancel request")?;
        append_action(
            &tx,
            &intent_id,
            version,
            "CANCEL_REQUESTED",
            None,
            now,
            json!({"cancel_status": "REQUESTED"}),
        )
        .await?;
        tx.commit()
            .await
            .context("failed to commit cancel request")?;
        self.load(&intent_id).await
    }

    pub async fn cancel_result(
        &mut self,
        intent_id: &str,
        result: CancelResult,
    ) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin cancel fact")?;
        let current = lock_fact(&tx, &intent_id).await?;
        if current.submission_status != SubmissionStatus::Accepted
            || !matches!(
                current.cancel_status,
                CancelStatus::Requested | CancelStatus::Unknown
            )
        {
            bail!("cancel result requires a requested cancellation");
        }
        let (status, action) = match result {
            CancelResult::Unknown => (CancelStatus::Unknown, "CANCEL_UNKNOWN"),
            CancelResult::Confirmed => (CancelStatus::Confirmed, "CANCEL_CONFIRMED"),
        };
        let version = current.last_fact_version + 1;
        let now = db_time()?;
        tx.execute(
            "UPDATE order_facts SET cancel_status=$2,last_fact_version=$3,updated_at_ms=$4 WHERE intent_id=$1",
            &[&intent_id, &status.as_str(), &version, &now],
        ).await.context("failed to persist cancel fact")?;
        append_action(
            &tx,
            &intent_id,
            version,
            action,
            None,
            now,
            json!({"cancel_status": status.as_str()}),
        )
        .await?;
        tx.commit().await.context("failed to commit cancel fact")?;
        self.load(&intent_id).await
    }

    pub async fn record_trade(
        &mut self,
        intent_id: &str,
        trade: TradeInput,
    ) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        validate_trade(&trade)?;
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin trade fact")?;
        let current = lock_fact(&tx, &intent_id).await?;
        if current.exchange_order_id.as_deref() != Some(trade.exchange_order_id.as_str())
            || current.venue != trade.venue
        {
            bail!("trade does not match the durable order identity");
        }
        let digest = trade_digest(&trade)?;
        let existing = tx.query_opt(
            "SELECT trade_digest FROM trade_facts WHERE venue=$1 AND exchange_order_id=$2 AND trade_id=$3",
            &[&trade.venue, &trade.exchange_order_id, &trade.trade_id],
        ).await.context("failed to inspect duplicate trade")?;
        let version = current.last_fact_version + 1;
        let now = db_time()?;
        if let Some(row) = existing {
            let old_digest: String = row.get(0);
            let same_content = old_digest == digest;
            let action = if same_content {
                "TRADE_DUPLICATE"
            } else {
                "TRADE_CONFLICT"
            };
            append_action(
                &tx,
                &intent_id,
                version,
                action,
                None,
                now,
                json!({"trade_id": trade.trade_id, "digest": digest}),
            )
            .await?;
            if same_content {
                tx.execute(
                    "UPDATE order_facts SET last_fact_version=$2,updated_at_ms=$3 WHERE intent_id=$1",
                    &[&intent_id, &version, &now],
                )
                .await
                .context("failed to persist duplicate trade fact")?;
            } else {
                tx.execute(
                    "UPDATE order_facts SET reconciliation_status='CONFLICT',last_fact_version=$2,updated_at_ms=$3 WHERE intent_id=$1",
                    &[&intent_id, &version, &now],
                )
                .await
                .context("failed to persist trade conflict")?;
            }
            tx.commit()
                .await
                .context("failed to commit duplicate trade fact")?;
            if !same_content {
                bail!("trade identity was reused with different content");
            }
            return self.load(&intent_id).await;
        }
        tx.execute(
            "INSERT INTO trade_facts(venue,exchange_order_id,trade_id,intent_id,trade_digest,quantity,price,fee_asset,fee_amount,source_sequence,occurred_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            &[&trade.venue, &trade.exchange_order_id, &trade.trade_id, &intent_id, &digest, &trade.quantity, &trade.price, &trade.fee_asset, &trade.fee_amount, &(trade.source_sequence as i64), &to_i64(trade.occurred_at_ms)?],
        ).await.context("failed to insert trade fact")?;
        let filled = current.filled_quantity + trade.quantity;
        let reconciliation = if filled <= current.ordered_quantity {
            ReconciliationStatus::Matched
        } else {
            ReconciliationStatus::Conflict
        };
        tx.execute(
            "UPDATE order_facts SET filled_quantity=$2,reconciliation_status=$3,last_fact_version=$4,updated_at_ms=$5 WHERE intent_id=$1",
            &[&intent_id, &filled, &reconciliation.as_str(), &version, &now],
        ).await.context("failed to update filled quantity")?;
        append_action(
            &tx,
            &intent_id,
            version,
            "TRADE_RECORDED",
            None,
            now,
            json!({"trade_id": trade.trade_id, "quantity": trade.quantity}),
        )
        .await?;
        tx.commit().await.context("failed to commit trade fact")?;
        self.load(&intent_id).await
    }

    pub async fn load(&self, intent_id: &str) -> Result<OrderFactSnapshot> {
        let intent_id = intent_id.to_owned();
        let row = self.inner.client.query_one("SELECT intent_id,account_id,instrument_id,venue,ordered_quantity,submission_status,cancel_status,reconciliation_status,exchange_order_id,filled_quantity,last_fact_version,last_error FROM order_facts WHERE intent_id=$1 AND account_id=$2 AND instrument_id=$3", &[&intent_id, &self.inner.account_id, &self.inner.instrument_id]).await.context("failed to load B-02 order fact")?;
        snapshot(row)
    }

    pub async fn recover_nonterminal(&self) -> Result<Vec<OrderFactSnapshot>> {
        let rows = self.inner.client.query("SELECT intent_id,account_id,instrument_id,venue,ordered_quantity,submission_status,cancel_status,reconciliation_status,exchange_order_id,filled_quantity,last_fact_version,last_error FROM order_facts WHERE account_id=$1 AND instrument_id=$2 AND submission_status NOT IN ('DEFINITELY_REJECTED') ORDER BY intent_id", &[&self.inner.account_id, &self.inner.instrument_id]).await.context("failed to recover B-02 order facts")?;
        rows.into_iter().map(snapshot).collect()
    }

    pub async fn disconnect(self) {
        self.inner.disconnect().await;
    }
}

async fn lock_fact(
    tx: &tokio_postgres::Transaction<'_>,
    intent_id: &str,
) -> Result<OrderFactSnapshot> {
    let row = tx.query_one("SELECT intent_id,account_id,instrument_id,venue,ordered_quantity,submission_status,cancel_status,reconciliation_status,exchange_order_id,filled_quantity,last_fact_version,last_error FROM order_facts WHERE intent_id=$1 FOR UPDATE", &[&intent_id]).await.context("B-02 order intent fact is missing")?;
    snapshot(row)
}

async fn append_action(
    tx: &tokio_postgres::Transaction<'_>,
    intent_id: &str,
    version: i64,
    action: &str,
    request_digest: Option<&str>,
    now: i64,
    payload: serde_json::Value,
) -> Result<()> {
    let fact_id = format!("{intent_id}:{version}");
    tx.execute("INSERT INTO order_action_facts(fact_id,intent_id,fact_version,action,request_digest,occurred_at_ms,payload) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&fact_id, &intent_id, &version, &action, &request_digest, &now, &payload]).await.context("failed to append immutable B-02 action fact")?;
    Ok(())
}

fn snapshot(row: Row) -> Result<OrderFactSnapshot> {
    let submission = parse_submission(row.get(5))?;
    let cancel = parse_cancel(row.get(6))?;
    let reconciliation = parse_reconciliation(row.get(7))?;
    Ok(OrderFactSnapshot {
        intent_id: row.get(0),
        account_id: row.get(1),
        instrument_id: row.get(2),
        venue: row.get(3),
        ordered_quantity: row.get(4),
        submission_status: submission,
        cancel_status: cancel,
        reconciliation_status: reconciliation,
        exchange_order_id: row.get(8),
        filled_quantity: row.get(9),
        last_fact_version: row.get(10),
        last_error: row.get(11),
    })
}

fn parse_submission(value: &str) -> Result<SubmissionStatus> {
    match value {
        "NOT_SENT" => Ok(SubmissionStatus::NotSent),
        "IN_FLIGHT" => Ok(SubmissionStatus::InFlight),
        "ACCEPTED" => Ok(SubmissionStatus::Accepted),
        "DEFINITELY_REJECTED" => Ok(SubmissionStatus::DefinitelyRejected),
        "UNKNOWN" => Ok(SubmissionStatus::Unknown),
        _ => bail!("unknown submission status: {value}"),
    }
}
fn parse_cancel(value: &str) -> Result<CancelStatus> {
    match value {
        "NONE" => Ok(CancelStatus::None),
        "REQUESTED" => Ok(CancelStatus::Requested),
        "UNKNOWN" => Ok(CancelStatus::Unknown),
        "CONFIRMED" => Ok(CancelStatus::Confirmed),
        _ => bail!("unknown cancel status: {value}"),
    }
}
fn parse_reconciliation(value: &str) -> Result<ReconciliationStatus> {
    match value {
        "PENDING" => Ok(ReconciliationStatus::Pending),
        "MATCHED" => Ok(ReconciliationStatus::Matched),
        "CONFLICT" => Ok(ReconciliationStatus::Conflict),
        _ => bail!("unknown reconciliation status: {value}"),
    }
}
fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("request digest must be 64 hexadecimal characters");
    }
    Ok(())
}
fn validate_trade(trade: &TradeInput) -> Result<()> {
    validate_id("trade venue", &trade.venue)?;
    validate_id("exchange_order_id", &trade.exchange_order_id)?;
    validate_id("trade_id", &trade.trade_id)?;
    validate_id("fee_asset", &trade.fee_asset)?;
    if trade.quantity <= Decimal::ZERO
        || trade.price <= Decimal::ZERO
        || trade.fee_amount < Decimal::ZERO
    {
        bail!("trade quantity and price must be positive, fee must be non-negative");
    }
    Ok(())
}
fn trade_digest(trade: &TradeInput) -> Result<String> {
    let bytes = serde_json::to_vec(&(
        trade.venue.as_str(),
        trade.exchange_order_id.as_str(),
        trade.trade_id.as_str(),
        trade.quantity,
        trade.price,
        trade.fee_asset.as_str(),
        trade.fee_amount,
        trade.source_sequence,
        trade.occurred_at_ms,
    ))
    .context("failed to digest trade")?;
    Ok(hex_digest(Sha256::digest(bytes).as_slice()))
}

pub async fn run_order_facts_smoke(database_url: &str) -> Result<OrderFactsSmokeReport> {
    migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let account = format!("order-account-{stamp}");
    let instrument = format!("order-instrument-{stamp}");
    set_paper_balance(
        database_url,
        &account,
        "order-buy",
        "USDT",
        Decimal::from(100),
        Decimal::from(100),
        unix_timestamp_ms()?,
    )
    .await?;
    set_paper_balance(
        database_url,
        &account,
        "order-sell",
        "BTC",
        Decimal::from(1),
        Decimal::from(1),
        unix_timestamp_ms()?,
    )
    .await?;
    let mut paper = crate::paper::PaperCore::acquire(database_url, &account, &instrument).await?;
    let prefix = format!("order-smoke-{stamp}");
    let request = ReservePlanRequest {
        request_id: format!("{prefix}-request"),
        plan_id: format!("{prefix}-plan"),
        account_id: account.clone(),
        instrument_id: instrument.clone(),
        opportunity_id: format!("{prefix}-opportunity"),
        strategy_config_version: "order-smoke-v1".to_owned(),
        target_quantity: Decimal::new(1, 2),
        max_unmatched_exposure: Decimal::from(1),
        intents: [
            OrderIntentInput {
                intent_id: format!("{prefix}-intent"),
                leg_id: "buy".to_owned(),
                attempt_id: format!("{prefix}-attempt"),
                client_order_id: format!("{prefix}-client"),
                venue: "order-buy".to_owned(),
                side: OrderSide::Buy,
                quantity: Decimal::new(1, 2),
                limit_price: Decimal::from(90),
            },
            OrderIntentInput {
                intent_id: format!("{prefix}-sell-intent"),
                leg_id: "sell".to_owned(),
                attempt_id: format!("{prefix}-sell-attempt"),
                client_order_id: format!("{prefix}-sell-client"),
                venue: "order-sell".to_owned(),
                side: OrderSide::Sell,
                quantity: Decimal::new(1, 2),
                limit_price: Decimal::from(100),
            },
        ],
        reservations: vec![
            ReservationInput {
                reservation_id: format!("{prefix}-quote-reservation"),
                venue: "order-buy".to_owned(),
                asset: "USDT".to_owned(),
                amount: Decimal::from(90),
            },
            ReservationInput {
                reservation_id: format!("{prefix}-base-reservation"),
                venue: "order-sell".to_owned(),
                asset: "BTC".to_owned(),
                amount: Decimal::new(1, 2),
            },
        ],
    };
    let outcome = paper.reserve_plan(request).await?;
    paper.disconnect().await;
    let intent_id = outcome
        .plan
        .intents
        .iter()
        .find(|intent| intent.side == OrderSide::Buy)
        .context("smoke BUY intent missing")?
        .intent_id
        .clone();
    let mut core = OrderCore::acquire(database_url, &account, &instrument).await?;
    core.ensure_intent(&intent_id).await?;
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    core.submit_started(&intent_id, digest).await?;
    let after_timeout = core
        .submit_result(
            &intent_id,
            SubmitResult::Unknown {
                reason: "transport timeout".to_owned(),
            },
        )
        .await?;
    let after_not_found = core
        .query_result(
            &intent_id,
            QueryResult::NotFound {
                query_id: format!("{prefix}-query-1"),
                visibility_deadline_ms: unix_timestamp_ms()? + 60_000,
            },
        )
        .await?;
    let after_found = core
        .query_result(
            &intent_id,
            QueryResult::Found {
                query_id: format!("{prefix}-query-2"),
                exchange_order_id: format!("{prefix}-exchange-order"),
            },
        )
        .await?;
    let trade = TradeInput {
        venue: "order-buy".to_owned(),
        exchange_order_id: format!("{prefix}-exchange-order"),
        trade_id: format!("{prefix}-trade"),
        quantity: Decimal::new(1, 2),
        price: Decimal::from(90),
        fee_asset: "USDT".to_owned(),
        fee_amount: Decimal::new(9, 2),
        source_sequence: 1,
        occurred_at_ms: unix_timestamp_ms()?,
    };
    let after_trade = core.record_trade(&intent_id, trade.clone()).await?;
    let duplicate = core.record_trade(&intent_id, trade).await?;
    core.cancel_requested(&intent_id).await?;
    let after_cancel = core
        .cancel_result(&intent_id, CancelResult::Confirmed)
        .await?;
    let rejected_intent_id = outcome
        .plan
        .intents
        .iter()
        .find(|intent| intent.side == OrderSide::Sell)
        .context("smoke SELL intent missing")?
        .intent_id
        .clone();
    core.ensure_intent(&rejected_intent_id).await?;
    core.submit_started(&rejected_intent_id, digest).await?;
    let after_rejection = core
        .submit_result(
            &rejected_intent_id,
            SubmitResult::DefinitelyRejected {
                reason: "venue rejected smoke order".to_owned(),
            },
        )
        .await?;
    let recovered = core.recover_nonterminal().await?;
    core.disconnect().await;
    Ok(OrderFactsSmokeReport {
        schema_version: 2,
        submit_definitely_rejected: after_rejection.submission_status
            == SubmissionStatus::DefinitelyRejected,
        submit_timeout_unknown: after_timeout.submission_status == SubmissionStatus::Unknown,
        query_not_found_preserved_unknown: after_not_found.submission_status
            == SubmissionStatus::Unknown,
        query_found_recovered: after_found.submission_status == SubmissionStatus::Accepted
            && after_found.exchange_order_id.is_some(),
        duplicate_trade_ignored: duplicate.filled_quantity == after_trade.filled_quantity,
        cancel_race_trade_preserved: after_cancel.cancel_status == CancelStatus::Confirmed
            && after_cancel.filled_quantity == Decimal::new(1, 2),
        recovered_filled_quantity: recovered
            .first()
            .map(|fact| fact.filled_quantity)
            .unwrap_or_default(),
        recovered_submission_status: recovered
            .first()
            .map(|fact| fact.submission_status)
            .unwrap_or(SubmissionStatus::NotSent),
        recovered_cancel_status: recovered
            .first()
            .map(|fact| fact.cancel_status)
            .unwrap_or(CancelStatus::None),
        external_order_calls: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strings_are_closed_world() {
        assert_eq!(SubmissionStatus::Unknown.as_str(), "UNKNOWN");
        assert_eq!(CancelStatus::Confirmed.as_str(), "CONFIRMED");
        assert_eq!(ReconciliationStatus::Conflict.as_str(), "CONFLICT");
    }

    #[test]
    fn trade_digest_changes_when_economic_content_changes() {
        let mut trade = TradeInput {
            venue: "v".to_owned(),
            exchange_order_id: "o".to_owned(),
            trade_id: "t".to_owned(),
            quantity: Decimal::ONE,
            price: Decimal::ONE,
            fee_asset: "q".to_owned(),
            fee_amount: Decimal::ZERO,
            source_sequence: 1,
            occurred_at_ms: 1,
        };
        let first = trade_digest(&trade).unwrap();
        trade.quantity += Decimal::ONE;
        assert_ne!(first, trade_digest(&trade).unwrap());
    }
}
