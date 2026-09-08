# C-01 Tauri 桌面控制台：任务文档

- 状态：`verified`
- 上游：A-03、A-04、A-05、B-01、B-02、B-03
- 安全边界：Tauri 桌面端；只读公共行情、只读账户元数据和 PostgreSQL/PAPER；无真实订单

## 1. 任务清单

| 阶段 | 任务 | 产出 | 状态 |
|---|---|---|---|
| 需求 | 定义桌面操作、展示和失败关闭结果 | C-01 需求文档 | completed |
| 分析 | 核对 Rust 编排与 Tauri/Vue 工程约束 | 迁移约束 | completed |
| 设计 | 定义 command、DTO、页面和删除旧路径 | C-01 设计文档 | completed |
| 实现 | 将 Rust 主入口迁移为 Tauri 应用 | `src-tauri/src/main.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/commands/`、`src-tauri/tauri.conf.json` | verified |
| 实现 | 创建 Vue + Element Plus + TypeScript 前端 | `src/`、`ui/` | verified |
| 实现 | 接通配置、账户、观测、回放和 PAPER 操作 | Tauri invoke 调用与 `ApiResponse<T>` | verified |
| 验证 | 开发、构建前端、Rust 应用并执行命令协议烟测 | `npm run tauri dev`、`npm run build`、`cargo fmt --all -- --check`、`cargo test --workspace`、`cargo build --workspace --release` | verified |
| 发布 | 更新架构与 SDLC 记录 | C-01 设计文档、运行命令与目录边界 | verified |
