use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverConfig {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub quantity: Decimal,
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

impl ObserverConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Self = toml::from_str(&raw)
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
        let config: Self = serde_json::from_str(&raw)
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

    pub fn default_config() -> Self {
        Self {
            symbol: "BTCUSDT".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDT".to_string(),
            quantity: "0.001".parse().unwrap(),
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
        if self.symbol.trim().is_empty()
            || self.base_asset.trim().is_empty()
            || self.quote_asset.trim().is_empty()
        {
            bail!("symbol, base_asset and quote_asset must not be empty");
        }
        if self.quantity <= Decimal::ZERO {
            bail!("quantity must be positive");
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
}
