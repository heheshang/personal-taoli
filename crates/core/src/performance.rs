//! 性能指标收集模块，用于系统性能指标收集和分析。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 性能指标类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PerformanceMetricType {
    /// 响应时间
    ResponseTime,
    /// 吞吐量
    Throughput,
    /// 错误率
    ErrorRate,
    /// 延迟
    Latency,
    /// 资源使用率
    ResourceUsage,
}

impl PerformanceMetricType {
    /// 获取指标类型名称
    pub fn as_str(&self) -> &'static str {
        match self {
            PerformanceMetricType::ResponseTime => "RESPONSE_TIME",
            PerformanceMetricType::Throughput => "THROUGHPUT",
            PerformanceMetricType::ErrorRate => "ERROR_RATE",
            PerformanceMetricType::Latency => "LATENCY",
            PerformanceMetricType::ResourceUsage => "RESOURCE_USAGE",
        }
    }
}

/// 性能指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetric {
    /// 指标名称
    pub name: String,
    /// 指标类型
    pub metric_type: PerformanceMetricType,
    /// 指标值
    pub value: f64,
    /// 指标单位
    pub unit: String,
    /// 指标标签
    pub labels: HashMap<String, String>,
    /// 指标时间戳
    pub timestamp_ms: u64,
}

/// 性能指标收集器
pub struct PerformanceMetricsCollector {
    metrics: Arc<RwLock<Vec<PerformanceMetric>>>,
}

impl PerformanceMetricsCollector {
    /// 创建新的性能指标收集器
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 记录性能指标
    pub async fn record_metric(
        &self,
        name: &str,
        metric_type: PerformanceMetricType,
        value: f64,
        unit: &str,
        labels: HashMap<String, String>,
    ) {
        let metric = PerformanceMetric {
            name: name.to_string(),
            metric_type,
            value,
            unit: unit.to_string(),
            labels,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        self.metrics.write().await.push(metric);
    }

    /// 获取所有性能指标
    pub async fn get_metrics(&self) -> Vec<PerformanceMetric> {
        self.metrics.read().await.clone()
    }

    /// 获取特定名称的性能指标
    pub async fn get_metrics_by_name(&self, name: &str) -> Vec<PerformanceMetric> {
        self.metrics
            .read()
            .await
            .iter()
            .filter(|m| m.name == name)
            .cloned()
            .collect()
    }

    /// 获取特定类型的性能指标
    pub async fn get_metrics_by_type(
        &self,
        metric_type: &PerformanceMetricType,
    ) -> Vec<PerformanceMetric> {
        self.metrics
            .read()
            .await
            .iter()
            .filter(|m| m.metric_type == *metric_type)
            .cloned()
            .collect()
    }

    /// 计算平均值
    pub async fn calculate_average(&self, name: &str) -> f64 {
        let metrics = self.get_metrics_by_name(name).await;
        if metrics.is_empty() {
            return 0.0;
        }
        let sum: f64 = metrics.iter().map(|m| m.value).sum();
        sum / metrics.len() as f64
    }

    /// 计算最大值
    pub async fn calculate_max(&self, name: &str) -> f64 {
        let metrics = self.get_metrics_by_name(name).await;
        metrics.iter().map(|m| m.value).fold(f64::MIN, f64::max)
    }

    /// 计算最小值
    pub async fn calculate_min(&self, name: &str) -> f64 {
        let metrics = self.get_metrics_by_name(name).await;
        metrics.iter().map(|m| m.value).fold(f64::MAX, f64::min)
    }

    /// 清除所有性能指标
    pub async fn clear_metrics(&self) {
        self.metrics.write().await.clear();
    }
}

impl Default for PerformanceMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// 性能指标分析器
pub struct PerformanceAnalyzer {
    collector: Arc<PerformanceMetricsCollector>,
}

impl PerformanceAnalyzer {
    /// 创建新的性能指标分析器
    pub fn new(collector: Arc<PerformanceMetricsCollector>) -> Self {
        Self { collector }
    }

    /// 分析性能指标
    pub async fn analyze_performance(&self, name: &str) -> PerformanceAnalysis {
        let average = self.collector.calculate_average(name).await;
        let max = self.collector.calculate_max(name).await;
        let min = self.collector.calculate_min(name).await;
        let metrics = self.collector.get_metrics_by_name(name).await;

        PerformanceAnalysis {
            name: name.to_string(),
            metric_count: metrics.len(),
            average_value: average,
            max_value: max,
            min_value: min,
            analysis_timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }
}

/// 性能指标分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceAnalysis {
    /// 指标名称
    pub name: String,
    /// 指标数量
    pub metric_count: usize,
    /// 平均值
    pub average_value: f64,
    /// 最大值
    pub max_value: f64,
    /// 最小值
    pub min_value: f64,
    /// 分析时间戳
    pub analysis_timestamp_ms: u64,
}

/// 性能指标测试套件
pub struct PerformanceTestSuite {
    tests: Vec<PerformanceTest>,
}

struct PerformanceTest {
    name: String,
    #[expect(dead_code)]
    description: String,
    #[expect(dead_code)]
    expected_behavior: String,
}

impl PerformanceTestSuite {
    /// 创建默认测试套件
    pub fn new_default() -> Self {
        let tests = vec![
            PerformanceTest {
                name: "metrics_collection".to_string(),
                description: "性能指标收集测试".to_string(),
                expected_behavior: "性能指标收集成功".to_string(),
            },
            PerformanceTest {
                name: "metrics_analysis".to_string(),
                description: "性能指标分析测试".to_string(),
                expected_behavior: "性能指标分析成功".to_string(),
            },
        ];

        Self { tests }
    }

    /// 运行所有测试
    pub async fn run_all(&self) -> Result<Vec<PerformanceTestResult>> {
        let mut results = Vec::new();

        for test in &self.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行单个测试
    async fn run_test(&self, test: &PerformanceTest) -> Result<PerformanceTestResult> {
        let collector = Arc::new(PerformanceMetricsCollector::new());

        // 记录性能指标
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                150.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                200.0,
                "ms",
                labels,
            )
            .await;

        // 分析性能指标
        let analyzer = PerformanceAnalyzer::new(collector.clone());
        let analysis = analyzer.analyze_performance("response_time").await;

        let passed = analysis.metric_count == 3
            && analysis.average_value == 150.0
            && analysis.max_value == 200.0
            && analysis.min_value == 100.0;

        Ok(PerformanceTestResult {
            test_name: test.name.clone(),
            passed,
            metrics_collected: 3,
            analysis_performed: true,
        })
    }
}

/// 性能指标测试结果
#[derive(Debug, Clone, Serialize)]
pub struct PerformanceTestResult {
    pub test_name: String,
    pub passed: bool,
    pub metrics_collected: usize,
    pub analysis_performed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_metric_type_as_str() {
        assert_eq!(
            PerformanceMetricType::ResponseTime.as_str(),
            "RESPONSE_TIME"
        );
        assert_eq!(PerformanceMetricType::Throughput.as_str(), "THROUGHPUT");
        assert_eq!(PerformanceMetricType::ErrorRate.as_str(), "ERROR_RATE");
        assert_eq!(PerformanceMetricType::Latency.as_str(), "LATENCY");
        assert_eq!(
            PerformanceMetricType::ResourceUsage.as_str(),
            "RESOURCE_USAGE"
        );
    }

    #[tokio::test]
    async fn performance_metrics_collector_new() {
        let collector = PerformanceMetricsCollector::new();
        let metrics = collector.get_metrics().await;
        assert!(metrics.is_empty());
    }

    #[tokio::test]
    async fn performance_metrics_collector_record_metric() {
        let collector = PerformanceMetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels,
            )
            .await;
        let metrics = collector.get_metrics().await;
        assert_eq!(metrics.len(), 1);
    }

    #[tokio::test]
    async fn performance_metrics_collector_get_metrics_by_name() {
        let collector = PerformanceMetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "throughput",
                PerformanceMetricType::Throughput,
                1000.0,
                "req/s",
                labels,
            )
            .await;
        let metrics = collector.get_metrics_by_name("response_time").await;
        assert_eq!(metrics.len(), 1);
    }

    #[tokio::test]
    async fn performance_metrics_collector_calculate_average() {
        let collector = PerformanceMetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                200.0,
                "ms",
                labels,
            )
            .await;
        let average = collector.calculate_average("response_time").await;
        assert_eq!(average, 150.0);
    }

    #[tokio::test]
    async fn performance_metrics_collector_calculate_max() {
        let collector = PerformanceMetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                200.0,
                "ms",
                labels,
            )
            .await;
        let max = collector.calculate_max("response_time").await;
        assert_eq!(max, 200.0);
    }

    #[tokio::test]
    async fn performance_metrics_collector_calculate_min() {
        let collector = PerformanceMetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                200.0,
                "ms",
                labels,
            )
            .await;
        let min = collector.calculate_min("response_time").await;
        assert_eq!(min, 100.0);
    }

    #[tokio::test]
    async fn performance_analyzer_analyze_performance() {
        let collector = Arc::new(PerformanceMetricsCollector::new());
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                100.0,
                "ms",
                labels.clone(),
            )
            .await;
        collector
            .record_metric(
                "response_time",
                PerformanceMetricType::ResponseTime,
                200.0,
                "ms",
                labels,
            )
            .await;
        let analyzer = PerformanceAnalyzer::new(collector);
        let analysis = analyzer.analyze_performance("response_time").await;
        assert_eq!(analysis.metric_count, 2);
        assert_eq!(analysis.average_value, 150.0);
    }

    #[test]
    fn performance_test_suite_new_default() {
        let suite = PerformanceTestSuite::new_default();
        assert_eq!(suite.tests.len(), 2);
    }

    #[tokio::test]
    async fn performance_test_suite_run_all() {
        let suite = PerformanceTestSuite::new_default();
        let results = suite.run_all().await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.passed));
    }
}
