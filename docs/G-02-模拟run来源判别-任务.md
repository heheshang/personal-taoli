# G-02 模拟 run 来源判别：任务文档

文档状态：待所有者评审（2026-09-10）。实现顺序与验收见需求/设计文档；本文件为文件级执行清单与验证证据留存。

**流程偏差**：本任务文档系事后补正，按已交付改动逐文件回溯整理。

## 1. 文件级改动清单

### 1.1 新增迁移 `crates/core/migrations/0008_simulation_run_source.sql`

```sql
ALTER TABLE simulation_runs
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'SMOKE'
    CHECK (source IN ('SMOKE', 'LIVE'));

ALTER TABLE simulation_runs
    ALTER COLUMN source SET DEFAULT 'LIVE';

CREATE INDEX IF NOT EXISTS simulation_runs_source_clock_idx
    ON simulation_runs (source, executed_at_ms DESC);
```

文件头注释记录**为什么不用 `UPDATE` 回填**（`0005` 的 `simulation_runs_immutable` 会拒绝）以及**为什么既存行都是探针**（实时 accepted 恒 0）。三个 `ALTER` 均可重复执行（`IF NOT EXISTS` / `SET DEFAULT` 幂等）。

### 1.2 `crates/core/src/simulation.rs`

- 新增 `pub enum SimulationRunSource { Smoke, Live }`（`as_str()` → `"SMOKE"`/`"LIVE"`），带文档说明二者语义差别。
- `SimulationRunReport` 增 `pub source: SimulationRunSource`（紧邻 `run_id`）。
- `run_simulation_run` 增第 6 个形参 `source: SimulationRunSource`，并在报告构造处赋值。
- 7 个烟测调用点显式传 `SimulationRunSource::Smoke`；`run_all_directions`（实时路径）传 `SimulationRunSource::Live`。
- `persist_simulation_run`：INSERT 增 `source` 列与 `$41` 占位，绑定 `report.source.as_str()`。
- `run_simulation_smoke` 首行新增 `tracing::info!` 横幅：`SYNTHETIC FIXTURES (venues s01..s09, symbol label BTCUSDT) — not market data; persisted runs are marked source=SMOKE`。
- 2 处测试内 `SimulationRunReport` 字面量补 `source` 字段。

### 1.3 `crates/core/src/simulation_query.rs`

- `RUN_PROJECTION` 追加 `, source`（第 23 列）。
- `run_row_at` 增 `source: row.try_get(base + 22)?`。
- `SimulationRunRow` 增 `pub source: String`。
- 详情查询的列偏移整体 +1（`report` 22→23，`plan_id` 23→24，…，`account_id` 26→27）。
- 概览 `agg` 查询追加两个 `COUNT(*) FILTER (WHERE source = 'SMOKE'|'LIVE')::BIGINT`；解出 `smoke_runs` / `live_runs`。
- `SimulationOverview` 增两字段并在构造处填入。
- DTO 往返测试字面量补 `smoke_runs: 3, live_runs: 0`。

### 1.4 `src/types.ts`

`SimulationOverview` 增 `smoke_runs` / `live_runs: number`；`SimulationRunRow` 增 `source: string`。

### 1.5 `src/components/SimulationStatCards.vue`

- `stats` 增 `smokeRuns` / `liveRuns`。
- 新增 `provenance` computed：三种分支（全探针 → 警示态 + 「实时 run 0 条 —— 准入未通过（凭证/资格未配置，见控制台账户状态）」；全实时 → 正常态；混合 → 中性态），空库返回 `null` 不渲染。
- 模板在场景标签前插入来源提示区块。
- 样式新增 `.provenance` 及 `.warn` / `.mixed` / `.live` 三态配色（warn 用 danger 色 8% 底色 + 45% 边框）。

### 1.6 `src/components/SimulationTrades.vue`

- 新增 `sourceTag(src)` 返回 `{type, text}`：`SMOKE` → `info`「合成簿」，其余 → `success`「实时」。
- 列表 RUN 列在 run_id 下方插入来源徽章。
- 详情抽屉标题栏插入同一徽章。

### 1.7 `src/components/SimulationPage.vue`

- 页头说明去掉会造成混淆的「F-02 合成簿仿真」，改为「只读仪表盘 · 无真实/测试网订单路径」。
- 按钮文案「运行一次烟测」→「运行合成簿烟测」，并在其右新增说明「烟测用合成簿（venues s01–s09，symbol 为夹具常量），不是市场数据」。
- 新增 `.probe-note` 样式。

## 2. 验证证据

### 2.1 五门禁

```bash
cargo fmt --all -- --check                              # PASS
cargo test --workspace                                  # 192 core + 8 tauri-lib passed; 0 failed
cargo clippy --workspace --all-targets -- -D warnings   # 零警告
cargo build --workspace --release                       # Finished `release` profile in 2m 03s
npm run build                                           # ✓ built in 2.58s
```

未新增测试：本迭代性质为「来源标注 + 计数透出」，其可观察结果需要真实 DB 与真实渲染；已用一次实跑烟测 + 一次 mock IPC 渲染断言覆盖，未新增纯逻辑测试。

### 2.2 实库（`taoli-postgres`，55432）

一次性驱动 `crates/core/examples/src_probe.rs`（**验证后已删除**）：

```
before: total=613 smoke=613 live=0
run_simulation_smoke → external_order_calls=0
after : total=620 smoke=620 live=0
newest source=SMOKE (must be SMOKE)
```

保护未被削弱：

```
pg_trigger simulation_runs_immutable      → 仍存在
UPDATE simulation_runs SET source='LIVE'  → ERROR: audit_events are immutable
column_default 'LIVE', is_nullable NO
```

页内过滤值域：

```
filter BTCUSDT -> 627 | ETHUSDT/BNBUSDT/SOLUSDT/XRPUSDT/DOGEUSDT/ADAUSDT/AVAXUSDT -> 0
no filter -> 627
```

### 2.3 界面渲染（mock IPC，一次性 harness 已删除）

以桩 `__TAURI_INTERNALS__.invoke` 提供罐头数据，打开 `ui/` 构建产物、切换到模拟套利页后读取页面文本：

```
运行合成簿烟测
烟测用合成簿（venues s01–s09，symbol 为夹具常量），不是市场数据
数据来源
全部 620 条为合成簿探针（venues s01–s09，非市场数据）；实时 run 0 条 —— 准入未通过（凭证/资格未配置，见控制台账户状态）
正常成交 × 265 / 深度不足 × 173 / 被抢单 × 92 / 拒绝 × 83
f02-1789033590398-73256  [合成簿]  2026-09-10 17:46  …
```

IPC 调用序列确认页面确实请求了 `get_simulation_overview_command` 与 `get_observer_config`。

**视觉确认**：截图经视觉模型读图确认为粉红底 + 深红字的警示条，与周围白色面板显著区分。

### 2.4 未执行的验证（明确记录）

- **真实 Tauri 宿主未验证**：渲染断言走 mock IPC（仓库既有做法），真实 WKWebView 中的观感由所有者在自己运行的实例上确认。
- **`LIVE` 分支未实测**：库中不存在实时 run（accepted 恒 0），故 `source='LIVE'` 的落库路径与「全实时 / 混合」两种提示分支只有代码层面保证，未经真实数据触发。这是本迭代最主要的验证缺口。
- **`Dirty` 人工处置仍未实测**：本次失败走回滚路径（见设计 §6），非 `Dirty`。

## 3. 失败与修正记录

**迁移 0008 首次实现失败**：用 `UPDATE … WHERE buy_venue NOT IN ('binance','bybit')` 回填，被 `simulation_runs_immutable` 拒绝，应用侧报 `SIMULATION_ERROR: failed to apply database migrations`。

修正：改用 `ADD COLUMN … DEFAULT 'SMOKE'` + `SET DEFAULT 'LIVE'`，不触碰触发器。

现场核查结论：失败**零残留**（`_sqlx_migrations` 无 version=8 行、`source` 列不存在、1–7 全部 `success`），因 sqlx 把每个迁移文件与其记账放在同一事务。修正文件后直接重跑即成功。

**教训**：本项目所有事实表都有 `BEFORE UPDATE OR DELETE` 不可变触发器（`0001`/`0002`/`0003`/`0004`/`0005` 共 11 个），因此**后续迁移不得对既有事实行做 `UPDATE`**；需要回填时一律通过默认值、新列或新表完成。

## 4. 文档同步

| 文档 | 变更 |
|---|---|
| `docs/个人加密货币套利系统-AIDLC迭代执行手册.md` | §6.8 追踪矩阵、§10 发布记录、§11 下一轮唯一入口 |
| `docs/个人加密货币套利系统-架构与需求设计.md` | §19.1 当前实现状态追加 G-02 条目与链接 |
| `docs/MIG-01-sqlx迁移统一-设计.md` | §7 R-3 补入首次实测证据（失败走回滚而非 `Dirty`），风险等级下调但保留 |
| 本迭代三份文档 | 新增 |

## 5. 清理项

| 项 | 状态 |
|---|---|
| `crates/core/examples/src_probe.rs` + 空目录 | 已删除 |
| `mock_render_probe.py`（一次性 mock IPC 服务） | 已删除 |
| mock IPC 服务进程 `sim-mock-render` | 已停止 |
| mock 渲染用的浏览器标签 | 已释放 |

## 6. 剩余风险与后续入口

剩余风险见设计文档 §7，其中最关键的是 **R-6 / §2.4 的 `LIVE` 分支未实测**——实时 run 为 0 的根因是准入失败关闭（凭证、资格、费率），这属于阶段 F 的前置条件，不在本迭代范围。

后续入口不变：下一轮商业迭代仍唯一进入 F 阶段后续迭代（如 F-03 生产部署）或经所有者批准进入生产环境。
