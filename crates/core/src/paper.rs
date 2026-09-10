use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgRow, Postgres};
use sqlx::{Row, Transaction};

use crate::db::{
    DomainConnection, SCHEMA_VERSION, advisory_key, hex_digest, migrate, pool, ready_pool, to_i64,
    validate_id,
};
use crate::market::unix_timestamp_ms;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderIntentInput {
    pub intent_id: String,
    pub leg_id: String,
    pub attempt_id: String,
    pub client_order_id: String,
    pub venue: String,
    pub side: OrderSide,
    pub quantity: Decimal,
    pub limit_price: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationInput {
    pub reservation_id: String,
    pub venue: String,
    pub asset: String,
    pub amount: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservePlanRequest {
    pub request_id: String,
    pub plan_id: String,
    pub account_id: String,
    pub instrument_id: String,
    pub opportunity_id: String,
    pub strategy_config_version: String,
    pub target_quantity: Decimal,
    pub max_unmatched_exposure: Decimal,
    pub intents: [OrderIntentInput; 2],
    pub reservations: Vec<ReservationInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveredIntent {
    pub intent_id: String,
    pub leg_id: String,
    pub attempt_id: String,
    pub client_order_id: String,
    pub venue: String,
    pub side: OrderSide,
    pub quantity: Decimal,
    pub limit_price: Decimal,
    pub submission_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveredReservation {
    pub reservation_id: String,
    pub venue: String,
    pub asset: String,
    pub amount: Decimal,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveredPlan {
    pub plan_id: String,
    pub request_id: String,
    pub request_digest: String,
    pub account_id: String,
    pub instrument_id: String,
    pub opportunity_id: String,
    pub strategy_config_version: String,
    pub target_quantity: Decimal,
    pub max_unmatched_exposure: Decimal,
    pub state: String,
    pub created_at_ms: u64,
    pub intents: Vec<RecoveredIntent>,
    pub reservations: Vec<RecoveredReservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReserveOutcome {
    pub idempotent_replay: bool,
    pub plan: RecoveredPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaperCoreSmokeReport {
    pub schema_version: i64,
    pub same_domain_lock_rejected: bool,
    pub concurrent_attempts: usize,
    pub concurrent_successes: usize,
    pub concurrent_rejections: usize,
    pub insufficient_request_rolled_back: bool,
    pub idempotent_replay: bool,
    pub conflicting_replay_rejected: bool,
    pub recovered_plans: usize,
    pub recovered_risk_decisions: usize,
    pub recovered_intents: usize,
    pub recovered_reservations: usize,
    pub recovered_audit_events: usize,
    pub audit_events_immutable: bool,
    pub rejected_request_rows: i64,
    pub all_recovered_intents_not_sent: bool,
    pub all_recovered_funds_local_reserved: bool,
    pub external_order_calls: usize,
}

pub struct PaperCore {
    inner: DomainConnection,
}

pub async fn set_paper_balance(
    database_url: &str,
    account_id: &str,
    venue: &str,
    asset: &str,
    observed_total: Decimal,
    observed_free: Decimal,
    observed_at_ms: u64,
) -> Result<()> {
    validate_id("account_id", account_id)?;
    validate_id("venue", venue)?;
    validate_id("asset", asset)?;
    if observed_total < Decimal::ZERO
        || observed_free < Decimal::ZERO
        || observed_free > observed_total
    {
        bail!("PAPER balance requires 0 <= observed_free <= observed_total");
    }
    if observed_at_ms == 0 {
        bail!("PAPER balance observed_at_ms must be positive");
    }
    let observed_at_ms = to_i64(observed_at_ms)?;
    let pool = ready_pool(database_url).await?;
    let mut transaction = pool
        .begin()
        .await
        .context("failed to begin PAPER balance transaction")?;
    let balance_lock = balance_advisory_key(account_id, venue, asset);
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(balance_lock)
        .execute(&mut *transaction)
        .await
        .context("failed to serialize PAPER balance initialization")?;
    let active: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM fund_reservations WHERE account_id=$1 AND venue=$2 AND asset=$3 AND state='LOCAL_RESERVED')",
    )
    .bind(account_id)
    .bind(venue)
    .bind(asset)
    .fetch_one(&mut *transaction)
    .await
    .context("failed to inspect active PAPER reservations")?
    .try_get::<bool, _>(0)?;
    if active {
        bail!("cannot replace a PAPER balance with active local reservations");
    }
    sqlx::query(
        "INSERT INTO paper_balances(account_id,venue,asset,observed_total,observed_free,local_reserved,observed_at_ms) VALUES($1,$2,$3,$4,$5,0,$6) ON CONFLICT(account_id,venue,asset) DO UPDATE SET observed_total=EXCLUDED.observed_total, observed_free=EXCLUDED.observed_free, local_reserved=0, observed_at_ms=EXCLUDED.observed_at_ms",
    )
    .bind(account_id)
    .bind(venue)
    .bind(asset)
    .bind(observed_total)
    .bind(observed_free)
    .bind(observed_at_ms)
    .execute(&mut *transaction)
    .await
    .context("failed to store PAPER balance")?;
    transaction
        .commit()
        .await
        .context("failed to commit PAPER balance")?;
    Ok(())
}

impl PaperCore {
    pub async fn acquire(
        database_url: &str,
        account_id: impl Into<String>,
        instrument_id: impl Into<String>,
    ) -> Result<Self> {
        let inner =
            DomainConnection::acquire(database_url, account_id, instrument_id, "PAPER").await?;
        Ok(Self { inner })
    }

    pub async fn reserve_plan(&mut self, request: ReservePlanRequest) -> Result<ReserveOutcome> {
        validate_request(&request, &self.inner.account_id, &self.inner.instrument_id)?;
        let digest = request_digest(&request)?;
        let created_at_ms = unix_timestamp_ms()?;
        let created_at_db = to_i64(created_at_ms)?;
        let mut transaction = self
            .inner
            .pool
            .begin()
            .await
            .context("failed to begin plan reservation transaction")?;
        let request_lock = advisory_key("reserve-request", &request.request_id);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(request_lock)
            .execute(&mut *transaction)
            .await
            .context("failed to serialize PAPER request")?;

        if let Some(row) =
            sqlx::query("SELECT plan_id, request_digest FROM execution_plans WHERE request_id=$1")
                .bind(&request.request_id)
                .fetch_optional(&mut *transaction)
                .await
                .context("failed to inspect idempotent PAPER request")?
        {
            let existing_plan_id: String = row.try_get(0)?;
            let existing_digest: String = row.try_get(1)?;
            if existing_digest != digest {
                bail!("request_id is already bound to different plan content");
            }
            transaction
                .commit()
                .await
                .context("failed to finish idempotent PAPER request")?;
            let plan = self.load_plan(&existing_plan_id).await?;
            return Ok(ReserveOutcome {
                idempotent_replay: true,
                plan,
            });
        }

        lock_and_check_balances(&mut transaction, &request).await?;
        insert_plan(&mut transaction, &request, &digest, created_at_db).await?;
        transaction
            .commit()
            .await
            .context("failed to commit atomic PAPER plan reservation")?;
        let plan = self.load_plan(&request.plan_id).await?;
        Ok(ReserveOutcome {
            idempotent_replay: false,
            plan,
        })
    }

    pub async fn recover_active_plans(&self) -> Result<Vec<RecoveredPlan>> {
        let rows = sqlx::query(
            "SELECT plan_id FROM execution_plans WHERE account_id=$1 AND instrument_id=$2 AND state='RESERVED' ORDER BY created_at_ms, plan_id",
        )
        .bind(&self.inner.account_id)
        .bind(&self.inner.instrument_id)
        .fetch_all(&self.inner.pool)
        .await
        .context("failed to list active PAPER plans")?;
        let mut plans = Vec::with_capacity(rows.len());
        for row in rows {
            let plan_id: String = row.try_get(0)?;
            plans.push(self.load_plan(&plan_id).await?);
        }
        Ok(plans)
    }

    pub async fn disconnect(self) {
        self.inner.disconnect().await;
    }

    async fn load_plan(&self, plan_id: &str) -> Result<RecoveredPlan> {
        let row = sqlx::query(
            "SELECT plan_id,request_id,request_digest,account_id,instrument_id,opportunity_id,strategy_config_version,target_quantity,max_unmatched_exposure,state,created_at_ms FROM execution_plans WHERE plan_id=$1",
        )
        .bind(plan_id)
        .fetch_one(&self.inner.pool)
        .await
        .context("failed to load PAPER execution plan")?;
        let intent_rows = sqlx::query(
            "SELECT intent_id,leg_id,attempt_id,client_order_id,venue,side,quantity,limit_price,submission_status FROM order_intents WHERE plan_id=$1 ORDER BY leg_id",
        )
        .bind(plan_id)
        .fetch_all(&self.inner.pool)
        .await
        .context("failed to load PAPER order intents")?;
        let reservation_rows = sqlx::query(
            "SELECT reservation_id,venue,asset,amount,state FROM fund_reservations WHERE plan_id=$1 ORDER BY venue,asset",
        )
        .bind(plan_id)
        .fetch_all(&self.inner.pool)
        .await
        .context("failed to load PAPER fund reservations")?;
        recovered_plan(row, intent_rows, reservation_rows)
    }
}

pub async fn run_paper_core_smoke(database_url: &str) -> Result<PaperCoreSmokeReport> {
    migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let account = format!("paper-account-{stamp}");
    let instrument_a = format!("paper:BTC-USDT:{stamp}");
    let instrument_b = format!("paper:ETH-USDT:{stamp}");
    let now = unix_timestamp_ms()?;
    set_paper_balance(
        database_url,
        &account,
        "paper-buy",
        "USDT",
        Decimal::from(100),
        Decimal::from(100),
        now,
    )
    .await?;
    set_paper_balance(
        database_url,
        &account,
        "paper-sell",
        "BTC",
        Decimal::ONE,
        Decimal::ONE,
        now,
    )
    .await?;

    let mut core_a = PaperCore::acquire(database_url, &account, &instrument_a).await?;
    let same_domain_lock_rejected = PaperCore::acquire(database_url, &account, &instrument_a)
        .await
        .is_err();
    let mut core_b = PaperCore::acquire(database_url, &account, &instrument_b).await?;
    let request_a = smoke_request(&stamp, "a", &account, &instrument_a);
    let request_b = smoke_request(&stamp, "b", &account, &instrument_b);
    let (result_a, result_b) = tokio::join!(
        core_a.reserve_plan(request_a.clone()),
        core_b.reserve_plan(request_b.clone())
    );
    let concurrent_successes = usize::from(result_a.is_ok()) + usize::from(result_b.is_ok());
    let concurrent_rejections = usize::from(result_a.is_err()) + usize::from(result_b.is_err());
    if concurrent_successes != 1 || concurrent_rejections != 1 {
        bail!("PAPER balance competition did not produce exactly one winner");
    }

    let (losing_plan_id, losing_request_id) = if result_a.is_err() {
        (request_a.plan_id.clone(), request_a.request_id.clone())
    } else {
        (request_b.plan_id.clone(), request_b.request_id.clone())
    };
    let (winning_request, winning_instrument, replay, conflict_rejected) = if result_a.is_ok() {
        let replay = core_a.reserve_plan(request_a.clone()).await?;
        let mut conflict = request_a.clone();
        conflict.target_quantity += Decimal::new(1, 4);
        let conflict_rejected = core_a.reserve_plan(conflict).await.is_err();
        (request_a, instrument_a.clone(), replay, conflict_rejected)
    } else {
        let replay = core_b.reserve_plan(request_b.clone()).await?;
        let mut conflict = request_b.clone();
        conflict.target_quantity += Decimal::new(1, 4);
        let conflict_rejected = core_b.reserve_plan(conflict).await.is_err();
        (request_b, instrument_b.clone(), replay, conflict_rejected)
    };
    let rejected_request_rows =
        business_row_count(database_url, &losing_plan_id, &losing_request_id).await?;
    let insufficient_request_rolled_back = rejected_request_rows == 0;

    core_a.disconnect().await;
    core_b.disconnect().await;
    let recovered_core = PaperCore::acquire(database_url, &account, &winning_instrument).await?;
    let recovered = recovered_core.recover_active_plans().await?;
    if recovered.len() != 1 || recovered[0].plan_id != winning_request.plan_id {
        bail!("PAPER recovery did not return the committed active plan");
    }
    let recovered_risk_decisions: i64 =
        sqlx::query("SELECT count(*) FROM risk_decisions WHERE plan_id=$1 AND approved")
            .bind(&winning_request.plan_id)
            .fetch_one(&recovered_core.inner.pool)
            .await
            .context("failed to verify recovered PAPER risk decision")?
            .try_get::<i64, _>(0)?;
    let recovered_audit_events: i64 = sqlx::query(
        "SELECT count(*) FROM audit_events WHERE aggregate_id=$1 AND event_type='PlanReserved'",
    )
    .bind(&winning_request.plan_id)
    .fetch_one(&recovered_core.inner.pool)
    .await
    .context("failed to verify recovered PAPER audit event")?
    .try_get::<i64, _>(0)?;
    let audit_events_immutable =
        sqlx::query("UPDATE audit_events SET payload=payload WHERE aggregate_id=$1")
            .bind(&winning_request.plan_id)
            .execute(&recovered_core.inner.pool)
            .await
            .is_err();
    recovered_core.disconnect().await;
    let recovered_intents = recovered.iter().map(|plan| plan.intents.len()).sum();
    let recovered_reservations = recovered.iter().map(|plan| plan.reservations.len()).sum();
    let report = PaperCoreSmokeReport {
        schema_version: SCHEMA_VERSION,
        same_domain_lock_rejected,
        concurrent_attempts: 2,
        concurrent_successes,
        concurrent_rejections,
        insufficient_request_rolled_back,
        idempotent_replay: replay.idempotent_replay
            && replay.plan.plan_id == winning_request.plan_id,
        conflicting_replay_rejected: conflict_rejected,
        recovered_plans: recovered.len(),
        recovered_risk_decisions: recovered_risk_decisions
            .try_into()
            .context("negative recovered PAPER risk decision count")?,
        recovered_intents,
        recovered_reservations,
        recovered_audit_events: recovered_audit_events
            .try_into()
            .context("negative recovered PAPER audit event count")?,
        audit_events_immutable,
        rejected_request_rows,
        all_recovered_intents_not_sent: recovered.iter().all(|plan| {
            plan.intents
                .iter()
                .all(|intent| intent.submission_status == "NOT_SENT")
        }),
        all_recovered_funds_local_reserved: recovered.iter().all(|plan| {
            plan.reservations
                .iter()
                .all(|reservation| reservation.state == "LOCAL_RESERVED")
        }),
        external_order_calls: 0,
    };
    if !report.same_domain_lock_rejected
        || report.concurrent_successes != 1
        || report.concurrent_rejections != 1
        || !report.insufficient_request_rolled_back
        || !report.idempotent_replay
        || !report.conflicting_replay_rejected
        || report.recovered_risk_decisions != 1
        || report.recovered_intents != 2
        || report.recovered_reservations != 2
        || report.recovered_audit_events != 1
        || !report.audit_events_immutable
        || !report.all_recovered_intents_not_sent
        || !report.all_recovered_funds_local_reserved
        || report.external_order_calls != 0
    {
        bail!("B-01 PAPER smoke acceptance invariant failed: {report:?}");
    }
    Ok(report)
}

fn validate_request(
    request: &ReservePlanRequest,
    writer_account: &str,
    writer_instrument: &str,
) -> Result<()> {
    for (name, value) in [
        ("request_id", request.request_id.as_str()),
        ("plan_id", request.plan_id.as_str()),
        ("account_id", request.account_id.as_str()),
        ("instrument_id", request.instrument_id.as_str()),
        ("opportunity_id", request.opportunity_id.as_str()),
        (
            "strategy_config_version",
            request.strategy_config_version.as_str(),
        ),
    ] {
        validate_id(name, value)?;
    }
    if request.account_id != writer_account || request.instrument_id != writer_instrument {
        bail!("plan request does not match the held PAPER execution domain");
    }
    if request.target_quantity <= Decimal::ZERO {
        bail!("target_quantity must be positive");
    }
    if request.max_unmatched_exposure < Decimal::ZERO {
        bail!("max_unmatched_exposure must be non-negative");
    }
    if request.reservations.is_empty() {
        bail!("at least one fund reservation is required");
    }
    let mut intent_ids = HashSet::new();
    let mut attempt_ids = HashSet::new();
    let mut client_order_ids = HashSet::new();
    let mut leg_ids = HashSet::new();
    let mut venues = HashSet::new();
    let mut has_buy = false;
    let mut has_sell = false;
    for intent in &request.intents {
        for (name, value) in [
            ("intent_id", intent.intent_id.as_str()),
            ("leg_id", intent.leg_id.as_str()),
            ("attempt_id", intent.attempt_id.as_str()),
            ("client_order_id", intent.client_order_id.as_str()),
            ("intent venue", intent.venue.as_str()),
        ] {
            validate_id(name, value)?;
        }
        if !intent_ids.insert(intent.intent_id.as_str())
            || !attempt_ids.insert(intent.attempt_id.as_str())
            || !client_order_ids.insert(intent.client_order_id.as_str())
            || !leg_ids.insert(intent.leg_id.as_str())
        {
            bail!("order intent, attempt, client order and leg IDs must each be unique");
        }
        if !venues.insert(intent.venue.as_str()) {
            bail!("the two PAPER order intents must use different venues");
        }
        has_buy |= intent.side == OrderSide::Buy;
        has_sell |= intent.side == OrderSide::Sell;
        if intent.quantity <= Decimal::ZERO || intent.limit_price <= Decimal::ZERO {
            bail!("order intent quantity and limit_price must be positive");
        }
    }
    if !has_buy || !has_sell {
        bail!("PAPER plan requires one BUY intent and one SELL intent");
    }
    let mut reservation_keys = HashSet::new();
    let mut reservation_ids = HashSet::new();
    for reservation in &request.reservations {
        for (name, value) in [
            ("reservation_id", reservation.reservation_id.as_str()),
            ("reservation venue", reservation.venue.as_str()),
            ("reservation asset", reservation.asset.as_str()),
        ] {
            validate_id(name, value)?;
        }
        if reservation.amount <= Decimal::ZERO {
            bail!("fund reservation amount must be positive");
        }
        if !venues.contains(reservation.venue.as_str()) {
            bail!("fund reservation venue has no matching order intent");
        }
        if !reservation_ids.insert(reservation.reservation_id.as_str())
            || !reservation_keys.insert((reservation.venue.as_str(), reservation.asset.as_str()))
        {
            bail!("fund reservation IDs and venue/asset keys must be unique");
        }
    }
    let reservation_venues = request
        .reservations
        .iter()
        .map(|reservation| reservation.venue.as_str())
        .collect::<HashSet<_>>();
    if reservation_venues != venues {
        bail!("each PAPER order intent venue requires a fund reservation");
    }
    Ok(())
}

fn request_digest(request: &ReservePlanRequest) -> Result<String> {
    let mut canonical = request.clone();
    canonical.intents.sort_by(|a, b| a.leg_id.cmp(&b.leg_id));
    canonical
        .reservations
        .sort_by(|a, b| (&a.venue, &a.asset).cmp(&(&b.venue, &b.asset)));
    let bytes = serde_json::to_vec(&canonical).context("failed to canonicalize PAPER request")?;
    Ok(hex_digest(Sha256::digest(bytes).as_slice()))
}

fn balance_advisory_key(account_id: &str, venue: &str, asset: &str) -> i64 {
    advisory_key("paper-balance", &format!("{account_id}\0{venue}\0{asset}"))
}

async fn lock_and_check_balances(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ReservePlanRequest,
) -> Result<()> {
    let mut reservations = request.reservations.iter().collect::<Vec<_>>();
    reservations.sort_by(|a, b| (&a.venue, &a.asset).cmp(&(&b.venue, &b.asset)));
    for reservation in reservations {
        let balance_lock =
            balance_advisory_key(&request.account_id, &reservation.venue, &reservation.asset);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(balance_lock)
            .execute(&mut **transaction)
            .await
            .context("failed to serialize PAPER balance reservation")?;
        let row = sqlx::query(
            "SELECT observed_free,local_reserved FROM paper_balances WHERE account_id=$1 AND venue=$2 AND asset=$3 FOR UPDATE",
        )
        .bind(&request.account_id)
        .bind(&reservation.venue)
        .bind(&reservation.asset)
        .fetch_optional(&mut **transaction)
        .await
        .context("failed to lock PAPER balance")?
        .with_context(|| {
            format!(
                "PAPER balance is missing for venue={} asset={}",
                reservation.venue, reservation.asset
            )
        })?;
        let observed_free: Decimal = row.try_get(0)?;
        let local_reserved: Decimal = row.try_get(1)?;
        let available = observed_free - local_reserved;
        if available < reservation.amount {
            bail!(
                "insufficient PAPER funds for venue={} asset={}: requested {}, available {}",
                reservation.venue,
                reservation.asset,
                reservation.amount,
                available
            );
        }
    }
    Ok(())
}

async fn insert_plan(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ReservePlanRequest,
    digest: &str,
    created_at_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO execution_plans(plan_id,request_id,request_digest,account_id,instrument_id,opportunity_id,strategy_config_version,target_quantity,max_unmatched_exposure,state,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'RESERVED',$10)",
    )
    .bind(&request.plan_id)
    .bind(&request.request_id)
    .bind(digest)
    .bind(&request.account_id)
    .bind(&request.instrument_id)
    .bind(&request.opportunity_id)
    .bind(&request.strategy_config_version)
    .bind(request.target_quantity)
    .bind(request.max_unmatched_exposure)
    .bind(created_at_ms)
    .execute(&mut **transaction)
    .await
    .context("failed to insert PAPER execution plan")?;
    sqlx::query("INSERT INTO risk_decisions(plan_id,approved,input_digest,decided_at_ms) VALUES($1,TRUE,$2,$3)")
        .bind(&request.plan_id)
        .bind(digest)
        .bind(created_at_ms)
        .execute(&mut **transaction)
        .await
        .context("failed to insert PAPER risk decision")?;
    for reservation in &request.reservations {
        sqlx::query(
            "UPDATE paper_balances SET local_reserved=local_reserved+$1 WHERE account_id=$2 AND venue=$3 AND asset=$4",
        )
        .bind(reservation.amount)
        .bind(&request.account_id)
        .bind(&reservation.venue)
        .bind(&reservation.asset)
        .execute(&mut **transaction)
        .await
        .context("failed to reserve PAPER balance")?;
        sqlx::query(
            "INSERT INTO fund_reservations(reservation_id,plan_id,account_id,venue,asset,amount,state,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,'LOCAL_RESERVED',$7)",
        )
        .bind(&reservation.reservation_id)
        .bind(&request.plan_id)
        .bind(&request.account_id)
        .bind(&reservation.venue)
        .bind(&reservation.asset)
        .bind(reservation.amount)
        .bind(created_at_ms)
        .execute(&mut **transaction)
        .await
        .context("failed to insert PAPER fund reservation")?;
    }
    for intent in &request.intents {
        sqlx::query(
            "INSERT INTO order_intents(intent_id,plan_id,leg_id,attempt_id,client_order_id,venue,side,quantity,limit_price,submission_status,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'NOT_SENT',$10)",
        )
        .bind(&intent.intent_id)
        .bind(&request.plan_id)
        .bind(&intent.leg_id)
        .bind(&intent.attempt_id)
        .bind(&intent.client_order_id)
        .bind(&intent.venue)
        .bind(intent.side.as_str())
        .bind(intent.quantity)
        .bind(intent.limit_price)
        .bind(created_at_ms)
        .execute(&mut **transaction)
        .await
        .context("failed to insert durable PAPER order intent")?;
    }
    let payload = json!({
        "account_id": request.account_id,
        "instrument_id": request.instrument_id,
        "opportunity_id": request.opportunity_id,
        "request_digest": digest,
        "reservation_count": request.reservations.len(),
        "intent_count": request.intents.len(),
        "submission_status": "NOT_SENT"
    });
    sqlx::query(
        "INSERT INTO audit_events(event_id,aggregate_id,aggregate_version,event_type,correlation_id,causation_id,occurred_at_ms,payload) VALUES($1,$2,1,'PlanReserved',$2,$3,$4,$5)",
    )
    .bind(format!("{}:reserved", request.plan_id))
    .bind(&request.plan_id)
    .bind(&request.request_id)
    .bind(created_at_ms)
    .bind(&payload)
    .execute(&mut **transaction)
    .await
    .context("failed to insert immutable PAPER audit event")?;
    Ok(())
}

fn recovered_plan(
    row: PgRow,
    intent_rows: Vec<PgRow>,
    reservation_rows: Vec<PgRow>,
) -> Result<RecoveredPlan> {
    let created_at_ms: i64 = row.try_get(10)?;
    Ok(RecoveredPlan {
        plan_id: row.try_get::<String, _>(0)?,
        request_id: row.try_get::<String, _>(1)?,
        request_digest: row.try_get::<String, _>(2)?,
        account_id: row.try_get::<String, _>(3)?,
        instrument_id: row.try_get::<String, _>(4)?,
        opportunity_id: row.try_get::<String, _>(5)?,
        strategy_config_version: row.try_get::<String, _>(6)?,
        target_quantity: row.try_get::<Decimal, _>(7)?,
        max_unmatched_exposure: row.try_get::<Decimal, _>(8)?,
        state: row.try_get::<String, _>(9)?,
        created_at_ms: created_at_ms
            .try_into()
            .context("database returned a negative plan timestamp")?,
        intents: intent_rows
            .iter()
            .map(|intent| -> Result<RecoveredIntent> {
                let side_str: String = intent.try_get(5)?;
                let side = match side_str.as_str() {
                    "BUY" => OrderSide::Buy,
                    "SELL" => OrderSide::Sell,
                    _ => panic!("unknown order side: {side_str}"),
                };
                Ok(RecoveredIntent {
                    intent_id: intent.try_get::<String, _>(0)?,
                    leg_id: intent.try_get::<String, _>(1)?,
                    attempt_id: intent.try_get::<String, _>(2)?,
                    client_order_id: intent.try_get::<String, _>(3)?,
                    venue: intent.try_get::<String, _>(4)?,
                    side,
                    quantity: intent.try_get::<Decimal, _>(6)?,
                    limit_price: intent.try_get::<Decimal, _>(7)?,
                    submission_status: intent.try_get::<String, _>(8)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        reservations: reservation_rows
            .iter()
            .map(|reservation| -> Result<RecoveredReservation> {
                Ok(RecoveredReservation {
                    reservation_id: reservation.try_get::<String, _>(0)?,
                    venue: reservation.try_get::<String, _>(1)?,
                    asset: reservation.try_get::<String, _>(2)?,
                    amount: reservation.try_get::<Decimal, _>(3)?,
                    state: reservation.try_get::<String, _>(4)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

async fn business_row_count(database_url: &str, plan_id: &str, request_id: &str) -> Result<i64> {
    let pool = pool(database_url).await?;
    let count: i64 = sqlx::query(
        "SELECT (SELECT count(*) FROM execution_plans WHERE plan_id=$1 OR request_id=$2) + (SELECT count(*) FROM risk_decisions WHERE plan_id=$1) + (SELECT count(*) FROM order_intents WHERE plan_id=$1) + (SELECT count(*) FROM fund_reservations WHERE plan_id=$1) + (SELECT count(*) FROM audit_events WHERE aggregate_id=$1 OR causation_id=$2)",
    )
    .bind(plan_id)
    .bind(request_id)
    .fetch_one(&pool)
    .await
    .context("failed to verify rejected PAPER request rollback")?
    .try_get::<i64, _>(0)?;
    Ok(count)
}

fn smoke_request(
    stamp: &str,
    suffix: &str,
    account_id: &str,
    instrument_id: &str,
) -> ReservePlanRequest {
    let prefix = format!("smoke-{stamp}-{suffix}");
    ReservePlanRequest {
        request_id: format!("{prefix}-request"),
        plan_id: format!("{prefix}-plan"),
        account_id: account_id.to_owned(),
        instrument_id: instrument_id.to_owned(),
        opportunity_id: format!("{prefix}-opportunity"),
        strategy_config_version: "paper-smoke-v1".to_owned(),
        target_quantity: Decimal::new(1, 3),
        max_unmatched_exposure: Decimal::from(80),
        intents: [
            OrderIntentInput {
                intent_id: format!("{prefix}-buy-intent"),
                leg_id: "buy".to_owned(),
                attempt_id: format!("{prefix}-buy-attempt"),
                client_order_id: format!("{prefix}-buy-client"),
                venue: "paper-buy".to_owned(),
                side: OrderSide::Buy,
                quantity: Decimal::new(1, 3),
                limit_price: Decimal::from(80_000),
            },
            OrderIntentInput {
                intent_id: format!("{prefix}-sell-intent"),
                leg_id: "sell".to_owned(),
                attempt_id: format!("{prefix}-sell-attempt"),
                client_order_id: format!("{prefix}-sell-client"),
                venue: "paper-sell".to_owned(),
                side: OrderSide::Sell,
                quantity: Decimal::new(1, 3),
                limit_price: Decimal::from(80_100),
            },
        ],
        reservations: vec![
            ReservationInput {
                reservation_id: format!("{prefix}-quote-reservation"),
                venue: "paper-buy".to_owned(),
                asset: "USDT".to_owned(),
                amount: Decimal::from(80),
            },
            ReservationInput {
                reservation_id: format!("{prefix}-base-reservation"),
                venue: "paper-sell".to_owned(),
                asset: "BTC".to_owned(),
                amount: Decimal::new(1, 3),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_digest_is_independent_of_input_order() {
        let mut first = smoke_request("digest", "a", "account", "instrument");
        let mut second = first.clone();
        second.intents.swap(0, 1);
        second.reservations.swap(0, 1);
        assert_eq!(
            request_digest(&first).unwrap(),
            request_digest(&second).unwrap()
        );
        first.target_quantity += Decimal::new(1, 4);
        assert_ne!(
            request_digest(&first).unwrap(),
            request_digest(&second).unwrap()
        );
    }

    #[test]
    fn validation_rejects_two_buy_legs() {
        let mut request = smoke_request("invalid", "a", "account", "instrument");
        request.intents[1].side = OrderSide::Buy;
        assert!(
            validate_request(&request, "account", "instrument")
                .unwrap_err()
                .to_string()
                .contains("one BUY intent and one SELL intent")
        );
    }

    #[test]
    fn validation_requires_reservations_for_both_venues() {
        let mut request = smoke_request("invalid-reservation", "a", "account", "instrument");
        request.reservations.pop();
        assert!(
            validate_request(&request, "account", "instrument")
                .unwrap_err()
                .to_string()
                .contains("each PAPER order intent venue")
        );
    }

    #[test]
    fn advisory_namespaces_do_not_share_keys() {
        assert_ne!(
            advisory_key("execution-domain", "same"),
            advisory_key("reserve-request", "same")
        );
    }
}
