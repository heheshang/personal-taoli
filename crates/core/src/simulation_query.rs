//! G-01 模拟套利仪表盘只读查询模块（设计 D3）。
//!
//! - 独立只读事务（池化连接 + `SET TRANSACTION READ ONLY`），**不经 `DomainConnection::acquire`**（不抢领域单写者锁）；
//! - 全部金额 `Decimal`（serde-str 全局生效 → DTO 直接透传 core 结构）；
//! - 页面查询层零写路径；`external_order_calls=0` 恒真由引擎 INSERT 契约保证。
//!
//! 投影表 `simulation_runs`（migration 0005）与 `balance_snapshots(source='SIMULATION')`
//! 均为只读冗余，事实单一真相源始终是既有 PAPER fact 表。

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgRow, Postgres};
use sqlx::{Executor, PgPool, Row, Transaction};

use crate::db::{ready_pool, to_u64};

/// PostgreSQL 服务端 JSONB ↔ serde_json::Value（sqlx `json` feature）。
type JsonValue = serde_json::Value;

/// 概览聚合（`get_simulation_overview` 返回）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimulationOverview {
    pub total_runs: u64,
    /// scenario <> REJECTED 的 run 数（含 COMPETED_AWAY 等有事实 run）。
    pub total_success: u64,
    /// 成功 run 的模拟净盈亏合计。
    pub total_net_profit: Decimal,
    /// 成功 run 的扫码预期净盈亏合计。
    pub scanned_net_profit: Decimal,
    /// 净盈亏 > 0 的成功 run 数。
    pub profit_runs: u64,
    /// 场景分布（全部 run）。
    pub by_scenario: Vec<ScenarioCount>,
    /// 最近 20 个 run（列表板块）。
    pub recent_runs: Vec<SimulationRunRow>,
    /// 累计净盈亏曲线（最近 `CUMULATIVE_LIMIT` 个 run，时间升序）。
    pub cumulative_points: Vec<NetProfitPoint>,
    /// 两账户现值（paper_balances，f02-account-* 前缀）。
    pub account_balances: Vec<AccountSnapshot>,
    /// 余额曲线（结算后视图，时间序，最近 1200 行）。
    pub balance_history: Vec<BalancePoint>,
    /// 山脊图按日桶聚合（净盈亏数组）。
    pub ridge: Vec<RidgeBucket>,
    /// 最近 50 条 f02 审计事件（活动日志板块）。
    pub activity: Vec<ActivityEvent>,
    /// 执行流转图聚合（走查 execution_events + audit_events，最近 20 run 数）。
    pub flow: FlowAggregate,
}

/// 执行流转图计数（需求 R04）：状态机 EVALUATED → COMPENSATION_DECIDED → COMPLETED/MANUAL_ESCALATED，
/// 资金流 注入 → 预留 → 双腿成交 → 补偿评估。全部自真实事件表聚合，不可变表为唯一事实来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FlowAggregate {
    /// 完成过机会评估的 run 数（= simulation_runs 中 scenario <> 'REJECTED'，EVALUATED 节点）。
    pub evaluated_runs: u64,
    /// 资金注入并预留的 run 数（audit_events 的 PlanReserved 事件去重）。
    pub reserved_runs: u64,
    /// 双腿均成交的 run 数（execution_events 的 COMPLETED 事件）。
    pub filled_runs: u64,
    /// 进过补偿评估的 run 数（execution_events 的 COMPENSATION_DECIDED）。
    pub compensated_runs: u64,
    /// 以 COMPLETED 完结的 run 数（execution_events COMPLETED 去重）。
    pub completed_runs: u64,
    /// 升级人工的 run 数（execution_events 的 MANUAL_ESCALATED 去重）。
    pub escalated_runs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScenarioCount {
    pub scenario: String,
    pub count: u64,
}

/// 累计净盈亏曲线点（按 `executed_at_ms` 升序全量返回）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NetProfitPoint {
    pub executed_at_ms: u64,
    pub scanned_net_profit: Decimal,
    pub simulated_net_profit: Decimal,
}

/// 列表行（不读 `report` JSONB 全文；主营列查询）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimulationRunRow {
    pub run_id: String,
    pub executed_at_ms: u64,
    pub evaluated_at_ms: u64,
    pub symbol: String,
    pub buy_venue: String,
    pub sell_venue: String,
    pub direction: String,
    pub scenario: String,
    pub rejection_reason: Option<String>,
    pub quantity: Decimal,
    pub bought_quantity: Decimal,
    pub sold_quantity: Decimal,
    pub buy_avg_price: Decimal,
    pub sell_avg_price: Decimal,
    pub buy_fee: Decimal,
    pub sell_fee: Decimal,
    pub scanned_net_profit: Decimal,
    pub simulated_net_profit: Decimal,
    pub compensation_decision: String,
    pub execution_state: String,
    pub idempotent_replay: bool,
    pub external_order_calls: u64,
}

/// 分页列表（`get_simulation_runs` 返回）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimulationRunsPage {
    pub total: u64,
    pub runs: Vec<SimulationRunRow>,
}

/// 单 run 详情（`get_simulation_run_detail` 返回）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimulationRunDetail {
    pub run: Box<SimulationRunRow>,
    /// `report` JSONB 全文（SimulationRunReport，snake_case / SCREAMING_SNAKE_CASE 枚举）。
    pub report: JsonValue,
    /// 双腿意图 + 事实 + 动作流。
    pub intents: Vec<IntentFactRow>,
    pub trades: Vec<TradeFactRow>,
    /// execution_events（plan 状态机：EVALUATED → COMPENSATION_DECIDED → …）。
    pub execution_events: Vec<ExecutionEventRow>,
    /// 该 run 的审计事件（run_id 前缀匹配）。
    pub audit: Vec<AuditEventRow>,
    /// 该 run 两账户三节点快照（source='SIMULATION'）。
    pub balances: Vec<BalancePoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntentFactRow {
    pub intent_id: String,
    pub leg_id: String,
    pub client_order_id: String,
    pub venue: String,
    pub side: String,
    pub quantity: Decimal,
    pub limit_price: Decimal,
    pub submission_status: String,
    pub filled_quantity: Decimal,
    pub exchange_order_id: Option<String>,
    pub cancel_status: String,
    pub reconciliation_status: String,
    pub created_at_ms: u64,
    /// order_action_facts 动作流（时间序）。
    pub actions: Vec<ActionFactRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ActionFactRow {
    pub fact_id: String,
    pub fact_version: i64,
    pub action: String,
    pub occurred_at_ms: u64,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TradeFactRow {
    pub venue: String,
    pub exchange_order_id: String,
    pub trade_id: String,
    pub intent_id: String,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee_asset: String,
    pub fee_amount: Decimal,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionEventRow {
    pub event_id: String,
    pub plan_id: String,
    pub event_version: i64,
    pub event_type: String,
    pub occurred_at_ms: u64,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuditEventRow {
    pub event_id: String,
    pub aggregate_id: String,
    pub aggregate_version: i64,
    pub event_type: String,
    pub correlation_id: String,
    pub occurred_at_ms: u64,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AccountSnapshot {
    pub account_id: String,
    pub venue: String,
    pub asset: String,
    pub observed_total: Decimal,
    pub observed_free: Decimal,
    pub local_reserved: Decimal,
    pub observed_at_ms: u64,
}

/// 余额曲线点（`simulation_runs` 快照解析，node ∈ fund/filled/evaluated）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BalancePoint {
    pub run_id: String,
    pub account_id: String,
    pub venue: String,
    pub asset: String,
    pub node: String,
    pub total: Decimal,
    pub free: Decimal,
    pub observed_at_ms: u64,
}

/// 山脊图按日桶（`bucket_ms = executed_at_ms / day_ms * day_ms`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RidgeBucket {
    pub bucket_start_ms: u64,
    pub bucket_end_ms: u64,
    pub nets: Vec<Decimal>,
}

/// 活动日志事件（overview 用；run 归属按 §2.3 前缀解析）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ActivityEvent {
    pub occurred_at_ms: u64,
    pub event_type: String,
    pub aggregate_id: String,
    pub correlation_id: String,
    pub payload: JsonValue,
    pub run_id: Option<String>,
}

const DAY_MS: u64 = 86_400_000;
/// 余额曲线读取上限（每 run 6 行 ≈ 200 run 窗口，曲线板块前端降采样之后仍有信息量）。
const BALANCE_HISTORY_LIMIT: u64 = 1200;
/// 累计曲线读取上限。曲线按像素渲染，超过该行数不增加可见信息，却会线性放大
/// 每 10s 轮询的传输量与前端 SVG 节点数。
const CUMULATIVE_LIMIT: i64 = 1200;

// ---------------------------------------------------------------------------
// 连接与事务
// ---------------------------------------------------------------------------

/// 打开 G-01 只读事务：从池中借连接、置只读。
///
/// schema 版本由 `ready_pool` 在进程内首次访问时校验一次，此处不再逐次往返。
///
/// 不用裸 `BEGIN READ ONLY`：池化连接上的裸事务语句若未 COMMIT 就归还，会污染下一个借用者
/// （DB-01 探针 T1b/S8 实测 `cannot execute INSERT in a read-only transaction`）。sqlx
/// `Transaction` 在 rollback/drop 时保证连接干净归还（T5b/L7b 实测池仍可写）。
async fn open_readonly(pool: &PgPool) -> Result<Transaction<'_, Postgres>> {
    let mut txn = pool
        .begin()
        .await
        .context("failed to start read-only transaction")?;
    (&mut *txn)
        .execute(sqlx::raw_sql("SET TRANSACTION READ ONLY"))
        .await
        .context("failed to start read-only transaction")?;
    Ok(txn)
}

// ---------------------------------------------------------------------------
// 查询入口
// ---------------------------------------------------------------------------

/// 概览：聚合 + 最近列表 + 账户现值 + 余额曲线 + 山脊 + 活动流。
pub async fn get_simulation_overview(database_url: &str) -> Result<SimulationOverview> {
    let pool = ready_pool(database_url).await?;
    let mut txn = open_readonly(&pool).await?;

    let agg = sqlx::query(
        "SELECT
                 COUNT(*)::BIGINT,
                 COUNT(*) FILTER (WHERE scenario <> 'REJECTED')::BIGINT,
                 COALESCE(SUM(simulated_net_profit) FILTER (WHERE scenario <> 'REJECTED'), 0),
                 COALESCE(SUM(scanned_net_profit) FILTER (WHERE scenario <> 'REJECTED'), 0),
                 COUNT(*) FILTER (WHERE simulated_net_profit > 0)::BIGINT
             FROM simulation_runs",
    )
    .fetch_one(&mut *txn)
    .await
    .context("failed to aggregate simulation runs")?;
    let total_runs = to_u64(agg.try_get::<i64, _>(0)?)?;
    let total_success = to_u64(agg.try_get::<i64, _>(1)?)?;
    let total_net_profit = agg.try_get::<Decimal, _>(2)?;
    let scanned_net_profit = agg.try_get::<Decimal, _>(3)?;
    let profit_runs = to_u64(agg.try_get::<i64, _>(4)?)?;

    let scenario_rows =
        sqlx::query("SELECT scenario, COUNT(*)::BIGINT FROM simulation_runs GROUP BY scenario")
            .fetch_all(&mut *txn)
            .await
            .context("failed to load scenario distribution")?;
    let by_scenario = scenario_rows
        .iter()
        .map(|row| -> Result<ScenarioCount> {
            Ok(ScenarioCount {
                scenario: row.try_get(0)?,
                count: to_u64(row.try_get::<i64, _>(1)?).unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let runs = read_run_rows(&mut txn, "ORDER BY executed_at_ms DESC LIMIT 20", &[]).await?;

    // 前缀和由服务端一次算出（`) OVER (ORDER BY …)` 的默认 frame 即整个前缀），
    // 且只取最近 `CUMULATIVE_LIMIT` 个 run 后再升序返回：前端无需累加，也不再有
    // 无界传输量。
    let cumulative_rows = sqlx::query(
        "SELECT executed_at_ms, scanned, simulated FROM (
             SELECT executed_at_ms,
                    SUM(scanned_net_profit) OVER (ORDER BY executed_at_ms) AS scanned,
                    SUM(simulated_net_profit) OVER (ORDER BY executed_at_ms) AS simulated
             FROM simulation_runs
             ORDER BY executed_at_ms DESC
             LIMIT $1
         ) AS recent ORDER BY executed_at_ms",
    )
    .bind(CUMULATIVE_LIMIT)
    .fetch_all(&mut *txn)
    .await
    .context("failed to load cumulative profit points")?;
    let cumulative_points = cumulative_rows
        .iter()
        .map(|row| -> Result<NetProfitPoint> {
            Ok(NetProfitPoint {
                executed_at_ms: to_u64(row.try_get::<i64, _>(0)?).unwrap_or(0),
                scanned_net_profit: row.try_get(1)?,
                simulated_net_profit: row.try_get(2)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let balance_rows = sqlx::query(
        "SELECT account_id, venue, asset, observed_total, observed_free, local_reserved,
                    observed_at_ms
             FROM paper_balances WHERE account_id LIKE 'f02-account-%'
             ORDER BY account_id, venue, asset",
    )
    .fetch_all(&mut *txn)
    .await
    .context("failed to load simulation account balances")?;
    let account_balances = balance_rows
        .iter()
        .map(|row| -> Result<AccountSnapshot> {
            Ok(AccountSnapshot {
                account_id: row.try_get(0)?,
                venue: row.try_get(1)?,
                asset: row.try_get(2)?,
                observed_total: row.try_get(3)?,
                observed_free: row.try_get(4)?,
                local_reserved: row.try_get(5)?,
                observed_at_ms: to_u64(row.try_get::<i64, _>(6)?).unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let history_rows = sqlx::query(
        "SELECT snapshot_id, account_id, venue, asset, total, free, observed_at_ms
             FROM balance_snapshots WHERE source = 'SIMULATION'
             ORDER BY observed_at_ms DESC LIMIT $1",
    )
    .bind(BALANCE_HISTORY_LIMIT as i64)
    .fetch_all(&mut *txn)
    .await
    .context("failed to load simulation balance history")?;
    let mut balance_history: Vec<BalancePoint> = history_rows
        .iter()
        .map(|row| -> Result<BalancePoint> {
            let snapshot_id: String = row.try_get(0)?;
            let (run_id, node) = parse_sim_snapshot_id(&snapshot_id)
                .unwrap_or_else(|| ("".to_owned(), "unknown".to_owned()));
            Ok(BalancePoint {
                run_id,
                account_id: row.try_get(1)?,
                venue: row.try_get(2)?,
                asset: row.try_get(3)?,
                total: row.try_get(4)?,
                free: row.try_get(5)?,
                node,
                observed_at_ms: to_u64(row.try_get::<i64, _>(6)?).unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    balance_history.reverse();

    let ridge = load_ridge(&mut txn).await?;

    let activity_rows = sqlx::query(
        "SELECT occurred_at_ms, event_type, aggregate_id, correlation_id, payload
             FROM audit_events WHERE aggregate_id LIKE 'f02-%'
             ORDER BY occurred_at_ms DESC LIMIT 50",
    )
    .fetch_all(&mut *txn)
    .await
    .context("failed to load simulation activity")?;
    let activity = activity_rows
        .iter()
        .map(|row| -> Result<ActivityEvent> {
            let aggregate_id: String = row.try_get(2)?;
            Ok(ActivityEvent {
                occurred_at_ms: to_u64(row.try_get::<i64, _>(0)?).unwrap_or(0),
                event_type: row.try_get(1)?,
                aggregate_id: aggregate_id.clone(),
                correlation_id: row.try_get(3)?,
                payload: row.try_get(4)?,
                run_id: parse_f02_run_id(&aggregate_id),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let flow = load_flow_aggregate(&mut txn).await?;

    let _ = txn.rollback().await;
    Ok(SimulationOverview {
        total_runs,
        total_success,
        total_net_profit,
        scanned_net_profit,
        profit_runs,
        by_scenario,
        recent_runs: runs,
        cumulative_points,
        account_balances,
        balance_history,
        ridge,
        activity,
        flow,
    })
}

/// 分页列表（limit ≤ 200；symbol / scenario 可选过滤，不做任何写）。
pub async fn get_simulation_runs(
    database_url: &str,
    limit: u64,
    offset: u64,
    symbol: Option<String>,
    scenario: Option<String>,
) -> Result<SimulationRunsPage> {
    let limit = limit.min(200);
    let pool = ready_pool(database_url).await?;
    let mut txn = open_readonly(&pool).await?;

    let (filter, params): (String, Vec<(String, String)>) = build_run_filter(&symbol, &scenario);
    let filter_sql = if filter.is_empty() {
        String::new()
    } else {
        format!("WHERE {filter}")
    };
    let count_sql = format!("SELECT COUNT(*)::BIGINT FROM simulation_runs {filter_sql}");
    let mut count_query = sqlx::query(&count_sql);
    for (_, value) in &params {
        count_query = count_query.bind(value);
    }
    let count_row = count_query
        .fetch_one(&mut *txn)
        .await
        .context("failed to count simulation runs")?;
    let total = to_u64(count_row.try_get::<i64, _>(0)?)?;

    let order_sql = format!(
        "ORDER BY executed_at_ms DESC LIMIT {} OFFSET {}",
        limit,
        offset.min(1_000_000)
    );
    let runs = read_run_rows(&mut txn, &format!("{filter_sql} {order_sql}"), &params).await?;

    let _ = txn.rollback().await;
    Ok(SimulationRunsPage { total, runs })
}

/// 单 run 详情：投影行 + report 全文 + 意图/成交/执行事件/审计/余额快照。
pub async fn get_simulation_run_detail(
    database_url: &str,
    run_id: &str,
) -> Result<SimulationRunDetail> {
    let pool = ready_pool(database_url).await?;
    let mut txn = open_readonly(&pool).await?;

    // 单次取回投影列 + report + 三个关联标识（原为 3 次同表往返）。
    let sql = format!(
        "SELECT {RUN_PROJECTION}, report, plan_id, buy_intent_id, sell_intent_id, account_id
         FROM simulation_runs WHERE run_id = $1"
    );
    let row = sqlx::query(&sql)
        .bind(run_id)
        .fetch_optional(&mut *txn)
        .await
        .context("failed to load simulation run")?;
    let Some(row) = row else {
        let _ = txn.rollback().await;
        bail!("simulation run not found: {run_id}");
    };
    let run_row = run_row_at(&row, 0)?;
    let report: JsonValue = row.try_get(22)?;
    let plan_id: String = row.try_get(23)?;
    let buy_intent_id: String = row.try_get(24)?;
    let sell_intent_id: String = row.try_get(25)?;
    let account_id: String = row.try_get(26)?;

    let intents = read_intent_rows(&mut txn, &plan_id).await?;

    let trade_rows = sqlx::query(
        "SELECT venue, exchange_order_id, trade_id, intent_id, quantity, price,
                fee_asset, fee_amount, occurred_at_ms
             FROM trade_facts
             WHERE intent_id IN ($1, $2)
             ORDER BY occurred_at_ms",
    )
    .bind(&buy_intent_id)
    .bind(&sell_intent_id)
    .fetch_all(&mut *txn)
    .await
    .context("failed to load run trades")?;
    let trades = trade_rows
        .iter()
        .map(|row| -> Result<TradeFactRow> {
            Ok(TradeFactRow {
                venue: row.try_get(0)?,
                exchange_order_id: row.try_get(1)?,
                trade_id: row.try_get(2)?,
                intent_id: row.try_get(3)?,
                quantity: row.try_get(4)?,
                price: row.try_get(5)?,
                fee_asset: row.try_get(6)?,
                fee_amount: row.try_get(7)?,
                occurred_at_ms: to_u64(row.try_get::<i64, _>(8)?).unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let event_rows = sqlx::query(
        "SELECT event_id, plan_id, event_version, event_type, occurred_at_ms, payload
             FROM execution_events WHERE plan_id = $1 ORDER BY event_version",
    )
    .bind(&plan_id)
    .fetch_all(&mut *txn)
    .await
    .context("failed to load execution events")?;
    let execution_events = event_rows
        .iter()
        .map(|row| -> Result<ExecutionEventRow> {
            Ok(ExecutionEventRow {
                event_id: row.try_get(0)?,
                plan_id: row.try_get(1)?,
                event_version: row.try_get(2)?,
                event_type: row.try_get(3)?,
                occurred_at_ms: to_u64(row.try_get::<i64, _>(4)?).unwrap_or(0),
                payload: row.try_get(5)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let prefix = format!("{run_id}-%");
    let audit_rows = sqlx::query(
        "SELECT event_id, aggregate_id, aggregate_version, event_type, correlation_id,
                occurred_at_ms, payload
             FROM audit_events
             WHERE aggregate_id LIKE $1 OR correlation_id LIKE $1
             ORDER BY occurred_at_ms",
    )
    .bind(&prefix)
    .fetch_all(&mut *txn)
    .await
    .context("failed to load run audit events")?;
    let audit = audit_rows
        .iter()
        .map(|row| -> Result<AuditEventRow> {
            Ok(AuditEventRow {
                event_id: row.try_get(0)?,
                aggregate_id: row.try_get(1)?,
                aggregate_version: row.try_get(2)?,
                event_type: row.try_get(3)?,
                correlation_id: row.try_get(4)?,
                occurred_at_ms: to_u64(row.try_get::<i64, _>(5)?).unwrap_or(0),
                payload: row.try_get(6)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let balance_rows = sqlx::query(
        "SELECT snapshot_id, account_id, venue, asset, total, free, observed_at_ms
             FROM balance_snapshots
             WHERE account_id = $1 AND source = 'SIMULATION'
             ORDER BY observed_at_ms",
    )
    .bind(&account_id)
    .fetch_all(&mut *txn)
    .await
    .context("failed to load run balance snapshots")?;
    let balances = balance_rows
        .iter()
        .map(|row| -> Result<BalancePoint> {
            let snapshot_id: String = row.try_get(0)?;
            let (run, node) = parse_sim_snapshot_id(&snapshot_id)
                .unwrap_or_else(|| ("".to_owned(), "unknown".to_owned()));
            Ok(BalancePoint {
                run_id: run,
                account_id: row.try_get(1)?,
                venue: row.try_get(2)?,
                asset: row.try_get(3)?,
                total: row.try_get(4)?,
                free: row.try_get(5)?,
                node,
                observed_at_ms: to_u64(row.try_get::<i64, _>(6)?).unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let _ = txn.rollback().await;
    Ok(SimulationRunDetail {
        run: Box::new(run_row),
        report,
        intents,
        trades,
        execution_events,
        audit,
        balances,
    })
}

// ---------------------------------------------------------------------------
// 内部读取器
// ---------------------------------------------------------------------------

/// 构造 run 过滤条件（symbol 精确匹配 / scenario 精确匹配；参数位置化）。
fn build_run_filter(
    symbol: &Option<String>,
    scenario: &Option<String>,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let mut clauses = Vec::new();
    if let Some(symbol) = symbol {
        params.push(("symbol".to_owned(), symbol.clone()));
        clauses.push(format!("symbol = ${}", params.len()));
    }
    if let Some(scenario) = scenario {
        params.push(("scenario".to_owned(), scenario.clone()));
        clauses.push(format!("scenario = ${}", params.len()));
    }
    (clauses.join(" AND "), params)
}

/// `simulation_runs` 投影列清单（列表与详情共用，避免两处列清单漂移）。
const RUN_PROJECTION: &str =
    "run_id, executed_at_ms, evaluated_at_ms, symbol, buy_venue, sell_venue,
                direction, scenario, rejection_reason, quantity,
                bought_quantity, sold_quantity, buy_avg_price, sell_avg_price,
                buy_fee, sell_fee, scanned_net_profit, simulated_net_profit,
                compensation_decision, execution_state, idempotent_replay,
                external_order_calls";

/// 从 `base` 起读取 `RUN_PROJECTION` 的 22 列；详情查询在其后追加额外列，故带偏移。
fn run_row_at(row: &PgRow, base: usize) -> Result<SimulationRunRow> {
    Ok(SimulationRunRow {
        run_id: row.try_get(base)?,
        executed_at_ms: to_u64(row.try_get::<i64, _>(base + 1)?).unwrap_or(0),
        evaluated_at_ms: to_u64(row.try_get::<i64, _>(base + 2)?).unwrap_or(0),
        symbol: row.try_get(base + 3)?,
        buy_venue: row.try_get(base + 4)?,
        sell_venue: row.try_get(base + 5)?,
        direction: row.try_get(base + 6)?,
        scenario: row.try_get(base + 7)?,
        rejection_reason: row.try_get(base + 8)?,
        quantity: row.try_get(base + 9)?,
        bought_quantity: row.try_get(base + 10)?,
        sold_quantity: row.try_get(base + 11)?,
        buy_avg_price: row.try_get(base + 12)?,
        sell_avg_price: row.try_get(base + 13)?,
        buy_fee: row.try_get(base + 14)?,
        sell_fee: row.try_get(base + 15)?,
        scanned_net_profit: row.try_get(base + 16)?,
        simulated_net_profit: row.try_get(base + 17)?,
        compensation_decision: row.try_get(base + 18)?,
        execution_state: row.try_get(base + 19)?,
        idempotent_replay: row.try_get(base + 20)?,
        external_order_calls: to_u64(row.try_get::<i64, _>(base + 21)?).unwrap_or(0),
    })
}

/// 列表行读取（主营列；不读 `report` 列）。`where_sql` 支持 `$1..$n` 参数。
async fn read_run_rows(
    txn: &mut Transaction<'_, Postgres>,
    where_sql: &str,
    params: &[(String, String)],
) -> Result<Vec<SimulationRunRow>> {
    let sql = format!("SELECT {RUN_PROJECTION} FROM simulation_runs {where_sql}");
    let mut query = sqlx::query(&sql);
    for (_, value) in params {
        query = query.bind(value);
    }
    let rows = query
        .fetch_all(&mut **txn)
        .await
        .context("failed to read simulation runs")?;
    rows.iter().map(|row| run_row_at(row, 0)).collect()
}

/// 双腿意图 + order_facts + order_action_facts（一次意图查询 + 一次动作查询，Rust 端 join）。
async fn read_intent_rows(
    txn: &mut Transaction<'_, Postgres>,
    plan_id: &str,
) -> Result<Vec<IntentFactRow>> {
    let intent_rows = sqlx::query(
        "SELECT oi.intent_id, oi.leg_id, oi.client_order_id, oi.venue, oi.side,
                oi.quantity, oi.limit_price, oi.created_at_ms,
                of.submission_status, of.cancel_status, of.reconciliation_status,
                of.filled_quantity, of.exchange_order_id
             FROM order_intents oi
             LEFT JOIN order_facts of ON of.intent_id = oi.intent_id
             WHERE oi.plan_id = $1
             ORDER BY oi.leg_id",
    )
    .bind(plan_id)
    .fetch_all(&mut **txn)
    .await
    .context("failed to load run intents")?;
    let mut intents: Vec<IntentFactRow> = intent_rows
        .iter()
        .map(|row| -> Result<IntentFactRow> {
            Ok(IntentFactRow {
                intent_id: row.try_get(0)?,
                leg_id: row.try_get(1)?,
                client_order_id: row.try_get(2)?,
                venue: row.try_get(3)?,
                side: row.try_get(4)?,
                quantity: row.try_get(5)?,
                limit_price: row.try_get(6)?,
                created_at_ms: to_u64(row.try_get::<i64, _>(7)?).unwrap_or(0),
                submission_status: row.try_get(8)?,
                cancel_status: row.try_get(9)?,
                reconciliation_status: row.try_get(10)?,
                filled_quantity: row.try_get(11)?,
                exchange_order_id: row.try_get(12)?,
                actions: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let intent_ids: Vec<String> = intents.iter().map(|i| i.intent_id.clone()).collect();
    if !intent_ids.is_empty() {
        let action_rows = sqlx::query(
            "SELECT intent_id, fact_id, fact_version, action, occurred_at_ms, payload
             FROM order_action_facts
             WHERE intent_id = ANY($1)
             ORDER BY intent_id, fact_version",
        )
        .bind(&intent_ids)
        .fetch_all(&mut **txn)
        .await
        .context("failed to load order action facts")?;
        for row in action_rows {
            let intent_id: String = row.try_get(0)?;
            if let Some(intent) = intents.iter_mut().find(|i| i.intent_id == intent_id) {
                intent.actions.push(ActionFactRow {
                    fact_id: row.try_get(1)?,
                    fact_version: row.try_get(2)?,
                    action: row.try_get(3)?,
                    occurred_at_ms: to_u64(row.try_get::<i64, _>(4)?).unwrap_or(0),
                    payload: row.try_get(5)?,
                });
            }
        }
    }
    Ok(intents)
}

/// 山脊读取上限。每桶 KDE 成本为 O(桶内点数 × 56)，无界增长会同时放大
/// 每 10s 轮询的传输量与前端重算成本。
const RIDGE_LIMIT: i64 = 4000;

/// 山脊数据：非 Rejected run 按日桶聚合净盈亏数组（桶内 <8 点自动跨日合并）。
/// 只取最近 `RIDGE_LIMIT` 个 run——山脊图展示的是近期分布形状，历史桶早已被合并。
async fn load_ridge(txn: &mut Transaction<'_, Postgres>) -> Result<Vec<RidgeBucket>> {
    let rows = sqlx::query(
        "SELECT executed_at_ms, simulated_net_profit FROM (
             SELECT executed_at_ms, simulated_net_profit
             FROM simulation_runs WHERE scenario <> 'REJECTED'
             ORDER BY executed_at_ms DESC LIMIT $1
         ) AS recent ORDER BY executed_at_ms",
    )
    .bind(RIDGE_LIMIT)
    .fetch_all(&mut **txn)
    .await
    .context("failed to load ridge data")?;
    let mut buckets: BTreeMap<u64, Vec<Decimal>> = BTreeMap::new();
    for row in rows {
        let executed_at_ms = to_u64(row.try_get::<i64, _>(0)?).unwrap_or(0);
        let day = executed_at_ms / DAY_MS;
        let bucket_start = day * DAY_MS;
        buckets
            .entry(bucket_start)
            .or_default()
            .push(row.try_get(1)?);
    }
    let ridge = merge_sparse_buckets(buckets);
    Ok(ridge.into_iter().map(Into::into).collect())
}

/// 稀疏桶合并（完全确定、无随机）：不足 8 点的桶并入其前一个桶，直到不足 8 点者只剩
/// 最老一桶（保留历史端点信息）。返回按时间升序的 `(bucket_start_ms, nets)`。
fn merge_sparse_buckets(buckets: BTreeMap<u64, Vec<Decimal>>) -> Vec<(u64, Vec<Decimal>)> {
    let mut out: Vec<(u64, Vec<Decimal>)> = Vec::with_capacity(buckets.len());
    for (start, mut nets) in buckets {
        if nets.len() < 8
            && let Some((_, prev)) = out.last_mut()
        {
            prev.extend(nets);
            continue;
        }
        nets.shrink_to_fit();
        out.push((start, nets));
    }
    out
}

impl From<(u64, Vec<Decimal>)> for RidgeBucket {
    fn from((bucket_start_ms, nets): (u64, Vec<Decimal>)) -> Self {
        RidgeBucket {
            bucket_start_ms,
            bucket_end_ms: bucket_start_ms + DAY_MS,
            nets,
        }
    }
}

/// 执行流转图聚合：EVALUATED（非 Rejected run）/ 预留（PlanReserved 审计去重）/ 双腿成交 /
/// 补偿评估 / COMPLETED / MANUAL_ESCALATED，全部自不可变事件表计数。
async fn load_flow_aggregate(txn: &mut Transaction<'_, Postgres>) -> Result<FlowAggregate> {
    // 五个计数合并为一条聚合；三个 `event_type` 过滤共用一个顺序扫描。
    let events = sqlx::query(
        "SELECT
             count(DISTINCT plan_id) FILTER (WHERE event_type = 'COMPENSATION_DECIDED')::BIGINT,
             count(DISTINCT plan_id) FILTER (WHERE event_type = 'COMPLETED')::BIGINT,
             count(DISTINCT plan_id) FILTER (WHERE event_type = 'MANUAL_ESCALATED')::BIGINT
         FROM execution_events WHERE plan_id LIKE 'f02-%'",
    )
    .fetch_one(&mut **txn)
    .await
    .context("failed to aggregate execution events")?;
    let compensated: i64 = events.try_get(0)?;
    let completed: i64 = events.try_get(1)?;
    let escalated: i64 = events.try_get(2)?;

    // EVALUATED（非 Rejected run）、预留（PlanReserved 审计去重）与双腿成交。
    // `filled_runs` 必须反映真实成交：`COMPLETED` 事件只表示两腿数量相等
    // （execution.rs 的 `mismatch == ZERO` 分支），双腿零成交的 run 也会是
    // COMPLETED，因此按 `simulation_runs` 的实际成交量为准。
    let flows = sqlx::query(
        "SELECT
             (SELECT count(*) FROM simulation_runs WHERE scenario <> 'REJECTED')::BIGINT,
             (SELECT count(DISTINCT aggregate_id) FROM audit_events
                  WHERE event_type = 'PlanReserved' AND aggregate_id LIKE 'f02-%')::BIGINT,
             (SELECT count(*) FROM simulation_runs
                  WHERE bought_quantity > 0 AND sold_quantity > 0)::BIGINT",
    )
    .fetch_one(&mut **txn)
    .await
    .context("failed to aggregate run flow")?;
    let evaluated: i64 = flows.try_get(0)?;
    let reserved: i64 = flows.try_get(1)?;
    let filled: i64 = flows.try_get(2)?;

    Ok(FlowAggregate {
        evaluated_runs: to_u64(evaluated).unwrap_or(0),
        reserved_runs: to_u64(reserved).unwrap_or(0),
        filled_runs: to_u64(filled).unwrap_or(0),
        compensated_runs: to_u64(compensated).unwrap_or(0),
        completed_runs: to_u64(completed).unwrap_or(0),
        escalated_runs: to_u64(escalated).unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// run 归属解析（设计 §2.3）
// ---------------------------------------------------------------------------

/// 解析 `sim-snap-f02-{ts}-{pid}-{dir}-{node}-{asset}`（8 段固定）→ `(run_id, node)`。
fn parse_sim_snapshot_id(snapshot_id: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = snapshot_id.split('-').collect();
    if parts.len() != 8 || parts[0] != "sim" || parts[1] != "snap" {
        return None;
    }
    Some((
        format!("{}-{}-{}", parts[2], parts[3], parts[4]),
        parts[6].to_owned(),
    ))
}

/// 从 `f02-{ts}-{pid}-…` 聚合标识解析 run_id（前 3 段 join；ts/pid 无 `-`）。
fn parse_f02_run_id(aggregate_id: &str) -> Option<String> {
    let parts: Vec<&str> = aggregate_id.split('-').collect();
    if parts.len() < 3 || parts[0] != "f02" {
        return None;
    }
    Some(format!("{}-{}-{}", parts[0], parts[1], parts[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    #[test]
    fn parse_sim_snapshot_id_extracts_run_and_node() {
        let (run, node) =
            parse_sim_snapshot_id("sim-snap-f02-1720000000000-4242-a-evaluated-quote")
                .expect("snapshot id should parse");
        assert_eq!(run, "f02-1720000000000-4242");
        assert_eq!(node, "evaluated");
        // 非本格式：空/乱序 → None。
        assert!(parse_sim_snapshot_id("audit-1720000000000").is_none());
        assert!(parse_sim_snapshot_id("").is_none());
    }

    #[test]
    fn parse_f02_run_id_takes_first_three_segments() {
        let run = parse_f02_run_id("f02-1720000000000-4242-plan-a")
            .expect("audit aggregate should parse");
        assert_eq!(run, "f02-1720000000000-4242");
        assert_eq!(run, "f02-1720000000000-4242");
        // 非 f02 前缀（真实域审计）→ None，活动日志正确留空 run 归属。
        assert!(parse_f02_run_id("b03-exec-1").is_none());
    }

    #[test]
    fn merge_sparse_buckets_merges_into_previous() {
        let mut buckets = BTreeMap::new();
        let mut fill = |day: u64, n: usize| {
            buckets.insert(day * DAY_MS, vec![dec("1"); n]);
        };
        // 第 0 天 10 点（不合并）；第 1 天 3 点（并入前桶 → 13）；第 2 天 7 点（并入 → 20）。
        fill(0, 10);
        fill(1, 3);
        fill(2, 7);
        let merged = merge_sparse_buckets(buckets);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].0, 0);
        assert_eq!(merged[0].1.len(), 20);
        // 新的满桶出现后不再合并。
        let mut second = BTreeMap::new();
        second.insert(0, vec![dec("1"); 10]);
        second.insert(DAY_MS, vec![dec("2"); 3]);
        second.insert(2 * DAY_MS, vec![dec("3"); 9]);
        let merged = merge_sparse_buckets(second);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].1.len(), 13);
        assert_eq!(merged[1].1.len(), 9);
        assert_eq!(merged[1].0, 2 * DAY_MS);
    }

    #[test]
    fn ridge_bucket_from_tuple_computes_end_exclusive() {
        let bucket: RidgeBucket = (1720000000000 / DAY_MS * DAY_MS, vec![dec("1.5")]).into();
        assert_eq!(bucket.bucket_end_ms - bucket.bucket_start_ms, DAY_MS);
    }

    #[test]
    fn overview_dto_round_trips_with_decimal_strings() {
        let overview = SimulationOverview {
            total_runs: 3,
            total_success: 2,
            total_net_profit: dec("12.500"),
            scanned_net_profit: dec("13.000"),
            profit_runs: 2,
            by_scenario: vec![ScenarioCount {
                scenario: "NORMAL".into(),
                count: 2,
            }],
            recent_runs: Vec::new(),
            cumulative_points: vec![NetProfitPoint {
                executed_at_ms: 1720000000000,
                scanned_net_profit: dec("13.000"),
                simulated_net_profit: dec("12.500"),
            }],
            account_balances: Vec::new(),
            balance_history: Vec::new(),
            ridge: Vec::new(),
            activity: Vec::new(),
            flow: FlowAggregate {
                evaluated_runs: 2,
                reserved_runs: 2,
                filled_runs: 2,
                compensated_runs: 1,
                completed_runs: 2,
                escalated_runs: 0,
            },
        };
        let json = serde_json::to_value(&overview).unwrap();
        assert_eq!(json["total_net_profit"], "12.500");
        assert_eq!(json["by_scenario"][0]["scenario"], "NORMAL");
        assert_eq!(json["flow"]["compensated_runs"], 1);
    }
}
