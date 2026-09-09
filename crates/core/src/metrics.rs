//! 监控指标模块，用于收集和暴露系统监控指标。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 指标类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MetricType {
    /// 计数器
    Counter,
    /// 仪表盘
    Gauge,
    /// 直方图
    Histogram,
    /// 摘要
    Summary,
}

impl MetricType {
    /// 获取类型名称
    pub fn as_str(&self) -> &'static str {
        match self {
            MetricType::Counter => "COUNTER",
            MetricType::Gauge => "GAUGE",
            MetricType::Histogram => "HISTOGRAM",
            MetricType::Summary => "SUMMARY",
        }
    }
}

/// 指标值
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricValue {
    /// 整数值
    Integer(i64),
    /// 浮点值
    Float(f64),
}

/// 指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// 指标名称
    pub name: String,
    /// 指标类型
    pub metric_type: MetricType,
    /// 指标值
    pub value: MetricValue,
    /// 指标标签
    pub labels: HashMap<String, String>,
    /// 指标时间戳
    pub timestamp_ms: u64,
}

/// 指标收集器
pub struct MetricsCollector {
    metrics: Arc<RwLock<HashMap<String, Metric>>>,
}

impl MetricsCollector {
    /// 创建新的指标收集器
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 记录计数器指标
    pub async fn record_counter(&self, name: &str, value: i64, labels: HashMap<String, String>) {
        let mut metrics = self.metrics.write().await;
        let metric = Metric {
            name: name.to_string(),
            metric_type: MetricType::Counter,
            value: MetricValue::Integer(value),
            labels,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        metrics.insert(name.to_string(), metric);
    }

    /// 记录仪表盘指标
    pub async fn record_gauge(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        let mut metrics = self.metrics.write().await;
        let metric = Metric {
            name: name.to_string(),
            metric_type: MetricType::Gauge,
            value: MetricValue::Float(value),
            labels,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        metrics.insert(name.to_string(), metric);
    }

    /// 记录直方图指标
    pub async fn record_histogram(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        let mut metrics = self.metrics.write().await;
        let metric = Metric {
            name: name.to_string(),
            metric_type: MetricType::Histogram,
            value: MetricValue::Float(value),
            labels,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        metrics.insert(name.to_string(), metric);
    }

    /// 获取所有指标
    pub async fn get_metrics(&self) -> HashMap<String, Metric> {
        self.metrics.read().await.clone()
    }

    /// 获取特定指标
    pub async fn get_metric(&self, name: &str) -> Option<Metric> {
        self.metrics.read().await.get(name).cloned()
    }

    /// 清除所有指标
    pub async fn clear_metrics(&self) {
        self.metrics.write().await.clear();
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// 系统指标
pub struct SystemMetrics {
    collector: Arc<MetricsCollector>,
}

impl SystemMetrics {
    /// 创建新的系统指标
    pub fn new(collector: Arc<MetricsCollector>) -> Self {
        Self { collector }
    }

    /// 记录系统启动时间
    pub async fn record_startup_time(&self, startup_time_ms: u64) {
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        self.collector
            .record_gauge("system_startup_time_ms", startup_time_ms as f64, labels)
            .await;
    }

    /// 记录系统运行时间
    pub async fn record_uptime(&self, uptime_seconds: u64) {
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        self.collector
            .record_gauge("system_uptime_seconds", uptime_seconds as f64, labels)
            .await;
    }

    /// 记录内存使用
    pub async fn record_memory_usage(&self, usage_bytes: u64) {
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        self.collector
            .record_gauge("system_memory_usage_bytes", usage_bytes as f64, labels)
            .await;
    }

    /// 记录CPU使用
    pub async fn record_cpu_usage(&self, usage_percent: f64) {
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        self.collector
            .record_gauge("system_cpu_usage_percent", usage_percent, labels)
            .await;
    }

    /// 记录磁盘使用
    pub async fn record_disk_usage(&self, usage_bytes: u64) {
        let mut labels = HashMap::new();
        labels.insert("component".to_string(), "system".to_string());
        self.collector
            .record_gauge("system_disk_usage_bytes", usage_bytes as f64, labels)
            .await;
    }
}

/// 交易指标
pub struct TradingMetrics {
    collector: Arc<MetricsCollector>,
}

impl TradingMetrics {
    /// 创建新的交易指标
    pub fn new(collector: Arc<MetricsCollector>) -> Self {
        Self { collector }
    }

    /// 记录订单数量
    pub async fn record_order_count(&self, venue: &str, count: i64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_counter("trading_order_count", count, labels)
            .await;
    }

    /// 记录成交数量
    pub async fn record_trade_count(&self, venue: &str, count: i64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_counter("trading_trade_count", count, labels)
            .await;
    }

    /// 记录订单延迟
    pub async fn record_order_latency(&self, venue: &str, latency_ms: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_histogram("trading_order_latency_ms", latency_ms, labels)
            .await;
    }

    /// 记录成交延迟
    pub async fn record_trade_latency(&self, venue: &str, latency_ms: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_histogram("trading_trade_latency_ms", latency_ms, labels)
            .await;
    }

    /// 记录收益
    pub async fn record_pnl(&self, venue: &str, pnl: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_gauge("trading_pnl", pnl, labels)
            .await;
    }
}

/// 市场指标
pub struct MarketMetrics {
    collector: Arc<MetricsCollector>,
}

impl MarketMetrics {
    /// 创建新的市场指标
    pub fn new(collector: Arc<MetricsCollector>) -> Self {
        Self { collector }
    }

    /// 记录行情延迟
    pub async fn record_market_data_latency(&self, venue: &str, latency_ms: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_histogram("market_data_latency_ms", latency_ms, labels)
            .await;
    }

    /// 记录行情断档
    pub async fn record_market_data_gap(&self, venue: &str, gap_seconds: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_gauge("market_data_gap_seconds", gap_seconds, labels)
            .await;
    }

    /// 记录订单簿深度
    pub async fn record_orderbook_depth(&self, venue: &str, depth: i64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_gauge("market_orderbook_depth", depth as f64, labels)
            .await;
    }
}

/// 风险指标
pub struct RiskMetrics {
    collector: Arc<MetricsCollector>,
}

impl RiskMetrics {
    /// 创建新的风险指标
    pub fn new(collector: Arc<MetricsCollector>) -> Self {
        Self { collector }
    }

    /// 记录敞口
    pub async fn record_exposure(&self, venue: &str, exposure_usd: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_gauge("risk_exposure_usd", exposure_usd, labels)
            .await;
    }

    /// 记录保证金使用率
    pub async fn record_margin_usage(&self, venue: &str, usage_percent: f64) {
        let mut labels = HashMap::new();
        labels.insert("venue".to_string(), venue.to_string());
        self.collector
            .record_gauge("risk_margin_usage_percent", usage_percent, labels)
            .await;
    }

    /// 记录风险事件
    pub async fn record_risk_event(&self, event_type: &str, count: i64) {
        let mut labels = HashMap::new();
        labels.insert("event_type".to_string(), event_type.to_string());
        self.collector
            .record_counter("risk_event_count", count, labels)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_type_as_str() {
        assert_eq!(MetricType::Counter.as_str(), "COUNTER");
        assert_eq!(MetricType::Gauge.as_str(), "GAUGE");
        assert_eq!(MetricType::Histogram.as_str(), "HISTOGRAM");
        assert_eq!(MetricType::Summary.as_str(), "SUMMARY");
    }

    #[tokio::test]
    async fn metrics_collector_new() {
        let collector = MetricsCollector::new();
        let metrics = collector.get_metrics().await;
        assert!(metrics.is_empty());
    }

    #[tokio::test]
    async fn metrics_collector_record_counter() {
        let collector = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("test".to_string(), "value".to_string());
        collector.record_counter("test_counter", 1, labels).await;
        let metric = collector.get_metric("test_counter").await.unwrap();
        assert_eq!(metric.metric_type, MetricType::Counter);
    }

    #[tokio::test]
    async fn metrics_collector_record_gauge() {
        let collector = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("test".to_string(), "value".to_string());
        collector.record_gauge("test_gauge", 1.5, labels).await;
        let metric = collector.get_metric("test_gauge").await.unwrap();
        assert_eq!(metric.metric_type, MetricType::Gauge);
    }

    #[tokio::test]
    async fn metrics_collector_record_histogram() {
        let collector = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("test".to_string(), "value".to_string());
        collector
            .record_histogram("test_histogram", 2.5, labels)
            .await;
        let metric = collector.get_metric("test_histogram").await.unwrap();
        assert_eq!(metric.metric_type, MetricType::Histogram);
    }

    #[tokio::test]
    async fn metrics_collector_clear_metrics() {
        let collector = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("test".to_string(), "value".to_string());
        collector.record_counter("test_counter", 1, labels).await;
        collector.clear_metrics().await;
        let metrics = collector.get_metrics().await;
        assert!(metrics.is_empty());
    }

    #[tokio::test]
    async fn system_metrics_record_startup_time() {
        let collector = Arc::new(MetricsCollector::new());
        let metrics = SystemMetrics::new(collector.clone());
        metrics.record_startup_time(1000).await;
        let metric = collector
            .get_metric("system_startup_time_ms")
            .await
            .unwrap();
        assert_eq!(metric.metric_type, MetricType::Gauge);
    }

    #[tokio::test]
    async fn trading_metrics_record_order_count() {
        let collector = Arc::new(MetricsCollector::new());
        let metrics = TradingMetrics::new(collector.clone());
        metrics.record_order_count("binance", 10).await;
        let metric = collector.get_metric("trading_order_count").await.unwrap();
        assert_eq!(metric.metric_type, MetricType::Counter);
    }

    #[tokio::test]
    async fn market_metrics_record_market_data_latency() {
        let collector = Arc::new(MetricsCollector::new());
        let metrics = MarketMetrics::new(collector.clone());
        metrics.record_market_data_latency("binance", 50.0).await;
        let metric = collector
            .get_metric("market_data_latency_ms")
            .await
            .unwrap();
        assert_eq!(metric.metric_type, MetricType::Histogram);
    }

    #[tokio::test]
    async fn risk_metrics_record_exposure() {
        let collector = Arc::new(MetricsCollector::new());
        let metrics = RiskMetrics::new(collector.clone());
        metrics.record_exposure("binance", 1000.0).await;
        let metric = collector.get_metric("risk_exposure_usd").await.unwrap();
        assert_eq!(metric.metric_type, MetricType::Gauge);
    }
}
