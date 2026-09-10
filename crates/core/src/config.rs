use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairConfig {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub quantity: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverConfig {
    /// 交易对列表，唯一交易对真相源（非空；旧单币种格式加载时自动迁移为单元素列表）。
    pub pairs: Vec<PairConfig>,
    pub poll_interval_ms: u64,
    pub http_timeout_ms: u64,
    pub stream_start_timeout_ms: u64,
    pub reconnect_delay_ms: u64,
    pub max_snapshot_age_ms: u64,
    pub max_pair_skew_ms: u64,
    pub orderbook_depth: u16,
    pub account_refresh_interval_ms: u64,
    pub max_fee_age_ms: u64,
    pub auth_recv_window_ms: u64,
    pub archive: ArchiveConfig,
    pub binance: VenueConfig,
    pub bybit: VenueConfig,
    pub strategy: StrategyConfig,
    #[serde(default)]
    pub simulation: SimulationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueConfig {
    pub base_url: String,
    pub websocket_url: String,
    pub fallback_taker_fee_rate: Decimal,
    pub api_key_env: String,
    pub api_secret_env: String,
    pub region_eligible_confirmed: bool,
    pub account_eligible_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct StrategyConfig {
    pub min_net_profit: Decimal,
    pub min_net_bps: Decimal,
    pub latency_loss_bps: Decimal,
    pub risk_buffer_bps: Decimal,
    pub rebalance_cost: Decimal,
    pub other_direct_cost: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveConfig {
    pub path: PathBuf,
    pub queue_capacity: usize,
    pub raw_retention_days: u16,
}

/// 模拟撮合引擎配置（F-02）。全字段 serde default：旧配置/前端往返缺字段时落入默认。
/// 所有 bps 字段为万分比（0–10000）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SimulationConfig {
    /// 连续观察是否把 accepted 机会接入模拟引擎。
    pub enabled: bool,
    /// 随机种子；0=按运行时间播种（回写报告），非 0=全链路确定性可复现。
    pub seed: u64,
    /// 每 run 买腿注入的报价资产（如 USDT）初始余额。
    pub initial_quote_balance: Decimal,
    /// 每 run 卖腿注入的基础资产（如 BTC）初始余额。
    pub initial_base_balance: Decimal,
    /// 决策→提交时延采样区间（毫秒，闭区间）。
    pub decision_to_submit_ms_min: u64,
    pub decision_to_submit_ms_max: u64,
    /// 提交→成交回报时延采样区间（毫秒，闭区间）。
    pub fill_latency_ms_min: u64,
    pub fill_latency_ms_max: u64,
    /// 成交价不利偏移（万分比）：买侧档价×(1+x)，卖侧档价×(1−x)。
    pub adverse_move_bps: Decimal,
    /// 敌手占盘比例（万分比）：每档有效量 = 档量×(1−x)。
    pub competitor_take_bps: Decimal,
    /// 提交落入 UNKNOWN 的概率（万分比）。
    pub unknown_submit_probability_bps: Decimal,
    /// UNKNOWN 后查询 Found 的概率（万分比）。
    pub query_found_probability_bps: Decimal,
    /// 传给 B-03 的未匹配敞口预算（USDT·unit）。
    pub max_unmatched_exposure: Decimal,
    /// 传给 B-03 的补偿预算（USDT·unit）。
    pub compensation_budget: Decimal,
    /// 补偿单位成本 = 双腿中间价×(1+markup)（万分比）。
    pub compensation_markup_bps: Decimal,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            seed: 0,
            initial_quote_balance: "10000".parse().unwrap(),
            initial_base_balance: "2".parse().unwrap(),
            decision_to_submit_ms_min: 50,
            decision_to_submit_ms_max: 400,
            fill_latency_ms_min: 20,
            fill_latency_ms_max: 250,
            adverse_move_bps: "15".parse().unwrap(),
            competitor_take_bps: "200".parse().unwrap(),
            unknown_submit_probability_bps: "300".parse().unwrap(),
            query_found_probability_bps: "6000".parse().unwrap(),
            max_unmatched_exposure: "100".parse().unwrap(),
            compensation_budget: "20".parse().unwrap(),
            compensation_markup_bps: "30".parse().unwrap(),
        }
    }
}

impl SimulationConfig {
    fn validate(&self) -> Result<()> {
        const TEN_THOUSAND: Decimal = Decimal::from_parts(10000, 0, 0, false, 0);
        if self.decision_to_submit_ms_min > self.decision_to_submit_ms_max {
            bail!("simulation.decision_to_submit_ms_min must not exceed its max");
        }
        if self.fill_latency_ms_min > self.fill_latency_ms_max {
            bail!("simulation.fill_latency_ms_min must not exceed its max");
        }
        for (name, value) in [
            ("adverse_move_bps", self.adverse_move_bps),
            ("competitor_take_bps", self.competitor_take_bps),
            (
                "unknown_submit_probability_bps",
                self.unknown_submit_probability_bps,
            ),
            (
                "query_found_probability_bps",
                self.query_found_probability_bps,
            ),
            ("compensation_markup_bps", self.compensation_markup_bps),
        ] {
            if value < Decimal::ZERO || value > TEN_THOUSAND {
                bail!("simulation.{name} must be in [0, 10000], got {value}");
            }
        }
        if self.initial_quote_balance <= Decimal::ZERO {
            bail!("simulation.initial_quote_balance must be positive");
        }
        if self.initial_base_balance <= Decimal::ZERO {
            bail!("simulation.initial_base_balance must be positive");
        }
        if self.max_unmatched_exposure < Decimal::ZERO || self.compensation_budget < Decimal::ZERO {
            bail!("simulation exposure and compensation budgets must be non-negative");
        }
        Ok(())
    }
}

impl ObserverConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Self = toml::from_str(&Self::migrate_legacy_toml(&raw))
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_from_json(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("config file {} does not exist", path.display());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Self = serde_json::from_str(&Self::migrate_legacy_json(&raw))
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save_to_json(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let json =
            serde_json::to_string_pretty(self).context("failed to serialize config to JSON")?;
        fs::write(path, json)
            .with_context(|| format!("failed to write config {}", path.display()))?;
        Ok(())
    }

    /// 参与观察的交易对列表（唯一真相源，非空由 validate 保证）。
    pub fn effective_pairs(&self) -> &[PairConfig] {
        &self.pairs
    }

    /// 旧格式（顶层 symbol 单数字段）→ 新格式（pairs 列表）一次性文本迁移。
    ///
    /// 先用廉价子串探测分流：只有「像旧格式」的文本才走完整的 `Value` 解析 + 重序列化，
    /// 否则原样借用返回（调用方本就要再解析一次，旧写法等于把新格式解析两遍）。
    fn migrate_legacy_toml(raw: &str) -> Cow<'_, str> {
        if raw.contains("pairs") || !raw.contains("symbol") {
            return Cow::Borrowed(raw);
        }
        let Ok(value) = toml::from_str::<toml::Value>(raw) else {
            return Cow::Borrowed(raw);
        };
        let Some(table) = value.as_table() else {
            return Cow::Borrowed(raw);
        };
        if table.contains_key("pairs") || !table.contains_key("symbol") {
            return Cow::Borrowed(raw);
        }
        let mut pair = toml::map::Map::new();
        for key in ["symbol", "base_asset", "quote_asset", "quantity"] {
            if let Some(item) = table.get(key) {
                pair.insert(key.to_string(), item.clone());
            }
        }
        let mut migrated = table.clone();
        for key in ["symbol", "base_asset", "quote_asset", "quantity"] {
            migrated.remove(key);
        }
        migrated.insert(
            "pairs".to_string(),
            toml::Value::Array(vec![toml::Value::Table(pair)]),
        );
        match toml::to_string(&toml::Value::Table(migrated)) {
            Ok(migrated) => Cow::Owned(migrated),
            Err(_) => Cow::Borrowed(raw),
        }
    }

    /// 旧格式（顶层 symbol 单数字段）→ 新格式（pairs 列表）一次性文本迁移（JSON 变体）。
    fn migrate_legacy_json(raw: &str) -> Cow<'_, str> {
        if raw.contains("pairs") || !raw.contains("symbol") {
            return Cow::Borrowed(raw);
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return Cow::Borrowed(raw);
        };
        let Some(object) = value.as_object() else {
            return Cow::Borrowed(raw);
        };
        if object.contains_key("pairs") || !object.contains_key("symbol") {
            return Cow::Borrowed(raw);
        }
        let mut pair = serde_json::Map::new();
        for key in ["symbol", "base_asset", "quote_asset", "quantity"] {
            if let Some(item) = object.get(key) {
                pair.insert(key.to_string(), item.clone());
            }
        }
        let mut migrated = object.clone();
        for key in ["symbol", "base_asset", "quote_asset", "quantity"] {
            migrated.remove(key);
        }
        migrated.insert(
            "pairs".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::Object(pair)]),
        );
        match serde_json::to_string(&serde_json::Value::Object(migrated)) {
            Ok(migrated) => Cow::Owned(migrated),
            Err(_) => Cow::Borrowed(raw),
        }
    }

    pub fn default_config() -> Self {
        Self {
            pairs: vec![PairConfig {
                symbol: "BTCUSDT".to_string(),
                base_asset: "BTC".to_string(),
                quote_asset: "USDT".to_string(),
                quantity: "0.001".parse().unwrap(),
            }],
            poll_interval_ms: 2000,
            http_timeout_ms: 3000,
            stream_start_timeout_ms: 10000,
            reconnect_delay_ms: 1000,
            max_snapshot_age_ms: 1000,
            max_pair_skew_ms: 500,
            orderbook_depth: 50,
            account_refresh_interval_ms: 300000,
            max_fee_age_ms: 900000,
            auth_recv_window_ms: 5000,
            archive: ArchiveConfig {
                path: "data/archive/observations.ndjson".into(),
                queue_capacity: 1024,
                raw_retention_days: 30,
            },
            binance: VenueConfig {
                base_url: "https://api.binance.com".to_string(),
                websocket_url: "wss://stream.binance.com:9443".to_string(),
                fallback_taker_fee_rate: "0.001".parse().unwrap(),
                api_key_env: "TAOLI_BINANCE_API_KEY".to_string(),
                api_secret_env: "TAOLI_BINANCE_API_SECRET".to_string(),
                region_eligible_confirmed: false,
                account_eligible_confirmed: false,
            },
            bybit: VenueConfig {
                base_url: "https://api.bybit.com".to_string(),
                websocket_url: "wss://stream.bybit.com/v5/public/spot".to_string(),
                fallback_taker_fee_rate: "0.001".parse().unwrap(),
                api_key_env: "TAOLI_BYBIT_API_KEY".to_string(),
                api_secret_env: "TAOLI_BYBIT_API_SECRET".to_string(),
                region_eligible_confirmed: false,
                account_eligible_confirmed: false,
            },
            strategy: StrategyConfig {
                min_net_profit: "0.01".parse().unwrap(),
                min_net_bps: "1".parse().unwrap(),
                latency_loss_bps: "1".parse().unwrap(),
                risk_buffer_bps: "1".parse().unwrap(),
                rebalance_cost: "0".parse().unwrap(),
                other_direct_cost: "0".parse().unwrap(),
            },
            simulation: SimulationConfig::default(),
        }
    }

    pub fn http_timeout(&self) -> Duration {
        Duration::from_millis(self.http_timeout_ms)
    }
    pub fn stream_start_timeout(&self) -> Duration {
        Duration::from_millis(self.stream_start_timeout_ms)
    }

    pub fn reconnect_delay(&self) -> Duration {
        Duration::from_millis(self.reconnect_delay_ms)
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.poll_interval_ms)
    }

    pub fn account_refresh_interval(&self) -> Duration {
        Duration::from_millis(self.account_refresh_interval_ms)
    }

    fn validate(&self) -> Result<()> {
        if self.pairs.is_empty() {
            bail!("pairs must not be empty");
        }
        let mut seen = std::collections::HashSet::new();
        for pair in &self.pairs {
            if pair.symbol.trim().is_empty()
                || pair.base_asset.trim().is_empty()
                || pair.quote_asset.trim().is_empty()
            {
                bail!("pair symbol, base_asset and quote_asset must not be empty");
            }
            if pair.quantity <= Decimal::ZERO {
                bail!("pair {} quantity must be positive", pair.symbol);
            }
            if !seen.insert(pair.symbol.clone()) {
                bail!("duplicate pair symbol {}", pair.symbol);
            }
        }
        if self.poll_interval_ms == 0
            || self.http_timeout_ms == 0
            || self.stream_start_timeout_ms == 0
            || self.reconnect_delay_ms == 0
            || self.account_refresh_interval_ms == 0
            || self.max_fee_age_ms == 0
            || self.auth_recv_window_ms == 0
        {
            bail!(
                "poll, HTTP, stream, reconnect, account refresh, fee age and auth durations must be positive"
            );
        }
        if self.auth_recv_window_ms > 60_000 {
            bail!("auth_recv_window_ms must not exceed 60000");
        }
        if self.max_snapshot_age_ms == 0 || self.max_pair_skew_ms == 0 {
            bail!("snapshot age and pair skew limits must be positive");
        }
        const SHARED_STREAM_DEPTHS: [u16; 3] = [50, 200, 1000];
        if !SHARED_STREAM_DEPTHS.contains(&self.orderbook_depth) {
            bail!("orderbook_depth must be one of 50, 200, 1000");
        }
        if self.archive.path.as_os_str().is_empty() {
            bail!("archive.path must not be empty");
        }
        if self.archive.queue_capacity == 0 {
            bail!("archive.queue_capacity must be positive");
        }
        if self.archive.raw_retention_days == 0 {
            bail!("archive.raw_retention_days must be positive");
        }
        validate_venue("binance", &self.binance)?;
        validate_venue("bybit", &self.bybit)?;
        validate_non_negative("min_net_profit", self.strategy.min_net_profit)?;
        validate_non_negative("min_net_bps", self.strategy.min_net_bps)?;
        validate_non_negative("latency_loss_bps", self.strategy.latency_loss_bps)?;
        validate_non_negative("risk_buffer_bps", self.strategy.risk_buffer_bps)?;
        validate_non_negative("rebalance_cost", self.strategy.rebalance_cost)?;
        validate_non_negative("other_direct_cost", self.strategy.other_direct_cost)?;
        self.simulation.validate()?;
        Ok(())
    }
}

fn validate_venue(name: &str, venue: &VenueConfig) -> Result<()> {
    if !(venue.base_url.starts_with("https://") || venue.base_url.starts_with("http://127.0.0.1")) {
        bail!("{name}.base_url must use HTTPS (localhost is allowed for tests)");
    }
    if !(venue.websocket_url.starts_with("wss://")
        || venue.websocket_url.starts_with("ws://127.0.0.1"))
    {
        bail!("{name}.websocket_url must use WSS (localhost is allowed for tests)");
    }
    if venue.fallback_taker_fee_rate < Decimal::ZERO
        || venue.fallback_taker_fee_rate >= Decimal::ONE
    {
        bail!("{name}.fallback_taker_fee_rate must be in [0, 1)");
    }
    if venue.api_key_env.trim().is_empty() || venue.api_secret_env.trim().is_empty() {
        bail!("{name} credential environment variable names must not be empty");
    }
    Ok(())
}

fn validate_non_negative(name: &str, value: Decimal) -> Result<()> {
    if value < Decimal::ZERO {
        bail!("{name} must be non-negative");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_positive_quantity() {
        let raw = include_str!("../tests/fixtures/observer.toml")
            .replace("quantity = \"0.001\"", "quantity = \"0\"");
        let config: ObserverConfig = toml::from_str(&raw).unwrap();
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("quantity")
        );
    }

    #[test]
    fn rejects_depth_not_supported_by_both_streams() {
        let raw = include_str!("../tests/fixtures/observer.toml")
            .replace("orderbook_depth = 50", "orderbook_depth = 25");
        let config: ObserverConfig = toml::from_str(&raw).unwrap();
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("orderbook_depth")
        );
    }

    #[test]
    fn migrates_legacy_toml_with_single_symbol_to_pairs() {
        let raw = r##"
symbol = "BTCUSDT"
base_asset = "BTC"
quote_asset = "USDT"
quantity = "0.001"
poll_interval_ms = 2000
[http]
"##;
        let migrated = ObserverConfig::migrate_legacy_toml(raw);
        assert!(migrated.contains("[[pairs]]"));
        let value: toml::Value = toml::from_str(&migrated).unwrap();
        assert!(
            !value.as_table().unwrap().contains_key("symbol"),
            "top-level symbol must be removed"
        );
        let pairs = value.get("pairs").unwrap().as_array().unwrap();
        assert_eq!(pairs.len(), 1);
        let pair = pairs[0].as_table().unwrap();
        assert_eq!(pair.get("symbol").unwrap().as_str().unwrap(), "BTCUSDT");
        assert_eq!(pair.get("base_asset").unwrap().as_str(), Some("BTC"));
        assert_eq!(pair.get("quote_asset").unwrap().as_str(), Some("USDT"));
        assert_eq!(pair.get("quantity").unwrap().as_str(), Some("0.001"));
        assert_ne!(
            value.get("poll_interval_ms").unwrap().as_integer().unwrap(),
            0
        );
    }

    #[test]
    fn migrates_legacy_json_with_single_symbol_to_pairs() {
        let raw = r#"{"symbol":"BTCUSDT","base_asset":"BTC","quote_asset":"USDT","quantity":"0.001","poll_interval_ms":2000}"#;
        let migrated = ObserverConfig::migrate_legacy_json(raw);
        let value: serde_json::Value = serde_json::from_str(&migrated).unwrap();
        assert!(
            value.get("symbol").is_none(),
            "top-level symbol must be removed"
        );
        assert!(migrated.contains("\"pairs\""));
        let pairs = value.get("pairs").unwrap().as_array().unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].get("symbol").unwrap().as_str().unwrap(), "BTCUSDT");
        assert_eq!(pairs[0].get("base_asset").unwrap().as_str(), Some("BTC"));
    }

    #[test]
    fn rejects_duplicate_pair_symbol() {
        let pair_block = "[[pairs]]\nsymbol = \"BTCUSDT\"\nbase_asset = \"BTC\"\nquote_asset = \"USDT\"\nquantity = \"0.001\"";
        let raw = include_str!("../tests/fixtures/observer.toml")
            .replace(pair_block, &format!("{pair_block}\n{pair_block}"));
        let config: ObserverConfig = toml::from_str(&raw).unwrap();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate pair symbol"), "{err}");
    }

    #[test]
    fn rejects_empty_pairs() {
        let raw =
            include_str!("../tests/fixtures/observer.toml").replace("[[pairs]]", "pairs = []");
        let config: ObserverConfig = toml::from_str(&raw).unwrap();
        assert!(config.validate().unwrap_err().to_string().contains("pairs"));
    }

    #[test]
    fn simulation_defaults_when_table_missing() {
        let raw = include_str!("../tests/fixtures/observer.toml");
        let config: ObserverConfig = toml::from_str(raw).unwrap();
        let sim = config.simulation.clone();
        assert!(!sim.enabled);
        assert_eq!(sim.adverse_move_bps, "15".parse::<Decimal>().unwrap());
        assert_eq!(sim.competitor_take_bps, "200".parse::<Decimal>().unwrap());
        config.validate().unwrap();
    }

    #[test]
    fn simulation_validate_rejects_out_of_range_bps() {
        let sim = SimulationConfig {
            adverse_move_bps: "10001".parse().unwrap(),
            ..SimulationConfig::default()
        };
        assert!(
            sim.validate()
                .unwrap_err()
                .to_string()
                .contains("adverse_move_bps")
        );
        let sim = SimulationConfig {
            adverse_move_bps: "0".parse().unwrap(),
            competitor_take_bps: Decimal::NEGATIVE_ONE,
            ..SimulationConfig::default()
        };
        assert!(
            sim.validate()
                .unwrap_err()
                .to_string()
                .contains("competitor_take_bps")
        );
    }

    #[test]
    fn simulation_validate_rejects_bad_ranges_and_balances() {
        let sim = SimulationConfig {
            decision_to_submit_ms_min: 500,
            decision_to_submit_ms_max: 100,
            ..SimulationConfig::default()
        };
        assert!(
            sim.validate()
                .unwrap_err()
                .to_string()
                .contains("decision_to_submit_ms_min")
        );
        let sim2 = SimulationConfig {
            initial_quote_balance: Decimal::ZERO,
            ..SimulationConfig::default()
        };
        assert!(
            sim2.validate()
                .unwrap_err()
                .to_string()
                .contains("initial_quote_balance")
        );
        let sim3 = SimulationConfig {
            compensation_budget: Decimal::NEGATIVE_ONE,
            ..SimulationConfig::default()
        };
        assert!(
            sim3.validate()
                .unwrap_err()
                .to_string()
                .contains("non-negative")
        );
    }
}
