mod binance;
mod binance_stream;
mod bybit;
mod bybit_stream;

use std::fmt;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{instrument::InstrumentSpec, local_book::BookFeed, market::OrderBookSnapshot};

pub use binance::BinanceMarketData;
pub use bybit::BybitMarketData;

/// Supported trading venues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Venue {
    Binance,
    Bybit,
}

impl Venue {
    /// Returns the lowercase string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Bybit => "bybit",
        }
    }

    /// Parses a venue from a string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "binance" => Ok(Self::Binance),
            "bybit" => Ok(Self::Bybit),
            _ => bail!("unsupported venue: {s}"),
        }
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[async_trait]
pub trait MarketDataVenue: Send + Sync {
    fn name(&self) -> &'static str;
    async fn load_instrument(&self, symbol: &str) -> Result<InstrumentSpec>;
    async fn fetch_order_book(&self, symbol: &str, depth: u16) -> Result<OrderBookSnapshot>;
    fn subscribe_order_book(
        &self,
        symbol: &str,
        depth: u16,
        stale_after: std::time::Duration,
        reconnect_delay: std::time::Duration,
    ) -> Result<BookFeed>;
}
