//! 告警管理模块，用于告警的创建、确认和解决。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 告警级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertLevel {
    /// P0: 无法确认的大额敞口、保证金危险、交易核心失联且有仓位、持久化失效
    Critical,
    /// P1: 未知订单超时、私有流中断、余额冲突、持续单场所故障
    High,
    /// P2: 库存偏移、行情延迟升高、归档积压、收益估计误差变大
    Medium,
    /// 信息: 正常启停、配置批准、日结、再平衡到账
    Info,
}

impl AlertLevel {
    /// 获取告警级别名称
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertLevel::Critical => "CRITICAL",
            AlertLevel::High => "HIGH",
            AlertLevel::Medium => "MEDIUM",
            AlertLevel::Info => "INFO",
        }
    }
}

/// 告警状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertStatus {
    /// 新告警
    Pending,
    /// 已确认
    Acknowledged,
    /// 已处理
    Resolved,
    /// 已忽略
    Ignored,
}

impl AlertStatus {
    /// 获取告警状态名称
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertStatus::Pending => "PENDING",
            AlertStatus::Acknowledged => "ACKNOWLEDGED",
            AlertStatus::Resolved => "RESOLVED",
            AlertStatus::Ignored => "IGNORED",
        }
    }
}

/// 告警事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    /// 告警ID
    pub alert_id: String,
    /// 告警级别
    pub level: AlertLevel,
    /// 告警标题
    pub title: String,
    /// 告警消息
    pub message: String,
    /// 告警状态
    pub status: AlertStatus,
    /// 告警来源
    pub source: String,
    /// 告警标签
    pub labels: HashMap<String, String>,
    /// 创建时间
    pub created_at_ms: u64,
    /// 更新时间
    pub updated_at_ms: Option<u64>,
    /// 确认时间
    pub acknowledged_at_ms: Option<u64>,
    /// 解决时间
    pub resolved_at_ms: Option<u64>,
}

/// 告警管理器
pub struct AlertManager {
    alerts: Arc<RwLock<HashMap<String, AlertEvent>>>,
}

impl AlertManager {
    /// 创建新的告警管理器
    pub fn new() -> Self {
        Self {
            alerts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 创建告警
    pub async fn create_alert(
        &self,
        level: AlertLevel,
        title: String,
        message: String,
        source: String,
        labels: HashMap<String, String>,
        now_ms: u64,
    ) -> AlertEvent {
        let alert_id = format!("alert-{}-{}", now_ms, self.alerts.read().await.len());
        let event = AlertEvent {
            alert_id: alert_id.clone(),
            level,
            title,
            message,
            status: AlertStatus::Pending,
            source,
            labels,
            created_at_ms: now_ms,
            updated_at_ms: None,
            acknowledged_at_ms: None,
            resolved_at_ms: None,
        };
        self.alerts.write().await.insert(alert_id, event.clone());
        event
    }

    /// 确认告警
    pub async fn acknowledge_alert(&self, alert_id: &str, now_ms: u64) -> Result<AlertEvent> {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.get_mut(alert_id) {
            alert.status = AlertStatus::Acknowledged;
            alert.acknowledged_at_ms = Some(now_ms);
            alert.updated_at_ms = Some(now_ms);
            Ok(alert.clone())
        } else {
            Err(anyhow::anyhow!("alert not found"))
        }
    }

    /// 解决告警
    pub async fn resolve_alert(&self, alert_id: &str, now_ms: u64) -> Result<AlertEvent> {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.get_mut(alert_id) {
            alert.status = AlertStatus::Resolved;
            alert.resolved_at_ms = Some(now_ms);
            alert.updated_at_ms = Some(now_ms);
            Ok(alert.clone())
        } else {
            Err(anyhow::anyhow!("alert not found"))
        }
    }

    /// 忽略告警
    pub async fn ignore_alert(&self, alert_id: &str, now_ms: u64) -> Result<AlertEvent> {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.get_mut(alert_id) {
            alert.status = AlertStatus::Ignored;
            alert.updated_at_ms = Some(now_ms);
            Ok(alert.clone())
        } else {
            Err(anyhow::anyhow!("alert not found"))
        }
    }

    /// 获取所有告警
    pub async fn get_alerts(&self) -> Vec<AlertEvent> {
        self.alerts.read().await.values().cloned().collect()
    }

    /// 获取待处理告警
    pub async fn get_pending_alerts(&self) -> Vec<AlertEvent> {
        self.alerts
            .read()
            .await
            .values()
            .filter(|a| a.status == AlertStatus::Pending)
            .cloned()
            .collect()
    }

    /// 获取特定级别的告警
    pub async fn get_alerts_by_level(&self, level: &AlertLevel) -> Vec<AlertEvent> {
        self.alerts
            .read()
            .await
            .values()
            .filter(|a| a.level == *level)
            .cloned()
            .collect()
    }

    /// 获取特定状态的告警
    pub async fn get_alerts_by_status(&self, status: &AlertStatus) -> Vec<AlertEvent> {
        self.alerts
            .read()
            .await
            .values()
            .filter(|a| a.status == *status)
            .cloned()
            .collect()
    }

    /// 清除所有告警
    pub async fn clear_alerts(&self) {
        self.alerts.write().await.clear();
    }
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 告警管理测试套件
pub struct AlertManagementTestSuite {
    tests: Vec<AlertManagementTest>,
}

struct AlertManagementTest {
    name: String,
    #[expect(dead_code)]
    description: String,
    #[expect(dead_code)]
    expected_behavior: String,
}

impl AlertManagementTestSuite {
    /// 创建默认测试套件
    pub fn new_default() -> Self {
        let tests = vec![
            AlertManagementTest {
                name: "alert_creation".to_string(),
                description: "告警创建测试".to_string(),
                expected_behavior: "告警创建成功".to_string(),
            },
            AlertManagementTest {
                name: "alert_acknowledgment".to_string(),
                description: "告警确认测试".to_string(),
                expected_behavior: "告警确认成功".to_string(),
            },
            AlertManagementTest {
                name: "alert_resolution".to_string(),
                description: "告警解决测试".to_string(),
                expected_behavior: "告警解决成功".to_string(),
            },
            AlertManagementTest {
                name: "alert_queries".to_string(),
                description: "告警查询测试".to_string(),
                expected_behavior: "告警查询功能正常".to_string(),
            },
        ];

        Self { tests }
    }

    /// 运行所有测试
    pub async fn run_all(&self) -> Result<Vec<AlertManagementTestResult>> {
        let mut results = Vec::new();

        for test in &self.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行单个测试
    async fn run_test(&self, test: &AlertManagementTest) -> Result<AlertManagementTestResult> {
        let manager = AlertManager::new();

        // 创建告警
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        let alert = manager
            .create_alert(
                AlertLevel::Critical,
                "Test Alert".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;

        // 确认告警
        manager.acknowledge_alert(&alert.alert_id, 2000).await?;

        // 解决告警
        manager.resolve_alert(&alert.alert_id, 3000).await?;

        // 查询告警
        let alerts = manager.get_alerts().await;
        let pending = manager.get_pending_alerts().await;
        let critical = manager.get_alerts_by_level(&AlertLevel::Critical).await;

        let passed = alerts.len() == 1
            && pending.is_empty()
            && !critical.is_empty()
            && alerts[0].status == AlertStatus::Resolved;

        Ok(AlertManagementTestResult {
            test_name: test.name.clone(),
            passed,
            alerts_created: 1,
            alerts_acknowledged: 1,
            alerts_resolved: 1,
        })
    }
}

/// 告警管理测试结果
#[derive(Debug, Clone, Serialize)]
pub struct AlertManagementTestResult {
    pub test_name: String,
    pub passed: bool,
    pub alerts_created: usize,
    pub alerts_acknowledged: usize,
    pub alerts_resolved: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_level_as_str() {
        assert_eq!(AlertLevel::Critical.as_str(), "CRITICAL");
        assert_eq!(AlertLevel::High.as_str(), "HIGH");
        assert_eq!(AlertLevel::Medium.as_str(), "MEDIUM");
        assert_eq!(AlertLevel::Info.as_str(), "INFO");
    }

    #[test]
    fn alert_status_as_str() {
        assert_eq!(AlertStatus::Pending.as_str(), "PENDING");
        assert_eq!(AlertStatus::Acknowledged.as_str(), "ACKNOWLEDGED");
        assert_eq!(AlertStatus::Resolved.as_str(), "RESOLVED");
        assert_eq!(AlertStatus::Ignored.as_str(), "IGNORED");
    }

    #[tokio::test]
    async fn alert_manager_new() {
        let manager = AlertManager::new();
        let alerts = manager.get_alerts().await;
        assert!(alerts.is_empty());
    }

    #[tokio::test]
    async fn alert_manager_create_alert() {
        let manager = AlertManager::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        let alert = manager
            .create_alert(
                AlertLevel::Critical,
                "Test Alert".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;
        assert_eq!(alert.status, AlertStatus::Pending);
        let alerts = manager.get_alerts().await;
        assert_eq!(alerts.len(), 1);
    }

    #[tokio::test]
    async fn alert_manager_acknowledge_alert() {
        let manager = AlertManager::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        let alert = manager
            .create_alert(
                AlertLevel::Critical,
                "Test Alert".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;
        manager
            .acknowledge_alert(&alert.alert_id, 2000)
            .await
            .unwrap();
        let alerts = manager.get_alerts().await;
        assert_eq!(alerts[0].status, AlertStatus::Acknowledged);
    }

    #[tokio::test]
    async fn alert_manager_resolve_alert() {
        let manager = AlertManager::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        let alert = manager
            .create_alert(
                AlertLevel::Critical,
                "Test Alert".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;
        manager.resolve_alert(&alert.alert_id, 2000).await.unwrap();
        let alerts = manager.get_alerts().await;
        assert_eq!(alerts[0].status, AlertStatus::Resolved);
    }

    #[tokio::test]
    async fn alert_manager_ignore_alert() {
        let manager = AlertManager::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        let alert = manager
            .create_alert(
                AlertLevel::Info,
                "Test Alert".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;
        manager.ignore_alert(&alert.alert_id, 2000).await.unwrap();
        let alerts = manager.get_alerts().await;
        assert_eq!(alerts[0].status, AlertStatus::Ignored);
    }

    #[tokio::test]
    async fn alert_manager_get_pending_alerts() {
        let manager = AlertManager::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        manager
            .create_alert(
                AlertLevel::Critical,
                "Test Alert 1".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels.clone(),
                1000,
            )
            .await;
        manager
            .create_alert(
                AlertLevel::High,
                "Test Alert 2".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;
        let pending = manager.get_pending_alerts().await;
        assert_eq!(pending.len(), 2);
    }

    #[tokio::test]
    async fn alert_manager_get_alerts_by_level() {
        let manager = AlertManager::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        manager
            .create_alert(
                AlertLevel::Critical,
                "Test Alert 1".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels.clone(),
                1000,
            )
            .await;
        manager
            .create_alert(
                AlertLevel::High,
                "Test Alert 2".to_string(),
                "This is a test alert".to_string(),
                "test".to_string(),
                labels,
                1000,
            )
            .await;
        let critical = manager.get_alerts_by_level(&AlertLevel::Critical).await;
        assert_eq!(critical.len(), 1);
    }

    #[test]
    fn alert_management_test_suite_new_default() {
        let suite = AlertManagementTestSuite::new_default();
        assert_eq!(suite.tests.len(), 4);
    }

    #[tokio::test]
    async fn alert_management_test_suite_run_all() {
        let suite = AlertManagementTestSuite::new_default();
        let results = suite.run_all().await.unwrap();
        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|r| r.passed));
    }
}
