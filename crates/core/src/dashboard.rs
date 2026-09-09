//! 实时监控仪表盘模块，用于系统状态可视化。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 系统状态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemStatus {
    /// 运行时间（秒）
    pub uptime_seconds: u64,
    /// 内存使用（字节）
    pub memory_usage_bytes: u64,
    /// CPU使用率（百分比）
    pub cpu_usage_percent: f64,
    /// 磁盘使用（字节）
    pub disk_usage_bytes: u64,
}

/// 交易状态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TradingStatus {
    /// 订单数量
    pub order_count: i64,
    /// 成交数量
    pub trade_count: i64,
    /// 收益
    pub pnl: f64,
    /// 活跃订单数
    pub active_orders: i64,
}

/// 市场状态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketStatus {
    /// 行情延迟（毫秒）
    pub market_data_latency_ms: f64,
    /// 行情断档（秒）
    pub market_data_gap_seconds: f64,
    /// 订单簿深度
    pub orderbook_depth: i64,
}

/// 风险状态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RiskStatus {
    /// 敞口（美元）
    pub exposure_usd: f64,
    /// 保证金使用率（百分比）
    pub margin_usage_percent: f64,
    /// 风险事件数量
    pub risk_event_count: i64,
}

/// 仪表盘数据
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardData {
    /// 系统状态
    pub system: SystemStatus,
    /// 交易状态
    pub trading: TradingStatus,
    /// 市场状态
    pub market: MarketStatus,
    /// 风险状态
    pub risk: RiskStatus,
    /// 时间戳
    pub timestamp_ms: u64,
}

/// 仪表盘管理器
pub struct DashboardManager {
    data: Arc<RwLock<DashboardData>>,
}

impl DashboardManager {
    /// 创建新的仪表盘管理器
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(DashboardData::default())),
        }
    }

    /// 更新系统状态
    pub async fn update_system_status(&self, status: SystemStatus) {
        let mut data = self.data.write().await;
        data.system = status;
        data.timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }

    /// 更新交易状态
    pub async fn update_trading_status(&self, status: TradingStatus) {
        let mut data = self.data.write().await;
        data.trading = status;
        data.timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }

    /// 更新市场状态
    pub async fn update_market_status(&self, status: MarketStatus) {
        let mut data = self.data.write().await;
        data.market = status;
        data.timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }

    /// 更新风险状态
    pub async fn update_risk_status(&self, status: RiskStatus) {
        let mut data = self.data.write().await;
        data.risk = status;
        data.timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }

    /// 获取仪表盘数据
    pub async fn get_dashboard_data(&self) -> DashboardData {
        self.data.read().await.clone()
    }

    /// 获取系统状态
    pub async fn get_system_status(&self) -> SystemStatus {
        self.data.read().await.system.clone()
    }

    /// 获取交易状态
    pub async fn get_trading_status(&self) -> TradingStatus {
        self.data.read().await.trading.clone()
    }

    /// 获取市场状态
    pub async fn get_market_status(&self) -> MarketStatus {
        self.data.read().await.market.clone()
    }

    /// 获取风险状态
    pub async fn get_risk_status(&self) -> RiskStatus {
        self.data.read().await.risk.clone()
    }
}

impl Default for DashboardManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 仪表盘测试套件
pub struct DashboardTestSuite {
    tests: Vec<DashboardTest>,
}

struct DashboardTest {
    name: String,
    #[expect(dead_code)]
    description: String,
    #[expect(dead_code)]
    expected_behavior: String,
}

impl DashboardTestSuite {
    /// 创建默认测试套件
    pub fn new_default() -> Self {
        let tests = vec![
            DashboardTest {
                name: "system_status_update".to_string(),
                description: "系统状态更新测试".to_string(),
                expected_behavior: "系统状态更新成功".to_string(),
            },
            DashboardTest {
                name: "trading_status_update".to_string(),
                description: "交易状态更新测试".to_string(),
                expected_behavior: "交易状态更新成功".to_string(),
            },
            DashboardTest {
                name: "market_status_update".to_string(),
                description: "市场状态更新测试".to_string(),
                expected_behavior: "市场状态更新成功".to_string(),
            },
            DashboardTest {
                name: "risk_status_update".to_string(),
                description: "风险状态更新测试".to_string(),
                expected_behavior: "风险状态更新成功".to_string(),
            },
        ];

        Self { tests }
    }

    /// 运行所有测试
    pub async fn run_all(&self) -> Result<Vec<DashboardTestResult>> {
        let mut results = Vec::new();

        for test in &self.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行单个测试
    async fn run_test(&self, test: &DashboardTest) -> Result<DashboardTestResult> {
        let manager = DashboardManager::new();

        // 更新系统状态
        let system_status = SystemStatus {
            uptime_seconds: 3600,
            memory_usage_bytes: 1024 * 1024 * 100,
            cpu_usage_percent: 25.0,
            disk_usage_bytes: 1024 * 1024 * 1024 * 10,
        };
        manager.update_system_status(system_status).await;

        // 更新交易状态
        let trading_status = TradingStatus {
            order_count: 100,
            trade_count: 50,
            pnl: 1000.0,
            active_orders: 10,
        };
        manager.update_trading_status(trading_status).await;

        // 更新市场状态
        let market_status = MarketStatus {
            market_data_latency_ms: 50.0,
            market_data_gap_seconds: 0.0,
            orderbook_depth: 100,
        };
        manager.update_market_status(market_status).await;

        // 更新风险状态
        let risk_status = RiskStatus {
            exposure_usd: 5000.0,
            margin_usage_percent: 30.0,
            risk_event_count: 0,
        };
        manager.update_risk_status(risk_status).await;

        // 获取数据
        let data = manager.get_dashboard_data().await;
        let passed = data.system.uptime_seconds == 3600
            && data.trading.order_count == 100
            && data.market.market_data_latency_ms == 50.0
            && data.risk.exposure_usd == 5000.0;

        Ok(DashboardTestResult {
            test_name: test.name.clone(),
            passed,
            data_updates: 4,
            data_queries: 4,
        })
    }
}

/// 仪表盘测试结果
#[derive(Debug, Clone, Serialize)]
pub struct DashboardTestResult {
    pub test_name: String,
    pub passed: bool,
    pub data_updates: usize,
    pub data_queries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dashboard_manager_new() {
        let manager = DashboardManager::new();
        let data = manager.get_dashboard_data().await;
        assert_eq!(data.system.uptime_seconds, 0);
        assert_eq!(data.trading.order_count, 0);
        assert_eq!(data.market.market_data_latency_ms, 0.0);
        assert_eq!(data.risk.exposure_usd, 0.0);
    }

    #[tokio::test]
    async fn dashboard_manager_update_system_status() {
        let manager = DashboardManager::new();
        let status = SystemStatus {
            uptime_seconds: 3600,
            memory_usage_bytes: 1024 * 1024 * 100,
            cpu_usage_percent: 25.0,
            disk_usage_bytes: 1024 * 1024 * 1024 * 10,
        };
        manager.update_system_status(status).await;
        let system_status = manager.get_system_status().await;
        assert_eq!(system_status.uptime_seconds, 3600);
    }

    #[tokio::test]
    async fn dashboard_manager_update_trading_status() {
        let manager = DashboardManager::new();
        let status = TradingStatus {
            order_count: 100,
            trade_count: 50,
            pnl: 1000.0,
            active_orders: 10,
        };
        manager.update_trading_status(status).await;
        let trading_status = manager.get_trading_status().await;
        assert_eq!(trading_status.order_count, 100);
    }

    #[tokio::test]
    async fn dashboard_manager_update_market_status() {
        let manager = DashboardManager::new();
        let status = MarketStatus {
            market_data_latency_ms: 50.0,
            market_data_gap_seconds: 0.0,
            orderbook_depth: 100,
        };
        manager.update_market_status(status).await;
        let market_status = manager.get_market_status().await;
        assert_eq!(market_status.market_data_latency_ms, 50.0);
    }

    #[tokio::test]
    async fn dashboard_manager_update_risk_status() {
        let manager = DashboardManager::new();
        let status = RiskStatus {
            exposure_usd: 5000.0,
            margin_usage_percent: 30.0,
            risk_event_count: 0,
        };
        manager.update_risk_status(status).await;
        let risk_status = manager.get_risk_status().await;
        assert_eq!(risk_status.exposure_usd, 5000.0);
    }

    #[test]
    fn dashboard_test_suite_new_default() {
        let suite = DashboardTestSuite::new_default();
        assert_eq!(suite.tests.len(), 4);
    }

    #[tokio::test]
    async fn dashboard_test_suite_run_all() {
        let suite = DashboardTestSuite::new_default();
        let results = suite.run_all().await.unwrap();
        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|r| r.passed));
    }
}
