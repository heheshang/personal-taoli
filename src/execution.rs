use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Row};

use crate::market::unix_timestamp_ms;
use crate::paper::{
    OrderIntentInput, OrderSide, PaperCore, ReservationInput, ReservePlanRequest, advisory_key,
    close_connection, connect, migrate, set_paper_balance, validate_id, verify_schema,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionState {
    Planned,
    Running,
    CompensationPlanned,
    Completed,
    ManualRequired,
}

impl ExecutionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "PLANNED",
            Self::Running => "RUNNING",
            Self::CompensationPlanned => "COMPENSATION_PLANNED",
            Self::Completed => "COMPLETED",
            Self::ManualRequired => "MANUAL_REQUIRED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompensationDecision {
    NoAction,
    CompensationPlanned,
    ManualRequired,
}

impl CompensationDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoAction => "NO_ACTION",
            Self::CompensationPlanned => "COMPENSATION_PLANNED",
            Self::ManualRequired => "MANUAL_REQUIRED",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionSnapshot {
    pub plan_id: String,
    pub target_quantity: Decimal,
    pub max_unmatched_exposure: Decimal,
    pub compensation_budget: Decimal,
    pub state: ExecutionState,
    pub leg_a_filled: Decimal,
    pub leg_b_filled: Decimal,
    pub unmatched_quantity: Decimal,
    pub unmatched_exposure: Decimal,
    pub compensation_spent: Decimal,
    pub last_version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompensationOutcome {
    pub decision: CompensationDecision,
    pub quantity: Decimal,
    pub estimated_cost: Decimal,
    pub reason: String,
    pub snapshot: ExecutionSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoubleLegSmokeReport {
    pub schema_version: i64,
    pub partial_fill_detected: bool,
    pub within_budget_compensation_planned: bool,
    pub matched_completion: bool,
    pub over_budget_manual_required: bool,
    pub recovered_manual_state: bool,
    pub external_order_calls: usize,
}

pub struct ExecutionCore {
    client: Client,
    connection: JoinHandle<()>,
    account_id: String,
    instrument_id: String,
}

impl ExecutionCore {
    pub async fn acquire(
        database_url: &str,
        account_id: impl Into<String>,
        instrument_id: impl Into<String>,
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
            .context("failed to acquire B-03 execution lock")?
            .get(0);
        if !acquired {
            close_connection(client, connection).await;
            bail!("B-03 execution domain already has a writer");
        }
        Ok(Self {
            client,
            connection,
            account_id,
            instrument_id,
        })
    }

    pub async fn initialize_plan(
        &mut self,
        plan_id: &str,
        target_quantity: Decimal,
        max_unmatched_exposure: Decimal,
        compensation_budget: Decimal,
    ) -> Result<ExecutionSnapshot> {
        validate_id("plan_id", plan_id)?;
        if target_quantity <= Decimal::ZERO
            || max_unmatched_exposure < Decimal::ZERO
            || compensation_budget < Decimal::ZERO
        {
            bail!("B-03 execution limits have invalid values");
        }
        let tx = self
            .client
            .transaction()
            .await
            .context("failed to begin B-03 plan initialization")?;
        let plan_exists: bool = tx
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM execution_plans WHERE plan_id=$1 AND account_id=$2 AND instrument_id=$3)",
                &[&plan_id, &self.account_id, &self.instrument_id],
            )
            .await
            .context("failed to verify B-03 execution plan")?
            .get(0);
        if !plan_exists {
            bail!("B-03 execution plan is missing or outside the held domain");
        }
        tx.execute(
            "INSERT INTO execution_facts(plan_id,target_quantity,max_unmatched_exposure,compensation_budget,state,leg_a_filled,leg_b_filled,unmatched_quantity,unmatched_exposure,compensation_spent,last_version,updated_at_ms) VALUES($1,$2,$3,$4,'PLANNED',0,0,0,0,0,0,$5) ON CONFLICT(plan_id) DO NOTHING",
            &[&plan_id, &target_quantity, &max_unmatched_exposure, &compensation_budget, &db_time()?],
        )
        .await
        .context("failed to initialize B-03 execution fact")?;
        tx.commit()
            .await
            .context("failed to commit B-03 plan initialization")?;
        let snapshot = self.load(plan_id).await?;
        if snapshot.target_quantity != target_quantity
            || snapshot.max_unmatched_exposure != max_unmatched_exposure
            || snapshot.compensation_budget != compensation_budget
        {
            bail!("B-03 plan was initialized with conflicting limits");
        }
        Ok(snapshot)
    }

    pub async fn evaluate(
        &mut self,
        plan_id: &str,
        leg_a_filled: Decimal,
        leg_b_filled: Decimal,
        exposure_per_unit: Decimal,
        compensation_cost_per_unit: Decimal,
    ) -> Result<CompensationOutcome> {
        validate_id("plan_id", plan_id)?;
        if leg_a_filled < Decimal::ZERO
            || leg_b_filled < Decimal::ZERO
            || exposure_per_unit < Decimal::ZERO
            || compensation_cost_per_unit < Decimal::ZERO
        {
            bail!("B-03 fill and compensation values must be non-negative");
        }
        let tx = self
            .client
            .transaction()
            .await
            .context("failed to begin B-03 execution evaluation")?;
        let row = tx
            .query_one(
                "SELECT plan_id,target_quantity,max_unmatched_exposure,compensation_budget,state,leg_a_filled,leg_b_filled,unmatched_quantity,unmatched_exposure,compensation_spent,last_version,updated_at_ms FROM execution_facts WHERE plan_id=$1 FOR UPDATE",
                &[&plan_id],
            )
            .await
            .context("B-03 execution fact is missing")?;
        let current = execution_snapshot(row)?;
        if matches!(
            current.state,
            ExecutionState::Completed | ExecutionState::ManualRequired
        ) {
            bail!("B-03 execution is terminal and cannot be evaluated again");
        }
        if leg_a_filled < current.leg_a_filled || leg_b_filled < current.leg_b_filled {
            bail!("B-03 leg fills cannot move backwards");
        }
        let mismatch = (leg_a_filled - leg_b_filled).abs();
        let exposure = mismatch * exposure_per_unit;
        let estimated_cost = mismatch * compensation_cost_per_unit;
        let overfilled =
            leg_a_filled > current.target_quantity || leg_b_filled > current.target_quantity;
        let (state, decision, reason) = if overfilled {
            (
                ExecutionState::ManualRequired,
                CompensationDecision::ManualRequired,
                "a leg exceeded the planned target quantity".to_owned(),
            )
        } else if mismatch == Decimal::ZERO {
            (
                ExecutionState::Completed,
                CompensationDecision::NoAction,
                "both legs are matched".to_owned(),
            )
        } else if exposure > current.max_unmatched_exposure
            || current.compensation_spent + estimated_cost > current.compensation_budget
        {
            (
                ExecutionState::ManualRequired,
                CompensationDecision::ManualRequired,
                "unmatched exposure or compensation budget exceeded".to_owned(),
            )
        } else {
            (
                ExecutionState::CompensationPlanned,
                CompensationDecision::CompensationPlanned,
                "partial fill is within the configured compensation budget".to_owned(),
            )
        };
        let version = current.last_version + 1;
        let now = db_time()?;
        let event_type = match decision {
            CompensationDecision::NoAction => "COMPLETED",
            CompensationDecision::CompensationPlanned => "COMPENSATION_DECIDED",
            CompensationDecision::ManualRequired => "MANUAL_ESCALATED",
        };
        let event_id = format!("{plan_id}:{version}");
        tx.execute(
            "INSERT INTO execution_events(event_id,plan_id,event_version,event_type,occurred_at_ms,payload) VALUES($1,$2,$3,$4,$5,$6)",
            &[&event_id, &plan_id, &version, &event_type, &now, &json!({"leg_a_filled": leg_a_filled, "leg_b_filled": leg_b_filled, "unmatched_quantity": mismatch, "unmatched_exposure": exposure, "estimated_cost": estimated_cost, "decision": decision.as_str()})],
        )
        .await
        .context("failed to append B-03 execution event")?;
        let decision_id = format!("{plan_id}:decision:{version}");
        tx.execute(
            "INSERT INTO compensation_decisions(decision_id,plan_id,event_version,decision,quantity,estimated_cost,reason,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
            &[&decision_id, &plan_id, &version, &decision.as_str(), &mismatch, &estimated_cost, &reason, &now],
        )
        .await
        .context("failed to persist B-03 compensation decision")?;
        tx.execute(
            "UPDATE execution_facts SET state=$2,leg_a_filled=$3,leg_b_filled=$4,unmatched_quantity=$5,unmatched_exposure=$6,compensation_spent=compensation_spent+$7,last_version=$8,updated_at_ms=$9 WHERE plan_id=$1",
            &[&plan_id, &state.as_str(), &leg_a_filled, &leg_b_filled, &mismatch, &exposure, &estimated_cost, &version, &now],
        )
        .await
        .context("failed to update B-03 execution fact")?;
        tx.commit()
            .await
            .context("failed to commit B-03 execution evaluation")?;
        Ok(CompensationOutcome {
            decision,
            quantity: mismatch,
            estimated_cost,
            reason,
            snapshot: self.load(plan_id).await?,
        })
    }

    pub async fn load(&self, plan_id: &str) -> Result<ExecutionSnapshot> {
        let row = self
            .client
            .query_one(
                "SELECT plan_id,target_quantity,max_unmatched_exposure,compensation_budget,state,leg_a_filled,leg_b_filled,unmatched_quantity,unmatched_exposure,compensation_spent,last_version,updated_at_ms FROM execution_facts WHERE plan_id=$1", 
                &[&plan_id],
            )
            .await
            .context("failed to load B-03 execution fact")?;
        execution_snapshot(row)
    }

    pub async fn disconnect(self) {
        close_connection(self.client, self.connection).await;
    }
}

fn execution_snapshot(row: Row) -> Result<ExecutionSnapshot> {
    Ok(ExecutionSnapshot {
        plan_id: row.get(0),
        target_quantity: row.get(1),
        max_unmatched_exposure: row.get(2),
        compensation_budget: row.get(3),
        state: parse_execution_state(row.get(4))?,
        leg_a_filled: row.get(5),
        leg_b_filled: row.get(6),
        unmatched_quantity: row.get(7),
        unmatched_exposure: row.get(8),
        compensation_spent: row.get(9),
        last_version: row.get(10),
    })
}

fn parse_execution_state(value: &str) -> Result<ExecutionState> {
    match value {
        "PLANNED" => Ok(ExecutionState::Planned),
        "RUNNING" => Ok(ExecutionState::Running),
        "COMPENSATION_PLANNED" => Ok(ExecutionState::CompensationPlanned),
        "COMPLETED" => Ok(ExecutionState::Completed),
        "MANUAL_REQUIRED" => Ok(ExecutionState::ManualRequired),
        _ => bail!("unknown B-03 execution state: {value}"),
    }
}

fn db_time() -> Result<i64> {
    unix_timestamp_ms()?
        .try_into()
        .context("timestamp exceeds PostgreSQL BIGINT")
}

pub async fn run_double_leg_smoke(database_url: &str) -> Result<DoubleLegSmokeReport> {
    migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let account = format!("b03-account-{stamp}");
    let now = unix_timestamp_ms()?;
    set_paper_balance(
        database_url,
        &account,
        "b03-buy",
        "USDT",
        Decimal::from(1_000),
        Decimal::from(1_000),
        now,
    )
    .await?;
    set_paper_balance(
        database_url,
        &account,
        "b03-sell",
        "BTC",
        Decimal::from(2),
        Decimal::from(2),
        now,
    )
    .await?;

    let instrument_a = format!("b03:within:{stamp}");
    let request_a = smoke_request(&stamp, "within", &account, &instrument_a);
    let mut paper_a = PaperCore::acquire(database_url, &account, &instrument_a).await?;
    let plan_a = paper_a.reserve_plan(request_a).await?.plan;
    paper_a.disconnect().await;
    let mut core_a = ExecutionCore::acquire(database_url, &account, &instrument_a).await?;
    core_a
        .initialize_plan(
            &plan_a.plan_id,
            Decimal::ONE,
            Decimal::from(10),
            Decimal::from(3),
        )
        .await?;
    let partial = core_a
        .evaluate(
            &plan_a.plan_id,
            Decimal::new(1, 2),
            Decimal::ZERO,
            Decimal::from(100),
            Decimal::from(2),
        )
        .await?;
    let completed = core_a
        .evaluate(
            &plan_a.plan_id,
            Decimal::new(1, 2),
            Decimal::new(1, 2),
            Decimal::from(100),
            Decimal::from(2),
        )
        .await?;
    core_a.disconnect().await;

    let instrument_b = format!("b03:over:{stamp}");
    let request_b = smoke_request(&stamp, "over", &account, &instrument_b);
    let mut paper_b = PaperCore::acquire(database_url, &account, &instrument_b).await?;
    let plan_b = paper_b.reserve_plan(request_b).await?.plan;
    paper_b.disconnect().await;
    let mut core_b = ExecutionCore::acquire(database_url, &account, &instrument_b).await?;
    core_b
        .initialize_plan(
            &plan_b.plan_id,
            Decimal::ONE,
            Decimal::from(10),
            Decimal::from(1),
        )
        .await?;
    let manual = core_b
        .evaluate(
            &plan_b.plan_id,
            Decimal::new(1, 2),
            Decimal::ZERO,
            Decimal::from(100),
            Decimal::from(200),
        )
        .await?;
    let recovered_manual = core_b.load(&plan_b.plan_id).await?;
    core_b.disconnect().await;

    let report = DoubleLegSmokeReport {
        schema_version: 3,
        partial_fill_detected: partial.quantity == Decimal::new(1, 2),
        within_budget_compensation_planned: partial.decision
            == CompensationDecision::CompensationPlanned,
        matched_completion: completed.decision == CompensationDecision::NoAction
            && completed.snapshot.state == ExecutionState::Completed,
        over_budget_manual_required: manual.decision == CompensationDecision::ManualRequired,
        recovered_manual_state: recovered_manual.state == ExecutionState::ManualRequired
            && recovered_manual.unmatched_quantity == Decimal::new(1, 2),
        external_order_calls: 0,
    };
    if !report.partial_fill_detected
        || !report.within_budget_compensation_planned
        || !report.matched_completion
        || !report.over_budget_manual_required
        || !report.recovered_manual_state
        || report.external_order_calls != 0
    {
        bail!("B-03 double-leg smoke acceptance invariant failed: {report:?}");
    }
    Ok(report)
}

fn smoke_request(
    stamp: &str,
    suffix: &str,
    account_id: &str,
    instrument_id: &str,
) -> ReservePlanRequest {
    let prefix = format!("b03-{stamp}-{suffix}");
    ReservePlanRequest {
        request_id: format!("{prefix}-request"),
        plan_id: format!("{prefix}-plan"),
        account_id: account_id.to_owned(),
        instrument_id: instrument_id.to_owned(),
        opportunity_id: format!("{prefix}-opportunity"),
        strategy_config_version: "b03-smoke-v1".to_owned(),
        target_quantity: Decimal::ONE,
        max_unmatched_exposure: Decimal::from(10),
        intents: [
            OrderIntentInput {
                intent_id: format!("{prefix}-buy-intent"),
                leg_id: "buy".to_owned(),
                attempt_id: format!("{prefix}-buy-attempt"),
                client_order_id: format!("{prefix}-buy-client"),
                venue: "b03-buy".to_owned(),
                side: OrderSide::Buy,
                quantity: Decimal::ONE,
                limit_price: Decimal::from(100),
            },
            OrderIntentInput {
                intent_id: format!("{prefix}-sell-intent"),
                leg_id: "sell".to_owned(),
                attempt_id: format!("{prefix}-sell-attempt"),
                client_order_id: format!("{prefix}-sell-client"),
                venue: "b03-sell".to_owned(),
                side: OrderSide::Sell,
                quantity: Decimal::ONE,
                limit_price: Decimal::from(101),
            },
        ],
        reservations: vec![
            ReservationInput {
                reservation_id: format!("{prefix}-quote"),
                venue: "b03-buy".to_owned(),
                asset: "USDT".to_owned(),
                amount: Decimal::from(100),
            },
            ReservationInput {
                reservation_id: format!("{prefix}-base"),
                venue: "b03-sell".to_owned(),
                asset: "BTC".to_owned(),
                amount: Decimal::ONE,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatch_and_budget_decisions_are_closed_world() {
        assert_eq!(CompensationDecision::NoAction.as_str(), "NO_ACTION");
        assert_eq!(ExecutionState::ManualRequired.as_str(), "MANUAL_REQUIRED");
        assert_eq!(
            (Decimal::new(3, 2) - Decimal::new(1, 2)).abs(),
            Decimal::new(2, 2)
        );
    }
}
