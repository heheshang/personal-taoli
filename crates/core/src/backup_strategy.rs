//! 备份策略模块，用于定义和执行备份策略。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 备份策略类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupStrategyType {
    /// 完整备份
    Full,
    /// 增量备份
    Incremental,
    /// 差异备份
    Differential,
    /// WAL归档
    WalArchive,
}

impl BackupStrategyType {
    /// 获取策略类型名称
    pub fn as_str(&self) -> &'static str {
        match self {
            BackupStrategyType::Full => "FULL",
            BackupStrategyType::Incremental => "INCREMENTAL",
            BackupStrategyType::Differential => "DIFFERENTIAL",
            BackupStrategyType::WalArchive => "WAL_ARCHIVE",
        }
    }
}

/// 备份策略配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStrategyConfig {
    /// 策略名称
    pub name: String,
    /// 策略类型
    pub strategy_type: BackupStrategyType,
    /// 备份间隔（秒）
    pub interval_seconds: u64,
    /// 保留份数
    pub retention_count: u32,
    /// 保留时间（秒）
    pub retention_seconds: u64,
    /// 备份路径
    pub backup_path: PathBuf,
    /// 压缩启用
    pub compression_enabled: bool,
    /// 加密启用
    pub encryption_enabled: bool,
}

impl Default for BackupStrategyConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            strategy_type: BackupStrategyType::Full,
            interval_seconds: 86400, // 24小时
            retention_count: 7,
            retention_seconds: 604800, // 7天
            backup_path: PathBuf::from("/tmp/backups"),
            compression_enabled: true,
            encryption_enabled: false,
        }
    }
}

/// 备份任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupTask {
    /// 任务ID
    pub task_id: String,
    /// 策略配置
    pub config: BackupStrategyConfig,
    /// 任务状态
    pub status: BackupTaskStatus,
    /// 创建时间
    pub created_at_ms: u64,
    /// 开始时间
    pub started_at_ms: Option<u64>,
    /// 完成时间
    pub completed_at_ms: Option<u64>,
    /// 备份大小（字节）
    pub size_bytes: Option<u64>,
    /// 错误信息
    pub error_message: Option<String>,
}

/// 备份任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupTaskStatus {
    /// 待执行
    Pending,
    /// 执行中
    Running,
    /// 完成
    Completed,
    /// 失败
    Failed,
    /// 取消
    Cancelled,
}

impl BackupTaskStatus {
    /// 获取状态名称
    pub fn as_str(&self) -> &'static str {
        match self {
            BackupTaskStatus::Pending => "PENDING",
            BackupTaskStatus::Running => "RUNNING",
            BackupTaskStatus::Completed => "COMPLETED",
            BackupTaskStatus::Failed => "FAILED",
            BackupTaskStatus::Cancelled => "CANCELLED",
        }
    }
}

/// 备份策略管理器
pub struct BackupStrategyManager {
    configs: HashMap<String, BackupStrategyConfig>,
    tasks: Vec<BackupTask>,
}

impl BackupStrategyManager {
    /// 创建新的备份策略管理器
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            tasks: Vec::new(),
        }
    }

    /// 添加备份策略配置
    pub fn add_config(&mut self, config: BackupStrategyConfig) {
        self.configs.insert(config.name.clone(), config);
    }

    /// 获取备份策略配置
    pub fn get_config(&self, name: &str) -> Option<&BackupStrategyConfig> {
        self.configs.get(name)
    }

    /// 获取所有备份策略配置
    pub fn get_configs(&self) -> &HashMap<String, BackupStrategyConfig> {
        &self.configs
    }

    /// 创建备份任务
    pub fn create_task(&mut self, config_name: &str, now_ms: u64) -> Result<BackupTask> {
        let config = self
            .configs
            .get(config_name)
            .ok_or_else(|| anyhow::anyhow!("backup strategy config not found"))?
            .clone();

        let task = BackupTask {
            task_id: format!("backup-{}-{}", now_ms, self.tasks.len()),
            config,
            status: BackupTaskStatus::Pending,
            created_at_ms: now_ms,
            started_at_ms: None,
            completed_at_ms: None,
            size_bytes: None,
            error_message: None,
        };

        self.tasks.push(task.clone());
        Ok(task)
    }

    /// 开始执行备份任务
    pub fn start_task(&mut self, task_id: &str, now_ms: u64) -> Result<()> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == task_id) {
            task.status = BackupTaskStatus::Running;
            task.started_at_ms = Some(now_ms);
            Ok(())
        } else {
            Err(anyhow::anyhow!("backup task not found"))
        }
    }

    /// 完成备份任务
    pub fn complete_task(&mut self, task_id: &str, size_bytes: u64, now_ms: u64) -> Result<()> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == task_id) {
            task.status = BackupTaskStatus::Completed;
            task.completed_at_ms = Some(now_ms);
            task.size_bytes = Some(size_bytes);
            Ok(())
        } else {
            Err(anyhow::anyhow!("backup task not found"))
        }
    }

    /// 备份任务失败
    pub fn fail_task(&mut self, task_id: &str, error_message: &str) -> Result<()> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == task_id) {
            task.status = BackupTaskStatus::Failed;
            task.error_message = Some(error_message.to_string());
            Ok(())
        } else {
            Err(anyhow::anyhow!("backup task not found"))
        }
    }

    /// 取消备份任务
    pub fn cancel_task(&mut self, task_id: &str) -> Result<()> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == task_id) {
            task.status = BackupTaskStatus::Cancelled;
            Ok(())
        } else {
            Err(anyhow::anyhow!("backup task not found"))
        }
    }

    /// 获取所有备份任务
    pub fn get_tasks(&self) -> &[BackupTask] {
        &self.tasks
    }

    /// 获取特定状态的备份任务
    pub fn get_tasks_by_status(&self, status: &BackupTaskStatus) -> Vec<&BackupTask> {
        self.tasks.iter().filter(|t| t.status == *status).collect()
    }

    /// 清理过期备份
    pub fn cleanup_expired_tasks(&mut self, now_ms: u64) -> Vec<BackupTask> {
        let mut expired_tasks = Vec::new();
        self.tasks.retain(|task| {
            if let Some(config) = self.configs.get(&task.config.name) {
                let age_ms = now_ms.saturating_sub(task.created_at_ms);
                let max_age_ms = config.retention_seconds * 1000;
                if age_ms > max_age_ms {
                    expired_tasks.push(task.clone());
                    return false;
                }
            }
            true
        });
        expired_tasks
    }
}

impl Default for BackupStrategyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 备份策略测试套件
pub struct BackupStrategyTestSuite {
    tests: Vec<BackupStrategyTest>,
}

struct BackupStrategyTest {
    name: String,
    #[expect(dead_code)]
    description: String,
    strategy_type: BackupStrategyType,
    #[expect(dead_code)]
    expected_behavior: String,
}

impl BackupStrategyTestSuite {
    /// 创建默认测试套件
    pub fn new_default() -> Self {
        let tests = vec![
            BackupStrategyTest {
                name: "full_backup_strategy".to_string(),
                description: "完整备份策略测试".to_string(),
                strategy_type: BackupStrategyType::Full,
                expected_behavior: "完整备份策略配置正确".to_string(),
            },
            BackupStrategyTest {
                name: "incremental_backup_strategy".to_string(),
                description: "增量备份策略测试".to_string(),
                strategy_type: BackupStrategyType::Incremental,
                expected_behavior: "增量备份策略配置正确".to_string(),
            },
            BackupStrategyTest {
                name: "differential_backup_strategy".to_string(),
                description: "差异备份策略测试".to_string(),
                strategy_type: BackupStrategyType::Differential,
                expected_behavior: "差异备份策略配置正确".to_string(),
            },
        ];

        Self { tests }
    }

    /// 运行所有测试
    pub async fn run_all(&self) -> Result<Vec<BackupStrategyTestResult>> {
        let mut results = Vec::new();

        for test in &self.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行单个测试
    async fn run_test(&self, test: &BackupStrategyTest) -> Result<BackupStrategyTestResult> {
        let mut manager = BackupStrategyManager::new();

        // 创建配置
        let config = BackupStrategyConfig {
            name: test.name.clone(),
            strategy_type: test.strategy_type.clone(),
            ..Default::default()
        };
        manager.add_config(config);

        // 创建任务
        let task = manager.create_task(&test.name, 1000)?;

        // 开始任务
        manager.start_task(&task.task_id, 2000)?;

        // 完成任务
        manager.complete_task(&task.task_id, 1024, 3000)?;

        let tasks = manager.get_tasks();
        let passed = tasks.len() == 1 && tasks[0].status == BackupTaskStatus::Completed;

        Ok(BackupStrategyTestResult {
            test_name: test.name.clone(),
            passed,
            tasks_created: tasks.len(),
            tasks_completed: tasks
                .iter()
                .filter(|t| t.status == BackupTaskStatus::Completed)
                .count(),
        })
    }
}

/// 备份策略测试结果
#[derive(Debug, Clone, Serialize)]
pub struct BackupStrategyTestResult {
    pub test_name: String,
    pub passed: bool,
    pub tasks_created: usize,
    pub tasks_completed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_strategy_type_as_str() {
        assert_eq!(BackupStrategyType::Full.as_str(), "FULL");
        assert_eq!(BackupStrategyType::Incremental.as_str(), "INCREMENTAL");
        assert_eq!(BackupStrategyType::Differential.as_str(), "DIFFERENTIAL");
        assert_eq!(BackupStrategyType::WalArchive.as_str(), "WAL_ARCHIVE");
    }

    #[test]
    fn backup_task_status_as_str() {
        assert_eq!(BackupTaskStatus::Pending.as_str(), "PENDING");
        assert_eq!(BackupTaskStatus::Running.as_str(), "RUNNING");
        assert_eq!(BackupTaskStatus::Completed.as_str(), "COMPLETED");
        assert_eq!(BackupTaskStatus::Failed.as_str(), "FAILED");
        assert_eq!(BackupTaskStatus::Cancelled.as_str(), "CANCELLED");
    }

    #[test]
    fn backup_strategy_config_default() {
        let config = BackupStrategyConfig::default();
        assert_eq!(config.name, "default");
        assert_eq!(config.strategy_type, BackupStrategyType::Full);
        assert_eq!(config.interval_seconds, 86400);
        assert_eq!(config.retention_count, 7);
    }

    #[test]
    fn backup_strategy_manager_new() {
        let manager = BackupStrategyManager::new();
        assert!(manager.configs.is_empty());
        assert!(manager.tasks.is_empty());
    }

    #[test]
    fn backup_strategy_manager_add_config() {
        let mut manager = BackupStrategyManager::new();
        let config = BackupStrategyConfig::default();
        manager.add_config(config);
        assert_eq!(manager.configs.len(), 1);
    }

    #[test]
    fn backup_strategy_manager_create_task() {
        let mut manager = BackupStrategyManager::new();
        let config = BackupStrategyConfig::default();
        manager.add_config(config);
        let task = manager.create_task("default", 1000).unwrap();
        assert_eq!(task.status, BackupTaskStatus::Pending);
    }

    #[test]
    fn backup_strategy_manager_start_task() {
        let mut manager = BackupStrategyManager::new();
        let config = BackupStrategyConfig::default();
        manager.add_config(config);
        let task = manager.create_task("default", 1000).unwrap();
        manager.start_task(&task.task_id, 2000).unwrap();
        let task = manager.get_tasks().first().unwrap();
        assert_eq!(task.status, BackupTaskStatus::Running);
    }

    #[test]
    fn backup_strategy_manager_complete_task() {
        let mut manager = BackupStrategyManager::new();
        let config = BackupStrategyConfig::default();
        manager.add_config(config);
        let task = manager.create_task("default", 1000).unwrap();
        manager.start_task(&task.task_id, 2000).unwrap();
        manager.complete_task(&task.task_id, 1024, 3000).unwrap();
        let task = manager.get_tasks().first().unwrap();
        assert_eq!(task.status, BackupTaskStatus::Completed);
        assert_eq!(task.size_bytes, Some(1024));
    }

    #[test]
    fn backup_strategy_manager_fail_task() {
        let mut manager = BackupStrategyManager::new();
        let config = BackupStrategyConfig::default();
        manager.add_config(config);
        let task = manager.create_task("default", 1000).unwrap();
        manager.start_task(&task.task_id, 2000).unwrap();
        manager.fail_task(&task.task_id, "test error").unwrap();
        let task = manager.get_tasks().first().unwrap();
        assert_eq!(task.status, BackupTaskStatus::Failed);
        assert_eq!(task.error_message, Some("test error".to_string()));
    }

    #[test]
    fn backup_strategy_manager_cancel_task() {
        let mut manager = BackupStrategyManager::new();
        let config = BackupStrategyConfig::default();
        manager.add_config(config);
        let task = manager.create_task("default", 1000).unwrap();
        manager.cancel_task(&task.task_id).unwrap();
        let task = manager.get_tasks().first().unwrap();
        assert_eq!(task.status, BackupTaskStatus::Cancelled);
    }

    #[test]
    fn backup_strategy_test_suite_new_default() {
        let suite = BackupStrategyTestSuite::new_default();
        assert_eq!(suite.tests.len(), 3);
    }

    #[tokio::test]
    async fn backup_strategy_test_suite_run_all() {
        let suite = BackupStrategyTestSuite::new_default();
        let results = suite.run_all().await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.passed));
    }
}
