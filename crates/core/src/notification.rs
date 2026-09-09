//! 告警通知模块，用于发送告警通知到多个通道。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 通知通道类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotificationChannel {
    /// 日志
    Log,
    /// 控制台
    Console,
    /// Webhook
    Webhook,
    /// 邮件
    Email,
    /// 短信
    SMS,
}

impl NotificationChannel {
    /// 获取通道名称
    pub fn as_str(&self) -> &'static str {
        match self {
            NotificationChannel::Log => "LOG",
            NotificationChannel::Console => "CONSOLE",
            NotificationChannel::Webhook => "WEBHOOK",
            NotificationChannel::Email => "EMAIL",
            NotificationChannel::SMS => "SMS",
        }
    }
}

/// 通知消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationMessage {
    /// 消息ID
    pub message_id: String,
    /// 告警级别
    pub level: String,
    /// 告警标题
    pub title: String,
    /// 告警内容
    pub content: String,
    /// 告警时间戳
    pub timestamp_ms: u64,
    /// 告警来源
    pub source: String,
    /// 告警标签
    pub labels: HashMap<String, String>,
}

/// 通知结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationResult {
    /// 通道
    pub channel: NotificationChannel,
    /// 是否成功
    pub success: bool,
    /// 错误信息
    pub error_message: Option<String>,
    /// 响应时间（毫秒）
    pub response_time_ms: u64,
}

/// 通知器trait
#[async_trait::async_trait]
pub trait NotificationSender: Send + Sync {
    /// 发送通知
    async fn send_notification(&self, message: &NotificationMessage) -> Result<NotificationResult>;
}

/// 日志通知器
pub struct LogNotificationSender;

#[async_trait::async_trait]
impl NotificationSender for LogNotificationSender {
    async fn send_notification(&self, message: &NotificationMessage) -> Result<NotificationResult> {
        let start = std::time::Instant::now();
        println!(
            "[ALERT][{}][{}] {}",
            message.level, message.title, message.content
        );
        let response_time_ms = start.elapsed().as_millis() as u64;

        Ok(NotificationResult {
            channel: NotificationChannel::Log,
            success: true,
            error_message: None,
            response_time_ms,
        })
    }
}

/// 控制台通知器
pub struct ConsoleNotificationSender;

#[async_trait::async_trait]
impl NotificationSender for ConsoleNotificationSender {
    async fn send_notification(&self, message: &NotificationMessage) -> Result<NotificationResult> {
        let start = std::time::Instant::now();
        println!(
            "[CONSOLE][{}][{}] {}",
            message.level, message.title, message.content
        );
        let response_time_ms = start.elapsed().as_millis() as u64;

        Ok(NotificationResult {
            channel: NotificationChannel::Console,
            success: true,
            error_message: None,
            response_time_ms,
        })
    }
}

/// Webhook通知器
pub struct WebhookNotificationSender {
    #[expect(dead_code)]
    url: String,
}

impl WebhookNotificationSender {
    /// 创建新的Webhook通知器
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[async_trait::async_trait]
impl NotificationSender for WebhookNotificationSender {
    async fn send_notification(
        &self,
        _message: &NotificationMessage,
    ) -> Result<NotificationResult> {
        let start = std::time::Instant::now();
        // 模拟Webhook发送
        let response_time_ms = start.elapsed().as_millis() as u64;

        Ok(NotificationResult {
            channel: NotificationChannel::Webhook,
            success: true,
            error_message: None,
            response_time_ms,
        })
    }
}

/// 通知管理器
pub struct NotificationManager {
    senders: Vec<Arc<dyn NotificationSender>>,
    results: Arc<RwLock<Vec<NotificationResult>>>,
}

impl NotificationManager {
    /// 创建新的通知管理器
    pub fn new() -> Self {
        Self {
            senders: Vec::new(),
            results: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 添加通知器
    pub fn add_sender(&mut self, sender: Arc<dyn NotificationSender>) {
        self.senders.push(sender);
    }

    /// 发送通知到所有通道
    pub async fn send_notification(
        &self,
        message: &NotificationMessage,
    ) -> Vec<NotificationResult> {
        let mut results = Vec::new();

        for sender in &self.senders {
            match sender.send_notification(message).await {
                Ok(result) => {
                    results.push(result.clone());
                    self.results.write().await.push(result);
                }
                Err(e) => {
                    let result = NotificationResult {
                        channel: NotificationChannel::Log,
                        success: false,
                        error_message: Some(e.to_string()),
                        response_time_ms: 0,
                    };
                    results.push(result.clone());
                    self.results.write().await.push(result);
                }
            }
        }

        results
    }

    /// 获取所有通知结果
    pub async fn get_results(&self) -> Vec<NotificationResult> {
        self.results.read().await.clone()
    }

    /// 清除所有通知结果
    pub async fn clear_results(&self) {
        self.results.write().await.clear();
    }
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 告警通知测试套件
pub struct NotificationTestSuite {
    tests: Vec<NotificationTest>,
}

struct NotificationTest {
    name: String,
    #[expect(dead_code)]
    description: String,
    channel: NotificationChannel,
    #[expect(dead_code)]
    expected_behavior: String,
}

impl NotificationTestSuite {
    /// 创建默认测试套件
    pub fn new_default() -> Self {
        let tests = vec![
            NotificationTest {
                name: "log_notification".to_string(),
                description: "日志通知测试".to_string(),
                channel: NotificationChannel::Log,
                expected_behavior: "日志通知发送成功".to_string(),
            },
            NotificationTest {
                name: "console_notification".to_string(),
                description: "控制台通知测试".to_string(),
                channel: NotificationChannel::Console,
                expected_behavior: "控制台通知发送成功".to_string(),
            },
            NotificationTest {
                name: "webhook_notification".to_string(),
                description: "Webhook通知测试".to_string(),
                channel: NotificationChannel::Webhook,
                expected_behavior: "Webhook通知发送成功".to_string(),
            },
        ];

        Self { tests }
    }

    /// 运行所有测试
    pub async fn run_all(&self) -> Result<Vec<NotificationTestResult>> {
        let mut results = Vec::new();

        for test in &self.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行单个测试
    async fn run_test(&self, test: &NotificationTest) -> Result<NotificationTestResult> {
        let mut manager = NotificationManager::new();

        // 添加对应的发送器
        match test.channel {
            NotificationChannel::Log => {
                manager.add_sender(Arc::new(LogNotificationSender));
            }
            NotificationChannel::Console => {
                manager.add_sender(Arc::new(ConsoleNotificationSender));
            }
            NotificationChannel::Webhook => {
                manager.add_sender(Arc::new(WebhookNotificationSender::new(
                    "http://example.com".to_string(),
                )));
            }
            _ => {}
        }

        // 创建测试消息
        let message = NotificationMessage {
            message_id: "test-1".to_string(),
            level: "INFO".to_string(),
            title: "Test Alert".to_string(),
            content: "This is a test alert".to_string(),
            timestamp_ms: 1000,
            source: "test".to_string(),
            labels: HashMap::new(),
        };

        let start = std::time::Instant::now();
        let results = manager.send_notification(&message).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let passed = results.iter().all(|r| r.success);

        Ok(NotificationTestResult {
            test_name: test.name.clone(),
            passed,
            duration_ms,
            channels_tested: results.len(),
            channels_succeeded: results.iter().filter(|r| r.success).count(),
        })
    }
}

/// 告警通知测试结果
#[derive(Debug, Clone, Serialize)]
pub struct NotificationTestResult {
    pub test_name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub channels_tested: usize,
    pub channels_succeeded: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_channel_as_str() {
        assert_eq!(NotificationChannel::Log.as_str(), "LOG");
        assert_eq!(NotificationChannel::Console.as_str(), "CONSOLE");
        assert_eq!(NotificationChannel::Webhook.as_str(), "WEBHOOK");
        assert_eq!(NotificationChannel::Email.as_str(), "EMAIL");
        assert_eq!(NotificationChannel::SMS.as_str(), "SMS");
    }

    #[tokio::test]
    async fn log_notification_sender() {
        let sender = LogNotificationSender;
        let message = NotificationMessage {
            message_id: "test-1".to_string(),
            level: "INFO".to_string(),
            title: "Test".to_string(),
            content: "Test content".to_string(),
            timestamp_ms: 1000,
            source: "test".to_string(),
            labels: HashMap::new(),
        };
        let result = sender.send_notification(&message).await.unwrap();
        assert!(result.success);
        assert_eq!(result.channel, NotificationChannel::Log);
    }

    #[tokio::test]
    async fn console_notification_sender() {
        let sender = ConsoleNotificationSender;
        let message = NotificationMessage {
            message_id: "test-1".to_string(),
            level: "INFO".to_string(),
            title: "Test".to_string(),
            content: "Test content".to_string(),
            timestamp_ms: 1000,
            source: "test".to_string(),
            labels: HashMap::new(),
        };
        let result = sender.send_notification(&message).await.unwrap();
        assert!(result.success);
        assert_eq!(result.channel, NotificationChannel::Console);
    }

    #[tokio::test]
    async fn webhook_notification_sender() {
        let sender = WebhookNotificationSender::new("http://example.com".to_string());
        let message = NotificationMessage {
            message_id: "test-1".to_string(),
            level: "INFO".to_string(),
            title: "Test".to_string(),
            content: "Test content".to_string(),
            timestamp_ms: 1000,
            source: "test".to_string(),
            labels: HashMap::new(),
        };
        let result = sender.send_notification(&message).await.unwrap();
        assert!(result.success);
        assert_eq!(result.channel, NotificationChannel::Webhook);
    }

    #[tokio::test]
    async fn notification_manager_send_notification() {
        let mut manager = NotificationManager::new();
        manager.add_sender(Arc::new(LogNotificationSender));
        manager.add_sender(Arc::new(ConsoleNotificationSender));

        let message = NotificationMessage {
            message_id: "test-1".to_string(),
            level: "INFO".to_string(),
            title: "Test".to_string(),
            content: "Test content".to_string(),
            timestamp_ms: 1000,
            source: "test".to_string(),
            labels: HashMap::new(),
        };

        let results = manager.send_notification(&message).await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.success));
    }

    #[test]
    fn notification_test_suite_new_default() {
        let suite = NotificationTestSuite::new_default();
        assert_eq!(suite.tests.len(), 3);
    }

    #[tokio::test]
    async fn notification_test_suite_run_all() {
        let suite = NotificationTestSuite::new_default();
        let results = suite.run_all().await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.passed));
    }
}
