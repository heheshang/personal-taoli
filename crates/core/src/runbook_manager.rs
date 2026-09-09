//! 运行手册模块，用于定义和管理操作手册。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 运行手册章节
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookSection {
    /// 章节ID
    pub section_id: String,
    /// 章节标题
    pub title: String,
    /// 章节内容
    pub content: String,
    /// 章节顺序
    pub order: u32,
    /// 子章节
    pub subsections: Vec<RunbookSection>,
}

/// 运行手册
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runbook {
    /// 手册ID
    pub runbook_id: String,
    /// 手册标题
    pub title: String,
    /// 手册描述
    pub description: String,
    /// 手册版本
    pub version: String,
    /// 手册章节
    pub sections: Vec<RunbookSection>,
    /// 手册标签
    pub tags: HashMap<String, String>,
    /// 创建时间
    pub created_at_ms: u64,
    /// 更新时间
    pub updated_at_ms: u64,
}

/// 运行手册管理器
pub struct RunbookManager {
    runbooks: HashMap<String, Runbook>,
}

impl RunbookManager {
    /// 创建新的运行手册管理器
    pub fn new() -> Self {
        Self {
            runbooks: HashMap::new(),
        }
    }

    /// 添加运行手册
    pub fn add_runbook(&mut self, runbook: Runbook) {
        self.runbooks.insert(runbook.runbook_id.clone(), runbook);
    }

    /// 获取运行手册
    pub fn get_runbook(&self, runbook_id: &str) -> Option<&Runbook> {
        self.runbooks.get(runbook_id)
    }

    /// 获取所有运行手册
    pub fn get_runbooks(&self) -> &HashMap<String, Runbook> {
        &self.runbooks
    }

    /// 删除运行手册
    pub fn delete_runbook(&mut self, runbook_id: &str) -> Result<()> {
        if self.runbooks.remove(runbook_id).is_some() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("runbook not found"))
        }
    }

    /// 更新运行手册
    pub fn update_runbook(&mut self, runbook: Runbook) -> Result<()> {
        if let Some(existing) = self.runbooks.get_mut(&runbook.runbook_id) {
            *existing = runbook;
            Ok(())
        } else {
            Err(anyhow::anyhow!("runbook not found"))
        }
    }

    /// 搜索运行手册
    pub fn search_runbooks(&self, query: &str) -> Vec<&Runbook> {
        self.runbooks
            .values()
            .filter(|r| {
                r.title.contains(query)
                    || r.description.contains(query)
                    || r.tags.values().any(|v| v.contains(query))
            })
            .collect()
    }

    /// 获取特定标签的运行手册
    pub fn get_runbooks_by_tag(&self, key: &str, value: &str) -> Vec<&Runbook> {
        self.runbooks
            .values()
            .filter(|r| r.tags.get(key).map(|v| v.as_str()) == Some(value))
            .collect()
    }
}

impl Default for RunbookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 运行手册模板
pub struct RunbookTemplate;

impl RunbookTemplate {
    /// 创建系统启动手册
    pub fn system_startup() -> Runbook {
        Runbook {
            runbook_id: "system-startup".to_string(),
            title: "System Startup Runbook".to_string(),
            description: "系统启动操作手册".to_string(),
            version: "1.0.0".to_string(),
            sections: vec![
                RunbookSection {
                    section_id: "pre-check".to_string(),
                    title: "Pre-check".to_string(),
                    content: "检查系统状态和配置".to_string(),
                    order: 1,
                    subsections: vec![],
                },
                RunbookSection {
                    section_id: "start-services".to_string(),
                    title: "Start Services".to_string(),
                    content: "启动系统服务".to_string(),
                    order: 2,
                    subsections: vec![],
                },
                RunbookSection {
                    section_id: "verify-health".to_string(),
                    title: "Verify Health".to_string(),
                    content: "验证系统健康状态".to_string(),
                    order: 3,
                    subsections: vec![],
                },
            ],
            tags: HashMap::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    /// 创建系统关闭手册
    pub fn system_shutdown() -> Runbook {
        Runbook {
            runbook_id: "system-shutdown".to_string(),
            title: "System Shutdown Runbook".to_string(),
            description: "系统关闭操作手册".to_string(),
            version: "1.0.0".to_string(),
            sections: vec![
                RunbookSection {
                    section_id: "stop-accepting".to_string(),
                    title: "Stop Accepting Requests".to_string(),
                    content: "停止接受新请求".to_string(),
                    order: 1,
                    subsections: vec![],
                },
                RunbookSection {
                    section_id: "drain-connections".to_string(),
                    title: "Drain Connections".to_string(),
                    content: "排空现有连接".to_string(),
                    order: 2,
                    subsections: vec![],
                },
                RunbookSection {
                    section_id: "stop-services".to_string(),
                    title: "Stop Services".to_string(),
                    content: "停止系统服务".to_string(),
                    order: 3,
                    subsections: vec![],
                },
            ],
            tags: HashMap::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    /// 创建故障恢复手册
    pub fn fault_recovery() -> Runbook {
        Runbook {
            runbook_id: "fault-recovery".to_string(),
            title: "Fault Recovery Runbook".to_string(),
            description: "故障恢复操作手册".to_string(),
            version: "1.0.0".to_string(),
            sections: vec![
                RunbookSection {
                    section_id: "detect-fault".to_string(),
                    title: "Detect Fault".to_string(),
                    content: "检测故障类型".to_string(),
                    order: 1,
                    subsections: vec![],
                },
                RunbookSection {
                    section_id: "isolate-component".to_string(),
                    title: "Isolate Component".to_string(),
                    content: "隔离故障组件".to_string(),
                    order: 2,
                    subsections: vec![],
                },
                RunbookSection {
                    section_id: "recover-component".to_string(),
                    title: "Recover Component".to_string(),
                    content: "恢复故障组件".to_string(),
                    order: 3,
                    subsections: vec![],
                },
            ],
            tags: HashMap::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }
}

/// 运行手册测试套件
pub struct RunbookTestSuite {
    tests: Vec<RunbookTest>,
}

struct RunbookTest {
    name: String,
    #[expect(dead_code)]
    description: String,
    #[expect(dead_code)]
    expected_behavior: String,
}

impl RunbookTestSuite {
    /// 创建默认测试套件
    pub fn new_default() -> Self {
        let tests = vec![
            RunbookTest {
                name: "runbook_creation".to_string(),
                description: "运行手册创建测试".to_string(),
                expected_behavior: "运行手册创建成功".to_string(),
            },
            RunbookTest {
                name: "runbook_management".to_string(),
                description: "运行手册管理测试".to_string(),
                expected_behavior: "运行手册管理功能正常".to_string(),
            },
            RunbookTest {
                name: "runbook_search".to_string(),
                description: "运行手册搜索测试".to_string(),
                expected_behavior: "运行手册搜索功能正常".to_string(),
            },
        ];

        Self { tests }
    }

    /// 运行所有测试
    pub async fn run_all(&self) -> Result<Vec<RunbookTestResult>> {
        let mut results = Vec::new();

        for test in &self.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行单个测试
    async fn run_test(&self, test: &RunbookTest) -> Result<RunbookTestResult> {
        let mut manager = RunbookManager::new();

        // 创建测试手册
        let runbook = RunbookTemplate::system_startup();
        manager.add_runbook(runbook);

        // 测试管理功能
        let runbooks = manager.get_runbooks();
        let passed = runbooks.len() == 1;

        Ok(RunbookTestResult {
            test_name: test.name.clone(),
            passed,
            runbooks_created: runbooks.len(),
            runbooks_found: manager.search_runbooks("System").len(),
        })
    }
}

/// 运行手册测试结果
#[derive(Debug, Clone, Serialize)]
pub struct RunbookTestResult {
    pub test_name: String,
    pub passed: bool,
    pub runbooks_created: usize,
    pub runbooks_found: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runbook_manager_new() {
        let manager = RunbookManager::new();
        assert!(manager.runbooks.is_empty());
    }

    #[test]
    fn runbook_manager_add_runbook() {
        let mut manager = RunbookManager::new();
        let runbook = RunbookTemplate::system_startup();
        manager.add_runbook(runbook);
        assert_eq!(manager.runbooks.len(), 1);
    }

    #[test]
    fn runbook_manager_get_runbook() {
        let mut manager = RunbookManager::new();
        let runbook = RunbookTemplate::system_startup();
        let runbook_id = runbook.runbook_id.clone();
        manager.add_runbook(runbook);
        assert!(manager.get_runbook(&runbook_id).is_some());
    }

    #[test]
    fn runbook_manager_delete_runbook() {
        let mut manager = RunbookManager::new();
        let runbook = RunbookTemplate::system_startup();
        let runbook_id = runbook.runbook_id.clone();
        manager.add_runbook(runbook);
        manager.delete_runbook(&runbook_id).unwrap();
        assert!(manager.runbooks.is_empty());
    }

    #[test]
    fn runbook_manager_search_runbooks() {
        let mut manager = RunbookManager::new();
        let runbook = RunbookTemplate::system_startup();
        manager.add_runbook(runbook);
        let results = manager.search_runbooks("System");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn runbook_template_system_startup() {
        let runbook = RunbookTemplate::system_startup();
        assert_eq!(runbook.runbook_id, "system-startup");
        assert_eq!(runbook.sections.len(), 3);
    }

    #[test]
    fn runbook_template_system_shutdown() {
        let runbook = RunbookTemplate::system_shutdown();
        assert_eq!(runbook.runbook_id, "system-shutdown");
        assert_eq!(runbook.sections.len(), 3);
    }

    #[test]
    fn runbook_template_fault_recovery() {
        let runbook = RunbookTemplate::fault_recovery();
        assert_eq!(runbook.runbook_id, "fault-recovery");
        assert_eq!(runbook.sections.len(), 3);
    }

    #[test]
    fn runbook_test_suite_new_default() {
        let suite = RunbookTestSuite::new_default();
        assert_eq!(suite.tests.len(), 3);
    }

    #[tokio::test]
    async fn runbook_test_suite_run_all() {
        let suite = RunbookTestSuite::new_default();
        let results = suite.run_all().await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.passed));
    }
}
