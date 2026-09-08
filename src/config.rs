use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
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
    pub binance: VenueConfig,
    pub bybit: VenueConfig,
    pub strategy: StrategyConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VenueConfig {
    pub base_url: String,
    pub websocket_url: String,
    pub fallback_taker_fee_rate: Decimal,
    pub api_key_env: String,
    pub api_secret_env: String,
    pub region_eligible_confirmed: bool,
    pub account_eligible_confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    pub min_net_profit: Decimal,
    pub min_net_bps: Decimal,
    pub latency_loss_bps: Decimal,
    pub risk_buffer_bps: Decimal,
    pub rebalance_cost: Decimal,
    pub other_direct_cost: Decimal,
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
        let raw = include_str!("../config/observer.toml")
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
        let raw = include_str!("../config/observer.toml")
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
