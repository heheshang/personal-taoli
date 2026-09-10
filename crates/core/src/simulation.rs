//! F-02 模拟撮合引擎（真实场景仿真执行）。
//!
//! 真实行情 → 决策（observer 连续模式）→ 本引擎撮合 → PAPER 事实落库 → 仿真报告。
//! 撮合输入只用 run 启动时的事件克隆快照；全程不产生真实/测试网订单，
//! `external_order_calls = 0` 恒真（AIDLC 安全边界）。
//!
//! - 成交价模型（D2）：逐档消费至目标量或深度耗尽；每档有效量 =
//!   档量 × (1 − competitor_take_bps/10000) 按 quantity_step 向下圆整；档价施加
//!   adverse 偏移（买腿 ×(1+bps) 向上圆整到 tick、卖腿 ×(1−bps) 向下圆整）。
//!   扫描最差价仅作滑点对比基线，不作成交价。
//! - 时延采样（D3）：自写 splitmix64 `SimRng`，seed=0 时按运行时间播种并回写报告，
//!   非 0 时全链路确定性可复现。
//! - 连续模式（D4）：`SimulationEngine` 单槽最新优先，积压跳帧只处理最新机会。

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio::sync::{Mutex, Notify};

use crate::archive::DecisionEvent;
use crate::config::SimulationConfig;
use crate::db::SCHEMA_VERSION;
use crate::db::{hex_digest, migrate, pool, ready_pool};
use crate::execution::{CompensationDecision, ExecutionCore, ExecutionState};
use crate::instrument::InstrumentSpec;
use crate::market::{BookSide, Level, OrderBookSnapshot, unix_timestamp_ms};
use crate::order::{OrderCore, QueryResult, SubmitResult, TradeInput};
use crate::paper::{
    OrderIntentInput, OrderSide, PaperCore, ReservationInput, ReservePlanRequest, set_paper_balance,
};
use crate::scan::Opportunity;

/// F-02 仿真执行策略配置版本（写入 PAPER 事实的 strategy_config_version）。
pub const SIMULATION_CONFIG_VERSION: &str = "f02-sim-v1";

const BPS_DENOMINATOR: &str = "10000";

fn bps_decimal() -> Decimal {
    BPS_DENOMINATOR.parse().expect("BPS_DENOMINATOR literal")
}

/// splitmix64 确定性随机源：无外部依赖；同 seed 全链路线性消费即完全复现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// 标准 splitmix64 初始化（0 种子混合后仍产生非零状态）。
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [min, max] 闭区间均匀整数采样（含两端）。要求 min <= max。
    pub fn gen_range_u64(&mut self, min: u64, max: u64) -> u64 {
        debug_assert!(min <= max);
        let span = u128::from(max - min) + 1;
        min + (u128::from(self.next_u64()) % span) as u64
    }
}

/// 施加 adverse 偏移的档价：买侧 ×(1+bps) 向上圆整到 tick（宁可多付），
/// 卖侧 ×(1−bps) 向下圆整到 tick（宁可少收）。price 必须为正。
fn offset_level_price(
    price: Decimal,
    side: BookSide,
    bps: &Decimal,
    tick: &Decimal,
) -> Result<Decimal> {
    if price <= Decimal::ZERO {
        bail!("simulation offset price must be positive");
    }
    let factor = match side {
        BookSide::Asks => Decimal::ONE + bps / bps_decimal(),
        BookSide::Bids => Decimal::ONE - bps / bps_decimal(),
    };
    let raw = price * factor;
    let rounding = match side {
        BookSide::Asks => RoundingStrategy::AwayFromZero,
        BookSide::Bids => RoundingStrategy::ToZero,
    };
    let ticks = (raw / *tick).round_dp_with_strategy(0, rounding);
    Ok(ticks * *tick)
}

/// 单腿撮合结果（全程 Decimal，禁止转浮点）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimLegResult {
    pub filled: Decimal,
    /// 成交名义金额（Σ 档量 × 偏移后档价）。
    pub quote_amount: Decimal,
    /// 偏移后加权均价；无成交时为 0。
    pub avg_price: Decimal,
    /// 最后消费档位的原始档价（滑点对比基线，未施偏移；无成交时为 0）。
    pub last_raw_price: Decimal,
    /// 最后消费档位的成交价（已施偏移）。
    pub last_fill_price: Decimal,
    /// 深度不足（部分成交且未达目标量）。
    pub depth_shortfall: bool,
    /// 圆整后得量低于 min_quantity 而整腿拒绝（无成交）。
    pub below_min_quantity: bool,
}

/// 逐档撮合单腿（D2 成交价模型）。买侧消费 asks（升序）、卖侧消费 bids（降序）。
///
/// 每档有效量 = 档量 × (1 − competitor_take_bps/10000)，按 quantity_step 向下圆整；
/// 档价施加 adverse 偏移并圆整到 price_tick。深度不足 → 部分成交 + depth_shortfall；
/// 首档可得 0 或簿空 → filled = 0（敌手吞单/无对手盘）。
pub fn simulate_leg(
    side: BookSide,
    book: &OrderBookSnapshot,
    instrument: &InstrumentSpec,
    requested: Decimal,
    config: &SimulationConfig,
) -> Result<SimLegResult> {
    if requested <= Decimal::ZERO {
        bail!("simulate_leg requested quantity must be positive");
    }
    let take_rate = Decimal::ONE - config.competitor_take_bps / bps_decimal();
    let levels: &[Level] = match side {
        BookSide::Bids => &book.bids,
        BookSide::Asks => &book.asks,
    };
    let step = instrument.quantity_step;
    let mut remaining = requested;
    let mut filled = Decimal::ZERO;
    let mut quote_amount = Decimal::ZERO;
    let mut last_raw_price = Decimal::ZERO;
    let mut last_fill_price = Decimal::ZERO;
    for level in levels {
        if remaining == Decimal::ZERO {
            break;
        }
        let effective = level.quantity * take_rate;
        let avail = (effective / step).floor() * step;
        if avail <= Decimal::ZERO {
            continue;
        }
        let take = remaining.min(avail);
        let fill_price = offset_level_price(
            level.price,
            side,
            &config.adverse_move_bps,
            &instrument.price_tick,
        )?;
        quote_amount += take * fill_price;
        filled += take;
        remaining -= take;
        last_raw_price = level.price;
        last_fill_price = fill_price;
    }
    let depth_shortfall = remaining > Decimal::ZERO && filled > Decimal::ZERO;
    let below_min_quantity = filled > Decimal::ZERO && filled < instrument.min_quantity;
    let filled = if below_min_quantity {
        Decimal::ZERO
    } else {
        filled
    };
    let avg_price = if filled > Decimal::ZERO {
        quote_amount / filled
    } else {
        Decimal::ZERO
    };
    Ok(SimLegResult {
        filled,
        quote_amount,
        avg_price,
        last_raw_price,
        last_fill_price,
        depth_shortfall,
        below_min_quantity,
    })
}

/// 模拟执行场景分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SimulationScenario {
    /// 双腿足额成交、无不利标记。
    Normal,
    /// 至少一腿深度不足，部分成交。
    DepthShortfall,
    /// 至少一腿完全没有成交（敌手吞单/无对手盘）。
    CompetedAway,
    /// 资金不足等前置校验拒绝，未产生任何事实。
    Rejected,
}

/// 单方向模拟执行 run 的完整报告（命令层/前端序列化用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimulationRunReport {
    pub schema_version: i64,
    /// 实际生效种子（config.seed=0 时按运行时间播种并回写）。
    pub seed: u64,
    /// 本次 run 的命名空间根：`f02-{unix_timestamp_ms}-{pid}`。
    pub run_id: String,
    pub symbol: String,
    pub quantity: Decimal,
    pub buy_venue: String,
    pub sell_venue: String,
    pub scenario: SimulationScenario,
    /// scenario=Rejected 时的原因。
    pub rejection_reason: Option<String>,
    pub plan_id: String,
    pub account_id: String,
    pub instrument_id: String,
    pub buy_intent_id: String,
    pub sell_intent_id: String,
    pub buy_client_order_id: String,
    pub sell_client_order_id: String,
    pub evaluated_at_ms: u64,
    /// 决策→提交时延采样（毫秒，来自配置区间；真实时延由观测方另行统计）。
    pub decision_to_submit_ms: u64,
    /// 提交→成交回报时延采样（毫秒）。
    pub fill_latency_ms: u64,
    pub buy: SimLegResult,
    pub sell: SimLegResult,
    /// 两腿费用（统一以报价资产计，见设计 §18.2；基础资产计费残差明确不做）。
    pub buy_fee: Decimal,
    pub sell_fee: Decimal,
    /// 扫码预期净盈亏（replay 基准）。
    pub scanned_net_profit: Decimal,
    /// 模拟净盈亏 = 卖腿收入 − 买腿成本 − 两腿费用。
    pub simulated_net_profit: Decimal,
    pub adverse_move_bps: Decimal,
    pub competitor_take_bps: Decimal,
    /// B-03 补偿决策：NO_ACTION / COMPENSATION_PLANNED / MANUAL_REQUIRED。
    pub compensation_decision: String,
    /// B-03 执行快照终态（仅成功 run 有实际事实；Rejected 时为占位）。
    pub execution_state: String,
    pub unmatched_quantity: Decimal,
    pub unmatched_exposure: Decimal,
    pub estimated_compensation_cost: Decimal,
    /// 任意一腿提交落入 UNKNOWN（模拟传输超时）。
    pub unknown_submit_tried: bool,
    /// UNKNOWN 后查询 Found（订单身份恢复）。
    pub query_found: bool,
    /// 同一 request 二次 reserve_plan 的幂等复放标记。
    pub idempotent_replay: bool,
    /// 连续模式积压跳帧数（命令层手动 run 为 0）。
    pub skipped_frames: u64,
    pub warnings: Vec<String>,
    /// 恒为 0：模拟层不产生任何真实/测试网订单（AIDLC 安全边界）。
    pub external_order_calls: usize,
}

enum SubmitOutcome {
    /// 注单身份被确认（直接 Accepted 或 UNKNOWN 后查询 Found）。
    Accepted { exchange_order_id: String },
    /// 提交 UNKNOWN 且查询 NotFound：订单身份不可恢复，不记录成交。
    UnknownUnrecoverable,
}

/// 节点日志用短标签：`accepted:{id}` | `unknown:not_found`。
fn outcome_tag(outcome: &SubmitOutcome) -> String {
    match outcome {
        SubmitOutcome::Accepted { exchange_order_id } => {
            format!("accepted:{exchange_order_id}")
        }
        SubmitOutcome::UnknownUnrecoverable => "unknown:not_found".to_owned(),
    }
}

/// 提交→确认子流程：digest 提交、按 unknown_submit_probability_bps 分支到
/// UNKNOWN（随后按 query_found_probability_bps 查询 Found/NotFound）、否则直接 Accepted。
async fn submit_and_confirm(
    order: &mut OrderCore,
    run_id: &str,
    intent_id: &str,
    leg: &str,
    evaluated_at_ms: u64,
    rng: &mut SimRng,
    config: &SimulationConfig,
) -> Result<(SubmitOutcome, bool, bool)> {
    let digest =
        hex_digest(Sha256::digest(format!("f02:{run_id}:{intent_id}").as_bytes()).as_slice());
    order.ensure_intent(intent_id).await?;
    order.submit_started(intent_id, &digest).await?;
    let unknown_prob = bps_to_u64(&config.unknown_submit_probability_bps)?;
    let found_prob = bps_to_u64(&config.query_found_probability_bps)?;
    let query_id = format!("{run_id}-{leg}-query");
    if rng.gen_range_u64(0, 10_000) < unknown_prob {
        order
            .submit_result(
                intent_id,
                SubmitResult::Unknown {
                    reason: "simulated transport timeout".to_owned(),
                },
            )
            .await?;
        if rng.gen_range_u64(0, 10_000) < found_prob {
            let exchange_order_id = format!("{run_id}-{leg}-exc");
            order
                .query_result(
                    intent_id,
                    QueryResult::Found {
                        query_id,
                        exchange_order_id: exchange_order_id.clone(),
                    },
                )
                .await?;
            Ok((SubmitOutcome::Accepted { exchange_order_id }, true, true))
        } else {
            order
                .query_result(
                    intent_id,
                    QueryResult::NotFound {
                        query_id,
                        visibility_deadline_ms: evaluated_at_ms + 60_000,
                    },
                )
                .await?;
            Ok((SubmitOutcome::UnknownUnrecoverable, true, false))
        }
    } else {
        let exchange_order_id = format!("{run_id}-{leg}-exc");
        order
            .submit_result(
                intent_id,
                SubmitResult::Accepted {
                    exchange_order_id: exchange_order_id.clone(),
                },
            )
            .await?;
        Ok((SubmitOutcome::Accepted { exchange_order_id }, false, false))
    }
}

fn bps_to_u64(bps: &Decimal) -> Result<u64> {
    let value = bps
        .round()
        .to_i64()
        .context("simulation bps field exceeds i64")?;
    u64::try_from(value).context("simulation bps field is negative")
}

/// 读取 PAPER 自由余额（observed_free − local_reserved）；无记录视为 0。
async fn available_balance(
    database_url: &str,
    account: &str,
    venue: &str,
    asset: &str,
) -> Result<Decimal> {
    let pool = ready_pool(database_url).await?;
    let row = sqlx::query(
        "SELECT observed_free, local_reserved FROM paper_balances
             WHERE account_id=$1 AND venue=$2 AND asset=$3",
    )
    .bind(account)
    .bind(venue)
    .bind(asset)
    .fetch_optional(&pool)
    .await
    .context("failed to read PAPER balance")?;
    Ok(match row {
        Some(r) => r.try_get::<Decimal, _>(0)? - r.try_get::<Decimal, _>(1)?,
        None => Decimal::ZERO,
    })
}

/// G-01 三节点结算后视图推演（设计 D5，纯逻辑）：返回 `[(quote_total, base_total); 3]`，
/// 依次为 fund（注入额）/ filled（注入 − 买腿成本/卖腿交割）/ evaluated（同 filled）。
/// 推演值 clamp >= 0 兜底 `balance_snapshots.total >= 0`（模拟撮合超支极端情况）。
fn settlement_points(
    report: &SimulationRunReport,
    quote_initial: Decimal,
    base_initial: Decimal,
) -> [(Decimal, Decimal); 3] {
    let quote_filled =
        (quote_initial - report.buy.quote_amount - report.buy_fee).max(Decimal::ZERO);
    let base_filled = (base_initial - report.sell.filled).max(Decimal::ZERO);
    [
        (quote_initial, base_initial),
        (quote_filled, base_filled),
        (quote_filled, base_filled),
    ]
}

/// G-01 投影列 `scenario` 值域映射（与 0005 CHECK 字面量一致，纯逻辑）。
fn scenario_db_value(scenario: SimulationScenario) -> &'static str {
    match scenario {
        SimulationScenario::Normal => "NORMAL",
        SimulationScenario::DepthShortfall => "DEPTH_SHORTFALL",
        SimulationScenario::CompetedAway => "COMPETED_AWAY",
        SimulationScenario::Rejected => "REJECTED",
    }
}

/// G-01 落库钩子（设计 D2/D5）：run 完成后写投影 + 余额快照。
///
/// - `simulation_runs` 投影行持久化失败 → **bail**（run 报告即产物，报告丢失即 run 失败；
///   `ON CONFLICT(run_id) DO NOTHING` 保证 replay 不覆盖首跑投影）；
/// - 三节点余额快照（fund/filled/evaluated × 每 run 两账户）写失败 → **仅 tracing 告警**，
///   不 bail（快照是展示层冗余投影，不绑架 run 语义）。
///
/// 快照为「模拟结算后视图」（record_trade 不写影子余额 → 从注入额推演成交后余额，
/// `source='SIMULATION'` 与对账快照严格区分）；推演值 clamp 到 0 兜底
/// `balance_snapshots.total >= 0` CHECK（模拟撮合超支极端情况的诚实呈现边界）。
async fn persist_simulation_run(
    database_url: &str,
    report: &SimulationRunReport,
    config: &SimulationConfig,
    quote_asset: &str,
    base_asset: &str,
    direction: &str,
    fund_observed_at_ms: u64,
) -> Result<()> {
    use crate::db::to_i64;

    let pool = pool(database_url).await?;
    let report_json =
        serde_json::to_value(report).context("failed to serialize simulation run report")?;
    let warnings_json =
        serde_json::to_value(&report.warnings).context("failed to serialize run warnings")?;
    let scenario = scenario_db_value(report.scenario);
    let rejection_reason = report.rejection_reason.as_deref();
    // 安全边界：模拟层无真实外呼路径，投影恒 0（既有无烟测断言把关）。
    debug_assert_eq!(report.external_order_calls, 0);
    sqlx::query(
        "INSERT INTO simulation_runs(
                 run_id, executed_at_ms, evaluated_at_ms, symbol, buy_venue, sell_venue,
                 direction, quantity, scenario, rejection_reason, planning_warnings,
                 plan_id, account_id, instrument_id, buy_intent_id, sell_intent_id,
                 seed, decision_to_submit_ms, fill_latency_ms,
                 bought_quantity, sold_quantity, buy_avg_price, sell_avg_price,
                 buy_fee, sell_fee, scanned_net_profit, simulated_net_profit,
                 adverse_move_bps, competitor_take_bps,
                 compensation_decision, execution_state, unmatched_quantity,
                 unmatched_exposure, estimated_compensation_cost,
                 unknown_submit_tried, query_found, idempotent_replay,
                 skipped_frames, external_order_calls, report
             ) VALUES(
                 $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                 $17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,
                 $31,$32,$33,$34,$35,$36,$37,$38,$39,$40
             ) ON CONFLICT (run_id) DO NOTHING",
    )
    .bind(&report.run_id)
    .bind(to_i64(fund_observed_at_ms)?)
    .bind(to_i64(report.evaluated_at_ms)?)
    .bind(&report.symbol)
    .bind(&report.buy_venue)
    .bind(&report.sell_venue)
    .bind(direction)
    .bind(report.quantity)
    .bind(scenario)
    .bind(rejection_reason)
    .bind(&warnings_json)
    .bind(&report.plan_id)
    .bind(&report.account_id)
    .bind(&report.instrument_id)
    .bind(&report.buy_intent_id)
    .bind(&report.sell_intent_id)
    .bind(to_i64(report.seed)?)
    .bind(to_i64(report.decision_to_submit_ms)?)
    .bind(to_i64(report.fill_latency_ms)?)
    .bind(report.buy.filled)
    .bind(report.sell.filled)
    .bind(report.buy.avg_price)
    .bind(report.sell.avg_price)
    .bind(report.buy_fee)
    .bind(report.sell_fee)
    .bind(report.scanned_net_profit)
    .bind(report.simulated_net_profit)
    .bind(report.adverse_move_bps)
    .bind(report.competitor_take_bps)
    .bind(&report.compensation_decision)
    .bind(&report.execution_state)
    .bind(report.unmatched_quantity)
    .bind(report.unmatched_exposure)
    .bind(report.estimated_compensation_cost)
    .bind(report.unknown_submit_tried)
    .bind(report.query_found)
    .bind(report.idempotent_replay)
    .bind(to_i64(report.skipped_frames)?)
    .bind(0_i64)
    .bind(&report_json)
    .execute(&pool)
    .await
    .context("failed to persist simulation run projection")?;

    // 三节点结算后视图（设计 D5）。fund=注入额；filled=注入 − 买腿成本/卖腿交割；
    // evaluated=同 filled（补偿为现金评估、不落地）。推演值 clamp>=0 见 settlement_points。
    let filled_at_ms = report
        .evaluated_at_ms
        .saturating_add(report.decision_to_submit_ms)
        .saturating_add(report.fill_latency_ms);
    let evaluated_now_ms = unix_timestamp_ms()?;
    let [fund, filled, evaluated] = settlement_points(
        report,
        config.initial_quote_balance,
        config.initial_base_balance,
    );
    let nodes: [(String, i64, Decimal, Decimal); 3] = [
        (
            "fund".to_owned(),
            to_i64(fund_observed_at_ms)?,
            fund.0,
            fund.1,
        ),
        (
            "filled".to_owned(),
            to_i64(filled_at_ms)?,
            filled.0,
            filled.1,
        ),
        (
            "evaluated".to_owned(),
            to_i64(evaluated_now_ms)?,
            evaluated.0,
            evaluated.1,
        ),
    ];
    for (node, at_ms, quote_total, base_total) in nodes {
        let quote_snapshot_id = format!("sim-snap-{}-{direction}-{node}-quote", report.run_id);
        let base_snapshot_id = format!("sim-snap-{}-{direction}-{node}-base", report.run_id);
        let quote_insert = sqlx::query(
            "INSERT INTO balance_snapshots(
                         snapshot_id, account_id, venue, asset, total, free, locked,
                         observed_at_ms, source, completeness
                     ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(&quote_snapshot_id)
        .bind(&report.account_id)
        .bind(&report.buy_venue)
        .bind(quote_asset)
        .bind(quote_total)
        .bind(quote_total)
        .bind(Decimal::ZERO)
        .bind(at_ms)
        .bind("SIMULATION")
        .bind("COMPLETE")
        .execute(&pool)
        .await;
        if let Err(error) = quote_insert {
            tracing::warn!(
                "G-01: balance snapshot failed for run {} node {node} quote: {error:#}",
                report.run_id
            );
        }
        let base_insert = sqlx::query(
            "INSERT INTO balance_snapshots(
                         snapshot_id, account_id, venue, asset, total, free, locked,
                         observed_at_ms, source, completeness
                     ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(&base_snapshot_id)
        .bind(&report.account_id)
        .bind(&report.sell_venue)
        .bind(base_asset)
        .bind(base_total)
        .bind(base_total)
        .bind(Decimal::ZERO)
        .bind(at_ms)
        .bind("SIMULATION")
        .bind("COMPLETE")
        .execute(&pool)
        .await;
        if let Err(error) = base_insert {
            tracing::warn!(
                "G-01: balance snapshot failed for run {} node {node} base: {error:#}",
                report.run_id
            );
        }
    }
    Ok(())
}

/// 单方向模拟执行 run：真实行情快照 → PAPER 事实落库 → 仿真报告。
///
/// 幂等/失败语义：资金不足 → 返回 `Rejected` 报告（无任何事实残留）而非 bail；
/// 其余 DB/参数错误一律失败关闭（bail）。
pub async fn run_simulation_run(
    database_url: &str,
    event: &DecisionEvent,
    opportunity: &Opportunity,
    config: &SimulationConfig,
    skipped_frames: u64,
) -> Result<SimulationRunReport> {
    migrate(database_url).await?;
    let evaluated_at_ms = event.evaluated_at_ms;
    let now = unix_timestamp_ms()?;
    let stamp = format!("{now}-{}", std::process::id());
    let run_id = format!("f02-{stamp}");

    let buy_index = event
        .books
        .iter()
        .position(|b| b.venue == opportunity.buy_venue)
        .with_context(|| {
            format!(
                "F-02 opportunity buy venue {} absent from event books",
                opportunity.buy_venue
            )
        })?;
    let sell_index = event
        .books
        .iter()
        .position(|b| b.venue == opportunity.sell_venue)
        .with_context(|| {
            format!(
                "F-02 opportunity sell venue {} absent from event books",
                opportunity.sell_venue
            )
        })?;
    let buy_book = &event.books[buy_index];
    let sell_book = &event.books[sell_index];
    let buy_instrument = &event.instruments[buy_index];
    let sell_instrument = &event.instruments[sell_index];

    let symbol = event.config.symbol.clone();
    let base_asset = event.config.base_asset.clone();
    let quote_asset = event.config.quote_asset.clone();
    let target = event.config.quantity;
    let dir = if buy_index == 0 { "a" } else { "b" };
    let account_id = format!("f02-account-{stamp}-{dir}");
    let instrument_id = format!("f02-instrument-{sl}-{stamp}", sl = symbol.replace('/', "-"));
    let plan_id = format!("{run_id}-plan-{dir}");
    let opportunity_id = format!("{run_id}-opportunity-{dir}");
    let buy_intent_id = format!("{run_id}-intent-buy");
    let sell_intent_id = format!("{run_id}-intent-sell");
    let buy_client_order_id = format!("{run_id}-client-buy");
    let sell_client_order_id = format!("{run_id}-client-sell");

    // 有效种子：0 = 按运行时间播种（回写报告），非 0 = 全链路确定性。
    let seed = if config.seed == 0 { now } else { config.seed };
    let mut rng = SimRng::new(seed);
    let decision_to_submit_ms = rng.gen_range_u64(
        config.decision_to_submit_ms_min,
        config.decision_to_submit_ms_max,
    );
    let fill_latency_ms = rng.gen_range_u64(config.fill_latency_ms_min, config.fill_latency_ms_max);

    // 保守限价与合作额度估计（禁用作成交价；仅限价/预留额度）。
    let buy_limit = offset_level_price(
        opportunity.buy_worst_price,
        BookSide::Asks,
        &config.adverse_move_bps,
        &buy_instrument.price_tick,
    )?;
    let sell_limit = offset_level_price(
        opportunity.sell_worst_price,
        BookSide::Bids,
        &config.adverse_move_bps,
        &sell_instrument.price_tick,
    )?;
    let quote_cost_estimate = (target
        * opportunity.buy_worst_price
        * (Decimal::ONE + config.adverse_move_bps / bps_decimal()))
    .round_dp_with_strategy(8, RoundingStrategy::AwayFromZero);

    // 关键节点：run 参数摘要（成交价/限价仅供模拟，禁用作真实委托价）。
    tracing::info!(
        "F-02 simulation: run {} start symbol={} qty={} buy={}->{} seed={} buy_limit={} sell_limit={} quote_cost={} decision_to_submit={}ms fill_latency={}ms",
        run_id,
        symbol,
        target,
        opportunity.buy_venue,
        opportunity.sell_venue,
        seed,
        buy_limit,
        sell_limit,
        quote_cost_estimate,
        decision_to_submit_ms,
        fill_latency_ms
    );

    let mut report = SimulationRunReport {
        schema_version: SCHEMA_VERSION,
        seed,
        run_id: run_id.clone(),
        symbol: symbol.clone(),
        quantity: target,
        buy_venue: opportunity.buy_venue.clone(),
        sell_venue: opportunity.sell_venue.clone(),
        scenario: SimulationScenario::Normal,
        rejection_reason: None,
        plan_id: plan_id.clone(),
        account_id: account_id.clone(),
        instrument_id: instrument_id.clone(),
        buy_intent_id: buy_intent_id.clone(),
        sell_intent_id: sell_intent_id.clone(),
        buy_client_order_id: buy_client_order_id.clone(),
        sell_client_order_id: sell_client_order_id.clone(),
        evaluated_at_ms,
        decision_to_submit_ms,
        fill_latency_ms,
        buy: SimLegResult {
            filled: Decimal::ZERO,
            quote_amount: Decimal::ZERO,
            avg_price: Decimal::ZERO,
            last_raw_price: Decimal::ZERO,
            last_fill_price: Decimal::ZERO,
            depth_shortfall: false,
            below_min_quantity: false,
        },
        sell: SimLegResult {
            filled: Decimal::ZERO,
            quote_amount: Decimal::ZERO,
            avg_price: Decimal::ZERO,
            last_raw_price: Decimal::ZERO,
            last_fill_price: Decimal::ZERO,
            depth_shortfall: false,
            below_min_quantity: false,
        },
        buy_fee: Decimal::ZERO,
        sell_fee: Decimal::ZERO,
        scanned_net_profit: opportunity.expected_net_profit,
        simulated_net_profit: Decimal::ZERO,
        adverse_move_bps: config.adverse_move_bps,
        competitor_take_bps: config.competitor_take_bps,
        // 合法占位：REJECTED / idempotent replay 等无执行事实路径直接持久化，
        // 空串会违反 0005 simulation_runs_compensation/execution_state CHECK；
        // 成功 run 在报告收尾按真实快照覆写（见下方 outcome.snapshot 赋值）。
        compensation_decision: "NO_ACTION".to_owned(),
        execution_state: "PLANNED".to_owned(),
        unmatched_quantity: Decimal::ZERO,
        unmatched_exposure: Decimal::ZERO,
        estimated_compensation_cost: Decimal::ZERO,
        unknown_submit_tried: false,
        query_found: false,
        idempotent_replay: false,
        skipped_frames,
        warnings: Vec::new(),
        external_order_calls: 0,
    };

    // 前置：注入每 run 独立资金池（D5 跨机会共享资金明确不做）。
    set_paper_balance(
        database_url,
        &account_id,
        &opportunity.buy_venue,
        &quote_asset,
        config.initial_quote_balance,
        config.initial_quote_balance,
        now,
    )
    .await?;
    set_paper_balance(
        database_url,
        &account_id,
        &opportunity.sell_venue,
        &base_asset,
        config.initial_base_balance,
        config.initial_base_balance,
        now,
    )
    .await?;
    tracing::info!(
        "F-02 simulation: run {} funded account={} quote@{}={} base@{}={}",
        run_id,
        account_id,
        opportunity.buy_venue,
        config.initial_quote_balance,
        opportunity.sell_venue,
        config.initial_base_balance
    );

    // 资金充足性预检：不足 → Rejected 报告，不触碰 PAPER 事实层（无残留）。
    let quote_available = available_balance(
        database_url,
        &account_id,
        &opportunity.buy_venue,
        &quote_asset,
    )
    .await?;
    let base_available = available_balance(
        database_url,
        &account_id,
        &opportunity.sell_venue,
        &base_asset,
    )
    .await?;
    tracing::info!(
        "F-02 simulation: run {} balance_check quote_available={} needed={} base_available={} needed={}",
        run_id,
        quote_available,
        quote_cost_estimate,
        base_available,
        target
    );
    if quote_available < quote_cost_estimate || base_available < target {
        report.scenario = SimulationScenario::Rejected;
        let reason = format!(
            "insufficient PAPER funds: quote available {} needed {}, base available {} needed {}",
            quote_available, quote_cost_estimate, base_available, target
        );
        report.rejection_reason = Some(reason.clone());
        tracing::warn!("F-02 simulation: run {} REJECTED: {reason}", run_id);
        persist_simulation_run(
            database_url,
            &report,
            config,
            &quote_asset,
            &base_asset,
            dir,
            now,
        )
        .await?;
        return Ok(report);
    }

    // PAPER 预留：原子 $request → plan + intents + reservations + risk_decision + audit。
    let mut paper = PaperCore::acquire(database_url, &account_id, &instrument_id).await?;
    let request = ReservePlanRequest {
        request_id: format!("{run_id}-request-{dir}"),
        plan_id: plan_id.clone(),
        account_id: account_id.clone(),
        instrument_id: instrument_id.clone(),
        opportunity_id: opportunity_id.clone(),
        strategy_config_version: SIMULATION_CONFIG_VERSION.to_owned(),
        target_quantity: target,
        max_unmatched_exposure: config.max_unmatched_exposure,
        intents: [
            OrderIntentInput {
                intent_id: buy_intent_id.clone(),
                leg_id: "buy".to_owned(),
                attempt_id: format!("{run_id}-attempt-buy"),
                client_order_id: buy_client_order_id.clone(),
                venue: opportunity.buy_venue.clone(),
                side: OrderSide::Buy,
                quantity: target,
                limit_price: buy_limit,
            },
            OrderIntentInput {
                intent_id: sell_intent_id.clone(),
                leg_id: "sell".to_owned(),
                attempt_id: format!("{run_id}-attempt-sell"),
                client_order_id: sell_client_order_id.clone(),
                venue: opportunity.sell_venue.clone(),
                side: OrderSide::Sell,
                quantity: target,
                limit_price: sell_limit,
            },
        ],
        reservations: vec![
            ReservationInput {
                reservation_id: format!("{run_id}-reserve-quote"),
                venue: opportunity.buy_venue.clone(),
                asset: quote_asset.clone(),
                amount: quote_cost_estimate,
            },
            ReservationInput {
                reservation_id: format!("{run_id}-reserve-base"),
                venue: opportunity.sell_venue.clone(),
                asset: base_asset.clone(),
                amount: target,
            },
        ],
    };
    let outcome = paper.reserve_plan(request).await?;
    paper.disconnect().await;
    if outcome.idempotent_replay {
        report.idempotent_replay = true;
        report
            .warnings
            .push("idempotent replay: plan already exists; run skipped".to_owned());
        tracing::warn!(
            "F-02 simulation: run {} idempotent_replay plan={} already exists; run skipped",
            run_id,
            outcome.plan.plan_id
        );
        persist_simulation_run(
            database_url,
            &report,
            config,
            &quote_asset,
            &base_asset,
            dir,
            now,
        )
        .await?;
        return Ok(report);
    }

    // B-03 执行事实初始化（domain 单写者锁：先占先释放，串行化 paper→execution→order→execution）。
    let mut execution = ExecutionCore::acquire(database_url, &account_id, &instrument_id).await?;
    execution
        .initialize_plan(
            &plan_id,
            target,
            config.max_unmatched_exposure,
            config.compensation_budget,
        )
        .await?;
    execution.disconnect().await;
    tracing::info!(
        "F-02 simulation: run {} execution_plan plan={} target={} exposure_budget={} compensation_budget={}",
        run_id,
        plan_id,
        target,
        config.max_unmatched_exposure,
        config.compensation_budget
    );

    // 订单事实：双腿提交 → 确认/UNKNOWN。
    let mut order = OrderCore::acquire(database_url, &account_id, &instrument_id).await?;
    let (buy_outcome, buy_unknown, buy_found) = submit_and_confirm(
        &mut order,
        &run_id,
        &buy_intent_id,
        "buy",
        evaluated_at_ms,
        &mut rng,
        config,
    )
    .await?;
    let (sell_outcome, sell_unknown, sell_found) = submit_and_confirm(
        &mut order,
        &run_id,
        &sell_intent_id,
        "sell",
        evaluated_at_ms,
        &mut rng,
        config,
    )
    .await?;
    report.unknown_submit_tried = buy_unknown || sell_unknown;
    report.query_found = buy_found || sell_found;
    tracing::info!(
        "F-02 simulation: run {} submit buy={} sell={} unknown_tried={}/{} query_found={}/{}",
        run_id,
        outcome_tag(&buy_outcome),
        outcome_tag(&sell_outcome),
        buy_unknown,
        sell_unknown,
        buy_found,
        sell_found
    );

    // 撮合：run 启动时事件快照按 D2 逐档消费；每档为独立成交 chunk（幂等 trade_id）。
    let buy_result = simulate_leg(BookSide::Asks, buy_book, buy_instrument, target, config)?;
    let sell_result = simulate_leg(BookSide::Bids, sell_book, sell_instrument, target, config)?;
    tracing::info!(
        "F-02 simulation: run {} fill buy filled={} avg={} shortfall={} below_min={} | sell filled={} avg={} shortfall={} below_min={}",
        run_id,
        buy_result.filled,
        buy_result.avg_price,
        buy_result.depth_shortfall,
        buy_result.below_min_quantity,
        sell_result.filled,
        sell_result.avg_price,
        sell_result.depth_shortfall,
        sell_result.below_min_quantity
    );

    let occurred_at_ms = evaluated_at_ms
        .saturating_add(decision_to_submit_ms)
        .saturating_add(fill_latency_ms);
    let mut buy_fee = Decimal::ZERO;
    let mut sell_fee = Decimal::ZERO;
    if let SubmitOutcome::Accepted { exchange_order_id } = &buy_outcome {
        if buy_result.filled > Decimal::ZERO {
            buy_fee = buy_result.quote_amount * event.fees[buy_index].buy_taker_rate;
            order
                .record_trade(
                    &buy_intent_id,
                    TradeInput {
                        venue: opportunity.buy_venue.clone(),
                        exchange_order_id: exchange_order_id.clone(),
                        trade_id: format!("{run_id}-buy-trade"),
                        quantity: buy_result.filled,
                        price: buy_result.avg_price,
                        fee_asset: quote_asset.clone(),
                        fee_amount: buy_fee,
                        source_sequence: 1,
                        occurred_at_ms,
                    },
                )
                .await?;
        }
    } else if buy_result.filled > Decimal::ZERO {
        report.warnings.push(
            "buy leg had simulated fills but submission is UNKNOWN/NotFound; fills not booked"
                .to_owned(),
        );
    }
    if let SubmitOutcome::Accepted { exchange_order_id } = &sell_outcome {
        if sell_result.filled > Decimal::ZERO {
            sell_fee = sell_result.quote_amount * event.fees[sell_index].sell_taker_rate;
            order
                .record_trade(
                    &sell_intent_id,
                    TradeInput {
                        venue: opportunity.sell_venue.clone(),
                        exchange_order_id: exchange_order_id.clone(),
                        trade_id: format!("{run_id}-sell-trade"),
                        quantity: sell_result.filled,
                        price: sell_result.avg_price,
                        fee_asset: quote_asset.clone(),
                        fee_amount: sell_fee,
                        source_sequence: 1,
                        occurred_at_ms,
                    },
                )
                .await?;
        }
    } else if sell_result.filled > Decimal::ZERO {
        report.warnings.push(
            "sell leg had simulated fills but submission is UNKNOWN/NotFound; fills not booked"
                .to_owned(),
        );
    }

    // B-03 补偿评估：保守敞口价 = 卖侧最差原始价；补偿单位成本 = 中间价×(1+markup)。
    order.disconnect().await;
    let mut execution = ExecutionCore::acquire(database_url, &account_id, &instrument_id).await?;
    let exposure_per_unit = opportunity.sell_worst_price;
    let mid = (opportunity.buy_worst_price + opportunity.sell_worst_price) / Decimal::from(2);
    let compensation_cost_per_unit =
        mid * (Decimal::ONE + config.compensation_markup_bps / bps_decimal());
    let outcome = execution
        .evaluate(
            &plan_id,
            buy_result.filled,
            sell_result.filled,
            exposure_per_unit,
            compensation_cost_per_unit,
        )
        .await?;
    execution.disconnect().await;
    tracing::info!(
        "F-02 simulation: run {} evaluate exposure_per_unit={} comp_cost_per_unit={} decision={:?} state={:?} unmatched_qty={} unmatched_exposure={} est_comp={}",
        run_id,
        exposure_per_unit,
        compensation_cost_per_unit,
        outcome.decision,
        outcome.snapshot.state,
        outcome.snapshot.unmatched_quantity,
        outcome.snapshot.unmatched_exposure,
        outcome.estimated_cost
    );

    let scenario = if buy_result.filled == Decimal::ZERO || sell_result.filled == Decimal::ZERO {
        SimulationScenario::CompetedAway
    } else if buy_result.depth_shortfall || sell_result.depth_shortfall {
        SimulationScenario::DepthShortfall
    } else {
        SimulationScenario::Normal
    };

    report.scenario = scenario;
    report.simulated_net_profit =
        sell_result.quote_amount - sell_fee - buy_result.quote_amount - buy_fee;
    report.buy = buy_result;
    report.sell = sell_result;
    report.buy_fee = buy_fee;
    report.sell_fee = sell_fee;
    report.compensation_decision = match outcome.decision {
        CompensationDecision::NoAction => "NO_ACTION",
        CompensationDecision::CompensationPlanned => "COMPENSATION_PLANNED",
        CompensationDecision::ManualRequired => "MANUAL_REQUIRED",
    }
    .to_owned();
    report.execution_state = match outcome.snapshot.state {
        ExecutionState::Planned => "PLANNED",
        ExecutionState::Running => "RUNNING",
        ExecutionState::CompensationPlanned => "COMPENSATION_PLANNED",
        ExecutionState::Completed => "COMPLETED",
        ExecutionState::ManualRequired => "MANUAL_REQUIRED",
    }
    .to_owned();
    report.unmatched_quantity = outcome.snapshot.unmatched_quantity;
    report.unmatched_exposure = outcome.snapshot.unmatched_exposure;
    report.estimated_compensation_cost = outcome.estimated_cost;
    persist_simulation_run(
        database_url,
        &report,
        config,
        &quote_asset,
        &base_asset,
        dir,
        now,
    )
    .await?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// 连续模式桥（D4）：observer 接入点，单槽最新优先。
// ---------------------------------------------------------------------------

/// 连续模式 `SimulationEngine`：observer 每 tick `offer` 机会事件（克隆），
/// 独立 tokio task 串行单写者执行撮合 run。积压跳帧只处理最新机会。
pub struct SimulationEngine {
    slot: Arc<Mutex<Option<DecisionEvent>>>,
    notify: Arc<Notify>,
    stopped: Arc<AtomicBool>,
    /// offer 覆盖未消费槽位的跳帧计数（每 run 报告携带该值）。
    skipped: Arc<AtomicU64>,
    worker: tokio::task::JoinHandle<()>,
}

impl SimulationEngine {
    /// 以配置启用；`TAOLI_DATABASE_URL` 缺失或迁移失败 → 告警并返回 None（不阻断观察循环）。
    pub async fn spawn(config: SimulationConfig) -> Option<Arc<Self>> {
        let Ok(database_url) = std::env::var("TAOLI_DATABASE_URL") else {
            tracing::error!(
                "F-02 simulation engine: TAOLI_DATABASE_URL is not set; simulation disabled"
            );
            return None;
        };
        if let Err(error) = migrate(&database_url).await {
            tracing::error!(
                "F-02 simulation engine: database migration failed: {error:#}; simulation disabled"
            );
            return None;
        }
        let slot = Arc::new(Mutex::new(None));
        let notify = Arc::new(Notify::new());
        let stopped = Arc::new(AtomicBool::new(false));
        let skipped = Arc::new(AtomicU64::new(0));
        let config_seed = config.seed;
        let worker = {
            let slot = Arc::clone(&slot);
            let notify = Arc::clone(&notify);
            let stopped = Arc::clone(&stopped);
            let skipped = Arc::clone(&skipped);
            tokio::spawn(async move {
                loop {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let event = { slot.lock().await.take() };
                    let Some(event) = event else {
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                        notify.notified().await;
                        continue;
                    };
                    let frame_skipped = skipped.swap(0, Ordering::Relaxed);
                    match run_all_directions(&database_url, &event, &config, frame_skipped).await {
                        Ok(count) => tracing::info!(
                            "F-02 simulation: ran {count} direction(s) for {} at {}",
                            event.config.symbol,
                            event.evaluated_at_ms
                        ),
                        Err(error) => tracing::error!("F-02 simulation run failed: {error:#}"),
                    }
                }
            })
        };
        tracing::info!(
            "F-02 simulation: engine started (continuous observer hook, seed config={config_seed})",
        );
        Some(Arc::new(Self {
            slot,
            notify,
            stopped,
            skipped,
            worker,
        }))
    }

    /// observer 每 tick 调用；槽位被占用（积压）时覆盖旧机会 → 跳帧只处理最新。
    pub async fn offer(&self, event: DecisionEvent) {
        let mut guard = self.slot.lock().await;
        if guard.is_some() {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
        *guard = Some(event);
        self.notify.notify_one();
    }

    /// 停止 worker（observer 正常结束路径调用）。
    ///
    /// `stopped` + `notify` 只用于唤醒空闲 worker 让它自行退出；如果 worker 正卡在一次
    /// 撮合里，`abort` 才是真正的终止手段。`Drop` 兜底同一条路径，覆盖 observer 循环
    /// 用 `?` 提前返回、因而跳过本方法的场景。
    pub async fn shutdown(self: Arc<Self>) {
        self.stopped.store(true, Ordering::Release);
        self.notify.notify_one();
        self.worker.abort();
        tracing::info!("F-02 simulation: engine stopped");
    }
}

impl Drop for SimulationEngine {
    fn drop(&mut self) {
        // 与 `shutdown` 等价：`Arc<Self>` 的最后一个引用释放时，worker 不能存活。
        self.stopped.store(true, Ordering::Release);
        self.worker.abort();
    }
}

/// 执行事件内全部 accepted 方向（每方向独立 run）。返回成功 run 数。
async fn run_all_directions(
    database_url: &str,
    event: &DecisionEvent,
    config: &SimulationConfig,
    skipped_frames: u64,
) -> Result<usize> {
    let mut ran = 0;
    for opportunity in event.report.directions.iter().filter(|d| d.accepted) {
        let report =
            run_simulation_run(database_url, event, opportunity, config, skipped_frames).await?;
        tracing::info!(
            "F-02 run {} {} -> {} scenario={:?} net={}",
            report.run_id,
            report.buy_venue,
            report.sell_venue,
            report.scenario,
            report.simulated_net_profit
        );
        ran += 1;
    }
    Ok(ran)
}

// ---------------------------------------------------------------------------
// PostgreSQL 烟测（S01–S09）：手动触发，不纳入常规 `cargo test`。
// ---------------------------------------------------------------------------

/// F-02 模拟套利烟测报告（S01–S09 + 恢复一致性 + external_order_calls=0）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SimulationSmokeReport {
    pub schema_version: i64,
    pub s01_full_fill_at_worst: bool,
    pub s02_partial_fill_depth_shortfall: bool,
    pub s03_competed_away: bool,
    pub s04_worse_than_scan_price: bool,
    pub s05_insufficient_funds_rejected: bool,
    pub s06_unknown_query_recovered_no_duplicate: bool,
    pub s07_compensation_over_budget_manual: bool,
    pub s08_idempotent_replay: bool,
    pub s09_fact_recovery_consistent: bool,
    pub external_order_calls: usize,
}

mod smoke_fixtures {
    //! 烟测专用合成簿/事件构造（仅经 `run_simulation_smoke` 使用）。

    use crate::archive::{ArchivedFeeVersion, DecisionEvent, FeedVersion};
    use crate::config::StrategyConfig;
    use crate::local_book::BookState;
    use crate::market::{Level, OrderBookSnapshot};
    use crate::scan::FreshnessLimits;
    use crate::scan::{Opportunity, ScanReport};
    use rust_decimal::Decimal;

    pub fn dec(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    pub fn smoke_spec(venue: &str, symbol: &str) -> crate::instrument::InstrumentSpec {
        crate::instrument::InstrumentSpec {
            venue: venue.into(),
            account_type: "spot".into(),
            venue_symbol: symbol.into(),
            canonical_instrument_id: format!("{symbol}:SPOT"),
            base_asset_id: "BTC".into(),
            quote_asset_id: "USDT".into(),
            settlement_asset_id: "USDT".into(),
            market_type: "spot".into(),
            contract_multiplier: Decimal::ONE,
            quantity_unit: "base_asset".into(),
            price_tick: dec("0.1"),
            quantity_step: dec("0.0001"),
            min_quantity: dec("0.0001"),
            max_quantity: Some(dec("100")),
            min_notional: dec("5"),
            max_notional: None,
            trading_status: crate::instrument::TradingStatus::Trading,
            supported_order_types: vec![
                crate::instrument::OrderCapability::Limit,
                crate::instrument::OrderCapability::Ioc,
            ],
            metadata_version: 1,
        }
    }

    pub fn smoke_book(
        venue: &str,
        symbol: &str,
        asks: &[(&str, &str)],
        bids: &[(&str, &str)],
        sequence: u64,
    ) -> OrderBookSnapshot {
        OrderBookSnapshot {
            venue: venue.into(),
            symbol: symbol.into(),
            bids: bids
                .iter()
                .map(|(price, quantity)| Level {
                    price: dec(price),
                    quantity: dec(quantity),
                })
                .collect(),
            asks: asks
                .iter()
                .map(|(price, quantity)| Level {
                    price: dec(price),
                    quantity: dec(quantity),
                })
                .collect(),
            sequence,
            source_timestamp_ms: None,
            received_timestamp_ms: 1,
        }
    }

    fn smoke_fee(venue: &str, symbol: &str, taker: &str) -> ArchivedFeeVersion {
        ArchivedFeeVersion {
            venue: venue.into(),
            symbol: symbol.into(),
            buy_taker_rate: dec(taker),
            sell_taker_rate: dec(taker),
            source: "smoke".into(),
            loaded_at_ms: None,
            expires_at_ms: None,
            actual_account_rate: false,
        }
    }

    fn smoke_opportunity(
        buy_venue: &str,
        sell_venue: &str,
        buy_worst: &str,
        sell_worst: &str,
        quantity: &str,
    ) -> Opportunity {
        let quantity = dec(quantity);
        let buy_worst = dec(buy_worst);
        let sell_worst = dec(sell_worst);
        let buy_cost = buy_worst * quantity;
        let sell_proceeds = sell_worst * quantity;
        let fees = buy_cost * dec("0.0005") + sell_proceeds * dec("0.0005");
        let expected_net_profit = sell_proceeds - buy_cost - fees;
        Opportunity {
            buy_venue: buy_venue.into(),
            sell_venue: sell_venue.into(),
            buy_vwap: buy_worst,
            sell_vwap: sell_worst,
            buy_worst_price: buy_worst,
            sell_worst_price: sell_worst,
            buy_cost,
            sell_proceeds,
            gross_profit: sell_proceeds - buy_cost,
            fees,
            latency_loss_estimate: Decimal::ZERO,
            rebalance_cost: Decimal::ZERO,
            other_direct_cost: Decimal::ZERO,
            expected_net_profit,
            risk_buffer: Decimal::ZERO,
            admission_profit: expected_net_profit,
            admission_net_bps: Decimal::ZERO,
            accepted: true,
            rejection_reasons: Vec::new(),
        }
    }

    /// 直接构造 DecisionEvent（全部字段 pub）：books 的 venue 顺序即买/卖侧索引。
    pub fn smoke_event(
        books: [OrderBookSnapshot; 2],
        quantity: &str,
        evaluated_at_ms: u64,
    ) -> DecisionEvent {
        let symbol = "BTCUSDT".to_owned();
        let buy_venue = books[0].venue.clone();
        let sell_venue = books[1].venue.clone();
        let quantity_d = dec(quantity);
        let instruments = [
            smoke_spec(&buy_venue, &symbol),
            smoke_spec(&sell_venue, &symbol),
        ];
        let report = ScanReport {
            symbol: symbol.clone(),
            quantity: quantity_d,
            observed_at_ms: evaluated_at_ms,
            pair_received_skew_ms: 1,
            directions: vec![smoke_opportunity(
                &buy_venue,
                &sell_venue,
                books[0].asks[0].price.to_string().as_str(),
                books[1].bids[0].price.to_string().as_str(),
                quantity,
            )],
        };
        DecisionEvent {
            evaluated_at_ms,
            feeds: books
                .iter()
                .map(|b| FeedVersion {
                    venue: b.venue.clone(),
                    symbol: b.symbol.clone(),
                    state: BookState::Valid,
                    generation: 1,
                    reconnects: 0,
                    applied_updates: 0,
                    reason: None,
                })
                .collect::<Vec<_>>()
                .try_into()
                .expect("two books"),
            books,
            instruments,
            fees: [
                smoke_fee(&buy_venue, &symbol, "0.0005"),
                smoke_fee(&sell_venue, &symbol, "0.0005"),
            ],
            config_version: String::new(),
            config: crate::archive::DecisionConfig {
                symbol: symbol.clone(),
                base_asset: "BTC".into(),
                quote_asset: "USDT".into(),
                quantity: quantity_d,
                strategy: StrategyConfig {
                    min_net_profit: dec("0"),
                    min_net_bps: dec("0"),
                    latency_loss_bps: dec("0.0001"),
                    risk_buffer_bps: dec("0"),
                    rebalance_cost: dec("0"),
                    other_direct_cost: dec("0"),
                },
                freshness: FreshnessLimits {
                    max_snapshot_age_ms: 60_000,
                    max_pair_skew_ms: 5_000,
                },
            },
            admission_rejections: Vec::new(),
            report,
        }
    }
}

/// F-02 模拟套利烟测：合成簿驱动 S01–S09，全部断言通过才返回 Ok。
/// PostgreSQL 手动触发（B 系列范式，不纳入常规 `cargo test`）。
pub async fn run_simulation_smoke(database_url: &str) -> Result<SimulationSmokeReport> {
    use smoke_fixtures::{dec, smoke_book, smoke_event};

    migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let evaluated_at_ms = unix_timestamp_ms()?;
    let quantity = "0.001";

    // S01 基线：adverse=0 / competitor=0，深度充足 → 双腿按最差档足额成交，
    // 模拟净盈亏 ≈ 扫码净盈亏（零偏移下两模型一致）。
    let s01_config = SimulationConfig {
        seed: 1,
        enabled: true,
        initial_quote_balance: dec("10000"),
        initial_base_balance: dec("2"),
        decision_to_submit_ms_min: 50,
        decision_to_submit_ms_max: 50,
        fill_latency_ms_min: 20,
        fill_latency_ms_max: 20,
        adverse_move_bps: dec("0"),
        competitor_take_bps: dec("0"),
        unknown_submit_probability_bps: dec("0"),
        query_found_probability_bps: dec("6000"),
        max_unmatched_exposure: dec("100"),
        compensation_budget: dec("20"),
        compensation_markup_bps: dec("30"),
    };
    let s01_books = [
        smoke_book(
            "s01-buy",
            "BTCUSDT",
            &[("80000", "0.002")],
            &[("79900", "0.002")],
            1,
        ),
        smoke_book(
            "s01-sell",
            "BTCUSDT",
            &[("80200", "0.002")],
            &[("80100", "0.002")],
            1,
        ),
    ];
    let s01_event = smoke_event(s01_books.clone(), quantity, evaluated_at_ms);
    let s01_opp = s01_event.report.directions[0].clone();
    let s01 = run_simulation_run(database_url, &s01_event, &s01_opp, &s01_config, 0).await?;
    let s01_full_fill_at_worst = s01.scenario == SimulationScenario::Normal
        && s01.buy.filled == dec(quantity)
        && s01.sell.filled == dec(quantity)
        && s01.buy.avg_price == dec("80000")
        && s01.sell.avg_price == dec("80100")
        && (s01.simulated_net_profit - s01.scanned_net_profit).abs() <= dec("0.00000001");

    // S02 深度不足：买侧可得 0.0005 < 目标 0.001 → 部分成交 + depth_shortfall，
    // 卖侧足额；补偿预算放宽以落入 COMPENSATION_PLANNED。
    let s02_config = SimulationConfig {
        seed: 2,
        initial_quote_balance: dec("10000"),
        initial_base_balance: dec("2"),
        unknown_submit_probability_bps: dec("0"),
        query_found_probability_bps: dec("6000"),
        compensation_budget: dec("200"),
        ..SimulationConfig::default()
    };
    let s02_books = [
        smoke_book(
            "s02-buy",
            "BTCUSDT",
            &[("80000", "0.0006")],
            &[("79900", "0.002")],
            1,
        ),
        smoke_book(
            "s02-sell",
            "BTCUSDT",
            &[("80200", "0.002")],
            &[("80100", "0.002")],
            1,
        ),
    ];
    let s02_event = smoke_event(s02_books.clone(), quantity, evaluated_at_ms);
    let s02_opp = s02_event.report.directions[0].clone();
    let s02 = run_simulation_run(database_url, &s02_event, &s02_opp, &s02_config, 0).await?;
    let s02_partial_fill_depth_shortfall = s02.scenario == SimulationScenario::DepthShortfall
        && s02.buy.depth_shortfall
        && s02.buy.filled == dec("0.0005")
        && s02.sell.filled == dec(quantity)
        && s02.compensation_decision == "COMPENSATION_PLANNED";

    // S03 敌手占盘 100% → 每档有效量为 0 → 双腿无成交 → CompetedAway。
    let s03_config = SimulationConfig {
        seed: 3,
        competitor_take_bps: dec("10000"),
        ..SimulationConfig::default()
    };
    let s03_books = [
        smoke_book(
            "s03-buy",
            "BTCUSDT",
            &[("80000", "0.002")],
            &[("79900", "0.002")],
            1,
        ),
        smoke_book(
            "s03-sell",
            "BTCUSDT",
            &[("80200", "0.002")],
            &[("80100", "0.002")],
            1,
        ),
    ];
    let s03_event = smoke_event(s03_books.clone(), quantity, evaluated_at_ms);
    let s03_opp = s03_event.report.directions[0].clone();
    let s03 = run_simulation_run(database_url, &s03_event, &s03_opp, &s03_config, 0).await?;
    let s03_competed_away = s03.scenario == SimulationScenario::CompetedAway
        && s03.buy.filled == Decimal::ZERO
        && s03.sell.filled == Decimal::ZERO
        && s03.compensation_decision == "NO_ACTION";

    // S04 adverse 生效：买价平均高于扫码最差、卖价平均低于扫码最差（滑点对比基线）。
    let s04_config = SimulationConfig {
        seed: 4,
        ..SimulationConfig::default()
    };
    let s04_books = [
        smoke_book(
            "s04-buy",
            "BTCUSDT",
            &[("80000", "0.002")],
            &[("79900", "0.002")],
            1,
        ),
        smoke_book(
            "s04-sell",
            "BTCUSDT",
            &[("80200", "0.002")],
            &[("80300", "0.002")],
            1,
        ),
    ];
    let s04_event = smoke_event(s04_books.clone(), quantity, evaluated_at_ms);
    let s04_opp = s04_event.report.directions[0].clone();
    let s04 = run_simulation_run(database_url, &s04_event, &s04_opp, &s04_config, 0).await?;
    let s04_worse_than_scan_price = s04.scenario == SimulationScenario::Normal
        && s04.buy.avg_price > s04_opp.buy_worst_price
        && s04.sell.avg_price < s04_opp.sell_worst_price;

    // S05 资金不足 → Rejected 报告，PAPER 事实零残留。
    let s05_config = SimulationConfig {
        seed: 5,
        initial_quote_balance: dec("1"),
        ..SimulationConfig::default()
    };
    let s05_event = smoke_event(s01_books.clone(), quantity, evaluated_at_ms);
    let s05_opp = s05_event.report.directions[0].clone();
    let s05 = run_simulation_run(database_url, &s05_event, &s05_opp, &s05_config, 0).await?;
    let plan_count =
        business_count(database_url, "execution_plans", "plan_id", &s05.plan_id).await?;
    // 投影行必须成功落库且占位枚举合法（0005 CHECK 不再被空串触发）。
    let s05_project_pool = pool(database_url).await?;
    let s05_projection = sqlx::query(
        "SELECT compensation_decision, execution_state, rejection_reason
             FROM simulation_runs WHERE run_id = $1",
    )
    .bind(&s05.run_id)
    .fetch_one(&s05_project_pool)
    .await?;
    let s05_projection_legal = s05_projection.try_get::<String, _>(0)? == "NO_ACTION"
        && s05_projection.try_get::<String, _>(1)? == "PLANNED"
        && s05_projection
            .try_get::<Option<String>, _>(2)?
            .is_some_and(|r| r.starts_with("insufficient PAPER funds"));
    let s05_insufficient_funds_rejected = s05.scenario == SimulationScenario::Rejected
        && s05.rejection_reason.is_some()
        && s05
            .rejection_reason
            .as_deref()
            .is_some_and(|r| r.contains("insufficient PAPER funds"))
        && plan_count == 0
        && s05_projection_legal;

    // S06 UNKNOWN 提交 → 查询 Found 恢复订单身份 → 成交落库；重复 trade 幂等忽略。
    let s06_config = SimulationConfig {
        seed: 6,
        unknown_submit_probability_bps: dec("10000"),
        query_found_probability_bps: dec("10000"),
        decision_to_submit_ms_min: 50,
        decision_to_submit_ms_max: 50,
        fill_latency_ms_min: 20,
        fill_latency_ms_max: 20,
        ..SimulationConfig::default()
    };
    let s06_event = smoke_event(s01_books.clone(), quantity, evaluated_at_ms);
    let s06_opp = s06_event.report.directions[0].clone();
    let s06_run = run_simulation_run(database_url, &s06_event, &s06_opp, &s06_config, 0).await?;
    if s06_run.scenario != SimulationScenario::Normal {
        bail!("S06 run was not Normal: {:?}", s06_run.scenario);
    }
    let mut order =
        OrderCore::acquire(database_url, &s06_run.account_id, &s06_run.instrument_id).await?;
    let buy_fact = order.load(&s06_run.buy_intent_id).await?;
    let sell_fact = order.load(&s06_run.sell_intent_id).await?;
    let duplicate = order
        .record_trade(
            &s06_run.buy_intent_id,
            TradeInput {
                venue: s06_run.buy_venue.clone(),
                exchange_order_id: format!("{}-buy-exc", s06_run.run_id),
                trade_id: format!("{}-buy-trade", s06_run.run_id),
                quantity: s06_run.buy.filled,
                price: s06_run.buy.avg_price,
                fee_asset: "USDT".into(),
                fee_amount: s06_run.buy_fee,
                source_sequence: 1,
                occurred_at_ms: s06_run.evaluated_at_ms + 70,
            },
        )
        .await?;
    order.disconnect().await;
    let s06_unknown_query_recovered_no_duplicate = s06_run.unknown_submit_tried
        && s06_run.query_found
        && buy_fact.filled_quantity == s06_run.buy.filled
        && sell_fact.filled_quantity == s06_run.sell.filled
        && duplicate.filled_quantity == buy_fact.filled_quantity;

    // S07 补偿超预算 → MANUAL_REQUIRED：卖侧深度仅 0.0003 可成交，mismatch 敞口超预算。
    let s07_config = SimulationConfig {
        seed: 7,
        ..SimulationConfig::default()
    };
    let s07_books = [
        smoke_book(
            "s07-buy",
            "BTCUSDT",
            &[("80000", "0.002")],
            &[("79900", "0.002")],
            1,
        ),
        smoke_book(
            "s07-sell",
            "BTCUSDT",
            &[("80200", "0.002")],
            &[("80100", "0.0004")],
            1,
        ),
    ];
    let s07_event = smoke_event(s07_books.clone(), quantity, evaluated_at_ms);
    let s07_opp = s07_event.report.directions[0].clone();
    let s07 = run_simulation_run(database_url, &s07_event, &s07_opp, &s07_config, 0).await?;
    let s07_compensation_over_budget_manual = s07.sell.depth_shortfall
        && s07.sell.filled == dec("0.0003")
        && s07.buy.filled == dec(quantity)
        && s07.compensation_decision == "MANUAL_REQUIRED"
        && s07.execution_state == "MANUAL_REQUIRED";

    // S08 同 request 二次 reserve_plan → idempotent replay，事实不重复。
    let s08_account = format!("s08-account-{stamp}");
    let s08_instrument = format!("s08-instrument-{stamp}");
    set_paper_balance(
        database_url,
        &s08_account,
        "s08-buy",
        "USDT",
        dec("10000"),
        dec("10000"),
        evaluated_at_ms,
    )
    .await?;
    set_paper_balance(
        database_url,
        &s08_account,
        "s08-sell",
        "BTC",
        dec("2"),
        dec("2"),
        evaluated_at_ms,
    )
    .await?;
    let mut s08_paper = PaperCore::acquire(database_url, &s08_account, &s08_instrument).await?;
    let s08_request = ReservePlanRequest {
        request_id: format!("s08-request-{stamp}"),
        plan_id: format!("s08-plan-{stamp}"),
        account_id: s08_account.clone(),
        instrument_id: s08_instrument.clone(),
        opportunity_id: format!("s08-opportunity-{stamp}"),
        strategy_config_version: SIMULATION_CONFIG_VERSION.to_owned(),
        target_quantity: dec(quantity),
        max_unmatched_exposure: dec("100"),
        intents: [
            OrderIntentInput {
                intent_id: format!("s08-intent-buy-{stamp}"),
                leg_id: "buy".to_owned(),
                attempt_id: format!("s08-attempt-buy-{stamp}"),
                client_order_id: format!("s08-client-buy-{stamp}"),
                venue: "s08-buy".to_owned(),
                side: OrderSide::Buy,
                quantity: dec(quantity),
                limit_price: dec("80000"),
            },
            OrderIntentInput {
                intent_id: format!("s08-intent-sell-{stamp}"),
                leg_id: "sell".to_owned(),
                attempt_id: format!("s08-attempt-sell-{stamp}"),
                client_order_id: format!("s08-client-sell-{stamp}"),
                venue: "s08-sell".to_owned(),
                side: OrderSide::Sell,
                quantity: dec(quantity),
                limit_price: dec("80100"),
            },
        ],
        reservations: vec![
            ReservationInput {
                reservation_id: format!("s08-reserve-quote-{stamp}"),
                venue: "s08-buy".to_owned(),
                asset: "USDT".to_owned(),
                amount: dec("80"),
            },
            ReservationInput {
                reservation_id: format!("s08-reserve-base-{stamp}"),
                venue: "s08-sell".to_owned(),
                asset: "BTC".to_owned(),
                amount: dec(quantity),
            },
        ],
    };
    let s08_first = s08_paper.reserve_plan(s08_request.clone()).await?;
    let s08_second = s08_paper.reserve_plan(s08_request).await?;
    s08_paper.disconnect().await;
    let s08_idempotent_replay = !s08_first.idempotent_replay
        && s08_second.idempotent_replay
        && s08_second.plan.plan_id == s08_first.plan.plan_id;

    // S09 恢复一致性：S01 的 run 事实可从三个 core 重读且与报告一致。
    let paper = PaperCore::acquire(database_url, &s01.account_id, &s01.instrument_id).await?;
    let recovered_plans = paper.recover_active_plans().await?;
    paper.disconnect().await;
    let plan = recovered_plans
        .iter()
        .find(|p| p.plan_id == s01.plan_id)
        .context("S09 recovered plan missing")?;
    let order = OrderCore::acquire(database_url, &s01.account_id, &s01.instrument_id).await?;
    let buy_fact = order.load(&s01.buy_intent_id).await?;
    let sell_fact = order.load(&s01.sell_intent_id).await?;
    order.disconnect().await;
    let execution =
        ExecutionCore::acquire(database_url, &s01.account_id, &s01.instrument_id).await?;
    let snapshot = execution.load(&s01.plan_id).await?;
    execution.disconnect().await;
    let s09_fact_recovery_consistent = plan.state == "RESERVED"
        && plan.intents.len() == 2
        && plan
            .reservations
            .iter()
            .all(|r| r.state == "LOCAL_RESERVED")
        && buy_fact.filled_quantity == s01.buy.filled
        && sell_fact.filled_quantity == s01.sell.filled
        && snapshot.leg_a_filled == s01.buy.filled
        && snapshot.leg_b_filled == s01.sell.filled;

    let report = SimulationSmokeReport {
        schema_version: SCHEMA_VERSION,
        s01_full_fill_at_worst,
        s02_partial_fill_depth_shortfall,
        s03_competed_away,
        s04_worse_than_scan_price,
        s05_insufficient_funds_rejected,
        s06_unknown_query_recovered_no_duplicate,
        s07_compensation_over_budget_manual,
        s08_idempotent_replay,
        s09_fact_recovery_consistent,
        external_order_calls: 0,
    };
    if !report.s01_full_fill_at_worst
        || !report.s02_partial_fill_depth_shortfall
        || !report.s03_competed_away
        || !report.s04_worse_than_scan_price
        || !report.s05_insufficient_funds_rejected
        || !report.s06_unknown_query_recovered_no_duplicate
        || !report.s07_compensation_over_budget_manual
        || !report.s08_idempotent_replay
        || !report.s09_fact_recovery_consistent
    {
        bail!("F-02 simulation smoke failed: {report:#?}");
    }
    Ok(report)
}

/// 数据库业务行计数（烟测断言残留用）。
async fn business_count(database_url: &str, table: &str, column: &str, value: &str) -> Result<i64> {
    let pool = ready_pool(database_url).await?;
    let query = format!("SELECT COUNT(*) FROM {table} WHERE {column}=$1");
    let row = sqlx::query(query.as_str())
        .bind(value)
        .fetch_one(&pool)
        .await
        .with_context(|| format!("failed to count {table}.{column}"))?;
    Ok(row.try_get::<i64, _>(0)?)
}

// ---------------------------------------------------------------------------
// 纯逻辑单元测试（无 DB，纳入常规 `cargo test`）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use smoke_fixtures::{dec, smoke_book};

    fn spec(venue: &str) -> InstrumentSpec {
        smoke_fixtures::smoke_spec(venue, "BTCUSDT")
    }

    #[test]
    fn sim_rng_is_deterministic_and_bounded() {
        let mut first = SimRng::new(42);
        let mut second = SimRng::new(42);
        let a: Vec<u64> = (0..8).map(|_| first.next_u64()).collect();
        let b: Vec<u64> = (0..8).map(|_| second.next_u64()).collect();
        assert_eq!(a, b);
        // 闭区间采样（含两端）与分布范围。
        for _ in 0..1000 {
            let value = first.gen_range_u64(50, 400);
            assert!((50..=400).contains(&value));
        }
        let mut third = SimRng::new(0);
        let value = third.gen_range_u64(0, 10_000);
        assert!((0..=10_000).contains(&value));
    }

    #[test]
    fn offset_rounds_buy_up_and_sell_down_to_tick() {
        let book = smoke_book(
            "v",
            "BTCUSDT",
            &[("80000.11", "0.001")],
            &[("80100.11", "0.001")],
            1,
        );
        let config = SimulationConfig {
            adverse_move_bps: dec("100"),
            competitor_take_bps: dec("0"),
            ..SimulationConfig::default()
        };
        // 买：80000.11 × 1.01 = 80800.1111 → 向上圆整到 tick 0.1 → 80800.2
        let buy = simulate_leg(BookSide::Asks, &book, &spec("v"), dec("0.001"), &config).unwrap();
        assert_eq!(buy.avg_price, dec("80800.2"));
        assert_eq!(buy.filled, dec("0.001"));
        // 卖：80100.11 × 0.99 = 79299.1089 → 向下圆整 → 79299.1
        let sell = simulate_leg(BookSide::Bids, &book, &spec("v"), dec("0.001"), &config).unwrap();
        assert_eq!(sell.avg_price, dec("79299.1"));
    }

    #[test]
    fn leg_partial_fill_flags_depth_shortfall() {
        // 档量 0.0006 × (1−0.02) = 0.000588 → 步进 0.0001 向下 = 0.0005 < 目标 0.001。
        let book = smoke_book(
            "v",
            "BTCUSDT",
            &[("80000", "0.0006")],
            &[("80100", "0.002")],
            1,
        );
        let result = simulate_leg(
            BookSide::Asks,
            &book,
            &spec("v"),
            dec("0.001"),
            &SimulationConfig::default(),
        )
        .unwrap();
        assert_eq!(result.filled, dec("0.0005"));
        assert!(result.depth_shortfall);
        assert_eq!(result.last_raw_price, dec("80000"));
        assert_eq!(result.quote_amount, result.avg_price * result.filled);
    }

    #[test]
    fn leg_competes_away_when_take_rate_floors_to_zero() {
        let book = smoke_book(
            "v",
            "BTCUSDT",
            &[("80000", "0.002")],
            &[("80100", "0.002")],
            1,
        );
        let config = SimulationConfig {
            competitor_take_bps: dec("10000"),
            ..SimulationConfig::default()
        };
        let result =
            simulate_leg(BookSide::Asks, &book, &spec("v"), dec("0.001"), &config).unwrap();
        assert_eq!(result.filled, Decimal::ZERO);
        assert!(!result.depth_shortfall);
        assert_eq!(result.avg_price, Decimal::ZERO);
    }

    #[test]
    fn leg_rejects_below_min_quantity() {
        // 可得 0.0002 < min_quantity 0.001 → 整腿拒绝（无成交）。
        let mut big_min = spec("v");
        big_min.min_quantity = dec("0.001");
        let book = smoke_book(
            "v",
            "BTCUSDT",
            &[("80000", "0.0002")],
            &[("80100", "0.002")],
            1,
        );
        let result = simulate_leg(
            BookSide::Asks,
            &book,
            &big_min,
            dec("0.001"),
            &SimulationConfig::default(),
        )
        .unwrap();
        assert_eq!(result.filled, Decimal::ZERO);
        assert!(result.below_min_quantity);
    }

    #[test]
    fn run_report_serializes_as_snake_case() {
        // 报告 DTO 序列化契约：场景/执行状态为 SCREAMING_SNAKE_CASE，Decimal 为字符串。
        let report = SimulationRunReport {
            schema_version: 4,
            seed: 0,
            run_id: "f02-1-1".into(),
            symbol: "BTCUSDT".into(),
            quantity: dec("0.001"),
            buy_venue: "a".into(),
            sell_venue: "b".into(),
            scenario: SimulationScenario::DepthShortfall,
            rejection_reason: None,
            plan_id: "p".into(),
            account_id: "acct".into(),
            instrument_id: "inst".into(),
            buy_intent_id: "bi".into(),
            sell_intent_id: "si".into(),
            buy_client_order_id: "bc".into(),
            sell_client_order_id: "sc".into(),
            evaluated_at_ms: 1,
            decision_to_submit_ms: 50,
            fill_latency_ms: 20,
            buy: SimLegResult {
                filled: dec("0.0005"),
                quote_amount: dec("40"),
                avg_price: dec("80000"),
                last_raw_price: dec("80000"),
                last_fill_price: dec("80000"),
                depth_shortfall: true,
                below_min_quantity: false,
            },
            sell: SimLegResult {
                filled: dec("0.001"),
                quote_amount: dec("80.1"),
                avg_price: dec("80100"),
                last_raw_price: dec("80100"),
                last_fill_price: dec("80100"),
                depth_shortfall: false,
                below_min_quantity: false,
            },
            buy_fee: dec("0.04"),
            sell_fee: dec("0.08"),
            scanned_net_profit: dec("0.1"),
            simulated_net_profit: dec("-0.02"),
            adverse_move_bps: dec("15"),
            competitor_take_bps: dec("200"),
            compensation_decision: "COMPENSATION_PLANNED".into(),
            execution_state: "COMPENSATION_PLANNED".into(),
            unmatched_quantity: dec("0.0005"),
            unmatched_exposure: dec("40"),
            estimated_compensation_cost: dec("40.1"),
            unknown_submit_tried: false,
            query_found: false,
            idempotent_replay: false,
            skipped_frames: 0,
            warnings: Vec::new(),
            external_order_calls: 0,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["scenario"], "DEPTH_SHORTFALL");
        assert_eq!(json["execution_state"], "COMPENSATION_PLANNED");
        assert_eq!(json["buy"]["avg_price"], "80000");
        assert_eq!(json["external_order_calls"], 0);
    }

    /// G-01 测试用最小报告骨架（成交字段可定制）。
    fn g01_report(buy_cost: Decimal, sell_filled: Decimal) -> SimulationRunReport {
        SimulationRunReport {
            schema_version: SCHEMA_VERSION,
            seed: 0,
            run_id: "f02-1-1".into(),
            symbol: "BTCUSDT".into(),
            quantity: dec("0.001"),
            buy_venue: "venue-a".into(),
            sell_venue: "venue-b".into(),
            scenario: SimulationScenario::DepthShortfall,
            rejection_reason: None,
            plan_id: "p".into(),
            account_id: "acct".into(),
            instrument_id: "inst".into(),
            buy_intent_id: "bi".into(),
            sell_intent_id: "si".into(),
            buy_client_order_id: "bc".into(),
            sell_client_order_id: "sc".into(),
            evaluated_at_ms: 1,
            decision_to_submit_ms: 50,
            fill_latency_ms: 20,
            buy: SimLegResult {
                filled: dec("0.0005"),
                quote_amount: buy_cost,
                avg_price: dec("80000"),
                last_raw_price: dec("80000"),
                last_fill_price: dec("80000"),
                depth_shortfall: true,
                below_min_quantity: false,
            },
            sell: SimLegResult {
                filled: sell_filled,
                quote_amount: dec("80.1"),
                avg_price: dec("80100"),
                last_raw_price: dec("80100"),
                last_fill_price: dec("80100"),
                depth_shortfall: false,
                below_min_quantity: false,
            },
            buy_fee: dec("0.4"),
            sell_fee: dec("0.08"),
            scanned_net_profit: dec("0.1"),
            simulated_net_profit: dec("-0.02"),
            adverse_move_bps: dec("15"),
            competitor_take_bps: dec("200"),
            compensation_decision: "COMPENSATION_PLANNED".into(),
            execution_state: "COMPENSATION_PLANNED".into(),
            unmatched_quantity: dec("0.0005"),
            unmatched_exposure: dec("40"),
            estimated_compensation_cost: dec("40.1"),
            unknown_submit_tried: false,
            query_found: false,
            idempotent_replay: false,
            skipped_frames: 0,
            warnings: Vec::new(),
            external_order_calls: 0,
        }
    }

    #[test]
    fn settlement_points_tracks_fund_filled_evaluated() {
        let report = g01_report(dec("400"), dec("1.5"));
        let points = settlement_points(&report, dec("1000"), dec("2"));
        // fund：注入额原值。
        assert_eq!(points[0], (dec("1000"), dec("2")));
        // filled：quote 减买腿成本+费用；base 减卖腿交割。
        assert_eq!(points[1], (dec("599.6"), dec("0.5")));
        // evaluated：同 filled（补偿为现金评估，不落地）。
        assert_eq!(points[2], points[1]);
    }

    #[test]
    fn settlement_points_clamps_overdraft_to_zero() {
        // 模拟撮合超出注入资金（预检使用估计值，撮合逐档实际值可超支）。
        let report = g01_report(dec("1200"), dec("7"));
        let points = settlement_points(&report, dec("1000"), dec("2"));
        assert_eq!(points[0], (dec("1000"), dec("2")));
        // 负值 clamp 到 0（balance_snapshots.total >= 0 CHECK）。
        assert_eq!(points[1], (Decimal::ZERO, Decimal::ZERO));
        assert_eq!(points[2], points[1]);
    }

    #[test]
    fn scenario_db_value_covers_full_check_value_domain() {
        // 与 0005_simulation_runs.sql 的 scenario CHECK 字面量逐一对齐。
        for (scenario, expected) in [
            (SimulationScenario::Normal, "NORMAL"),
            (SimulationScenario::DepthShortfall, "DEPTH_SHORTFALL"),
            (SimulationScenario::CompetedAway, "COMPETED_AWAY"),
            (SimulationScenario::Rejected, "REJECTED"),
        ] {
            assert_eq!(scenario_db_value(scenario), expected);
        }
    }
}
