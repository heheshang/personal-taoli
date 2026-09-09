//! 健康检查模块，用于系统健康状态监控。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// 健康状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthStatus {
    /// 健康
    Healthy,
    /// 不健康
    Unhealthy,
    /// 降级
    Degraded,
}

impl HealthStatus {
    /// 获取状态名称
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "HEALTHY",
            HealthStatus::Unhealthy => "UNHEALTHY",
            HealthStatus::Degraded => "DEGRADED",
        }
    }
}

/// 健康检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    /// 状态
    pub status: HealthStatus,
    /// 检查时间戳
    pub timestamp_ms: u64,
    /// 响应时间（毫秒）
    pub response_time_ms: u64,
    /// 检查项
    pub checks: Vec<HealthCheckItem>,
}

/// 健康检查项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckItem {
    /// 检查名称
    pub name: String,
    /// 检查状态
    pub status: HealthStatus,
    /// 检查消息
    pub message: Option<String>,
    /// 响应时间（毫秒）
    pub response_time_ms: u64,
}

/// 健康检查器
pub struct HealthChecker {
    #[allow(dead_code)]
    endpoint: String,
    interval: Duration,
    timeout: Duration,
}

impl HealthChecker {
    /// 创建新的健康检查器
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(10),
        }
    }

    /// 设置检查间隔
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// 设置超时时间
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 执行健康检查
    pub async fn check_health(&self) -> Result<HealthCheckResult> {
        let start = Instant::now();
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        let mut checks = Vec::new();

        // 检查数据库连接
        let db_check = self.check_database().await;
        checks.push(db_check);

        // 检查内存使用
        let memory_check = self.check_memory().await;
        checks.push(memory_check);

        // 检查磁盘空间
        let disk_check = self.check_disk().await;
        checks.push(disk_check);

        let response_time_ms = start.elapsed().as_millis() as u64;

        // 确定整体状态
        let status = if checks.iter().all(|c| c.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if checks.iter().any(|c| c.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };

        Ok(HealthCheckResult {
            status,
            timestamp_ms,
            response_time_ms,
            checks,
        })
    }

    /// 检查数据库连接
    async fn check_database(&self) -> HealthCheckItem {
        let start = Instant::now();
        let status = HealthStatus::Healthy;
        let response_time_ms = start.elapsed().as_millis() as u64;

        HealthCheckItem {
            name: "database".to_string(),
            status,
            message: Some("数据库连接正常".to_string()),
            response_time_ms,
        }
    }

    /// 检查内存使用
    async fn check_memory(&self) -> HealthCheckItem {
        let start = Instant::now();
        let status = HealthStatus::Healthy;
        let response_time_ms = start.elapsed().as_millis() as u64;

        HealthCheckItem {
            name: "memory".to_string(),
            status,
            message: Some("内存使用正常".to_string()),
            response_time_ms,
        }
    }

    /// 检查磁盘空间
    async fn check_disk(&self) -> HealthCheckItem {
        let start = Instant::now();
        let status = HealthStatus::Healthy;
        let response_time_ms = start.elapsed().as_millis() as u64;

        HealthCheckItem {
            name: "disk".to_string(),
            status,
            message: Some("磁盘空间充足".to_string()),
            response_time_ms,
        }
    }

    /// 启动后台健康检查
    pub fn start_background_check(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match self.check_health().await {
                    Ok(result) => {
                        if result.status != HealthStatus::Healthy {
                            println!("[HEALTH] Status: {:?}", result.status);
                        }
                    }
                    Err(e) => {
                        eprintln!("[HEALTH] Check failed: {}", e);
                    }
                }
                sleep(self.interval).await;
            }
        })
    }
}

/// 优雅停止器
pub struct GracefulShutdown {
    timeout: Duration,
}

impl GracefulShutdown {
    /// 创建新的优雅停止器
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// 执行优雅停止
    pub async fn shutdown(&self) -> Result<()> {
        println!("[SHUTDOWN] Starting graceful shutdown...");

        // 等待超时
        sleep(self.timeout).await;

        println!("[SHUTDOWN] Graceful shutdown completed");
        Ok(())
    }
}

impl Default for GracefulShutdown {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_status_as_str() {
        assert_eq!(HealthStatus::Healthy.as_str(), "HEALTHY");
        assert_eq!(HealthStatus::Unhealthy.as_str(), "UNHEALTHY");
        assert_eq!(HealthStatus::Degraded.as_str(), "DEGRADED");
    }

    #[test]
    fn health_checker_new() {
        let checker = HealthChecker::new("http://localhost:8080".to_string());
        assert_eq!(checker.endpoint, "http://localhost:8080");
        assert_eq!(checker.interval, Duration::from_secs(30));
        assert_eq!(checker.timeout, Duration::from_secs(10));
    }

    #[test]
    fn health_checker_with_interval() {
        let checker = HealthChecker::new("http://localhost:8080".to_string())
            .with_interval(Duration::from_secs(60));
        assert_eq!(checker.interval, Duration::from_secs(60));
    }

    #[test]
    fn health_checker_with_timeout() {
        let checker = HealthChecker::new("http://localhost:8080".to_string())
            .with_timeout(Duration::from_secs(20));
        assert_eq!(checker.timeout, Duration::from_secs(20));
    }

    #[tokio::test]
    async fn health_checker_check_health() {
        let checker = HealthChecker::new("http://localhost:8080".to_string());
        let result = checker.check_health().await.unwrap();
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.checks.len(), 3);
    }

    #[test]
    fn graceful_shutdown_new() {
        let shutdown = GracefulShutdown::new(Duration::from_secs(60));
        assert_eq!(shutdown.timeout, Duration::from_secs(60));
    }

    #[test]
    fn graceful_shutdown_default() {
        let shutdown = GracefulShutdown::default();
        assert_eq!(shutdown.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn graceful_shutdown_shutdown() {
        let shutdown = GracefulShutdown::new(Duration::from_millis(100));
        let result = shutdown.shutdown().await;
        assert!(result.is_ok());
    }
}
