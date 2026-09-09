//! 告警模块，用于监控系统状态并生成告警。

use serde::{Deserialize, Serialize};

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

    /// 获取告警级别描述
    pub fn description(&self) -> &'static str {
        match self {
            AlertLevel::Critical => {
                "P0: 无法确认的大额敞口、保证金危险、交易核心失联且有仓位、持久化失效"
            }
            AlertLevel::High => "P1: 未知订单超时、私有流中断、余额冲突、持续单场所故障",
            AlertLevel::Medium => "P2: 库存偏移、行情延迟升高、归档积压、收益估计误差变大",
            AlertLevel::Info => "信息: 正常启停、配置批准、日结、再平衡到账",
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

/// 告警类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertType {
    /// 私有流中断
    PrivateStream中断 {
        /// 交易所名称
        venue: String,
        /// 中断持续时间
        duration_ms: u64,
    },
    /// 裸露敞口
    Exposure {
        /// 敞口金额
        amount: String,
        /// 敞口持续时间
        duration_ms: u64,
    },
    /// 未知订单
    UnknownOrder {
        /// 交易所名称
        venue: String,
        /// 订单ID
        order_id: String,
        /// 超时时间
        timeout_ms: u64,
    },
    /// 余额冲突
    BalanceConflict {
        /// 交易所名称
        venue: String,
        /// 资产名称
        asset: String,
        /// 本地余额
        local_amount: String,
        /// 外部余额
        external_amount: String,
    },
    /// 数据库失效
    DatabaseFailure {
        /// 失效持续时间
        duration_ms: u64,
    },
    /// 进程崩溃
    ProcessCrash {
        /// 崩溃时间戳
        crashed_at_ms: u64,
    },
    /// 行情延迟
    MarketDataDelay {
        /// 交易所名称
        venue: String,
        /// 延迟时间
        delay_ms: u64,
    },
    /// 归档积压
    ArchiveBacklog {
        /// 积压事件数
        backlog_count: u64,
    },
}

/// 告警事件
#[derive(Debug, Clone, Serialize)]
pub struct AlertEvent {
    /// 告警ID
    pub alert_id: String,
    /// 告警级别
    pub level: AlertLevel,
    /// 告警类型
    pub alert_type: AlertType,
    /// 告警状态
    pub status: AlertStatus,
    /// 告警消息
    pub message: String,
    /// 创建时间
    pub created_at_ms: u64,
    /// 更新时间
    pub updated_at_ms: Option<u64>,
}

/// 告警管理器
#[derive(Default)]
pub struct AlertManager {
    alerts: Vec<AlertEvent>,
}

impl AlertManager {
    /// 创建新的告警管理器
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建新告警
    pub fn create_alert(
        &mut self,
        level: AlertLevel,
        alert_type: AlertType,
        message: String,
        now_ms: u64,
    ) -> AlertEvent {
        let alert_id = format!("alert-{}-{}", now_ms, self.alerts.len());
        let event = AlertEvent {
            alert_id: alert_id.clone(),
            level,
            alert_type,
            status: AlertStatus::Pending,
            message,
            created_at_ms: now_ms,
            updated_at_ms: None,
        };
        self.alerts.push(event.clone());
        event
    }

    /// 确认告警
    pub fn acknowledge_alert(&mut self, alert_id: &str, now_ms: u64) -> Option<&AlertEvent> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.alert_id == alert_id) {
            alert.status = AlertStatus::Acknowledged;
            alert.updated_at_ms = Some(now_ms);
        }
        self.alerts.iter().find(|a| a.alert_id == alert_id)
    }

    /// 解决告警
    pub fn resolve_alert(&mut self, alert_id: &str, now_ms: u64) -> Option<&AlertEvent> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.alert_id == alert_id) {
            alert.status = AlertStatus::Resolved;
            alert.updated_at_ms = Some(now_ms);
        }
        self.alerts.iter().find(|a| a.alert_id == alert_id)
    }

    /// 忽略告警
    pub fn ignore_alert(&mut self, alert_id: &str, now_ms: u64) -> Option<&AlertEvent> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.alert_id == alert_id) {
            alert.status = AlertStatus::Ignored;
            alert.updated_at_ms = Some(now_ms);
        }
        self.alerts.iter().find(|a| a.alert_id == alert_id)
    }

    /// 获取所有告警
    pub fn get_alerts(&self) -> &[AlertEvent] {
        &self.alerts
    }

    /// 获取待处理告警
    pub fn get_pending_alerts(&self) -> Vec<&AlertEvent> {
        self.alerts
            .iter()
            .filter(|a| a.status == AlertStatus::Pending)
            .collect()
    }

    /// 获取特定级别的告警
    pub fn get_alerts_by_level(&self, level: &AlertLevel) -> Vec<&AlertEvent> {
        self.alerts.iter().filter(|a| a.level == *level).collect()
    }
}

/// 告警通知器 trait
pub trait AlertNotifier {
    /// 发送告警通知
    fn send_notification(&self, alert: &AlertEvent) -> Result<(), String>;
}

/// 控制台通知器（用于测试）
pub struct ConsoleNotifier;

impl AlertNotifier for ConsoleNotifier {
    fn send_notification(&self, alert: &AlertEvent) -> Result<(), String> {
        println!(
            "[{}] {}: {}",
            alert.level.as_str(),
            alert.alert_id,
            alert.message
        );
        Ok(())
    }
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

    #[test]
    fn alert_manager_create_alert() {
        let mut manager = AlertManager::new();
        let alert = manager.create_alert(
            AlertLevel::Critical,
            AlertType::DatabaseFailure { duration_ms: 5000 },
            "数据库连接失败".to_string(),
            1000,
        );
        assert_eq!(alert.status, AlertStatus::Pending);
        assert_eq!(manager.get_alerts().len(), 1);
    }

    #[test]
    fn alert_manager_acknowledge_alert() {
        let mut manager = AlertManager::new();
        let alert = manager.create_alert(
            AlertLevel::High,
            AlertType::PrivateStream中断 {
                venue: "binance".to_string(),
                duration_ms: 3000,
            },
            "私有流中断".to_string(),
            1000,
        );
        manager.acknowledge_alert(&alert.alert_id, 2000);
        let updated = manager.get_alerts().first().unwrap();
        assert_eq!(updated.status, AlertStatus::Acknowledged);
    }

    #[test]
    fn alert_manager_resolve_alert() {
        let mut manager = AlertManager::new();
        let alert = manager.create_alert(
            AlertLevel::Medium,
            AlertType::MarketDataDelay {
                venue: "bybit".to_string(),
                delay_ms: 1000,
            },
            "行情延迟".to_string(),
            1000,
        );
        manager.resolve_alert(&alert.alert_id, 2000);
        let updated = manager.get_alerts().first().unwrap();
        assert_eq!(updated.status, AlertStatus::Resolved);
    }

    #[test]
    fn alert_manager_get_pending_alerts() {
        let mut manager = AlertManager::new();
        let alert1 = manager.create_alert(
            AlertLevel::Critical,
            AlertType::DatabaseFailure { duration_ms: 5000 },
            "数据库连接失败".to_string(),
            1000,
        );
        let alert2 = manager.create_alert(
            AlertLevel::High,
            AlertType::PrivateStream中断 {
                venue: "binance".to_string(),
                duration_ms: 3000,
            },
            "私有流中断".to_string(),
            1000,
        );
        manager.acknowledge_alert(&alert1.alert_id, 2000);
        let pending = manager.get_pending_alerts();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].alert_id, alert2.alert_id);
    }

    #[test]
    fn alert_manager_get_alerts_by_level() {
        let mut manager = AlertManager::new();
        manager.create_alert(
            AlertLevel::Critical,
            AlertType::DatabaseFailure { duration_ms: 5000 },
            "数据库连接失败".to_string(),
            1000,
        );
        manager.create_alert(
            AlertLevel::High,
            AlertType::PrivateStream中断 {
                venue: "binance".to_string(),
                duration_ms: 3000,
            },
            "私有流中断".to_string(),
            1000,
        );
        let critical_alerts = manager.get_alerts_by_level(&AlertLevel::Critical);
        assert_eq!(critical_alerts.len(), 1);
    }
}
