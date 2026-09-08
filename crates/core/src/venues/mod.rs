mod binance;
mod binance_stream;
mod bybit;
mod bybit_stream;

use anyhow::Result;
use async_trait::async_trait;

use crate::{instrument::InstrumentSpec, local_book::BookFeed, market::OrderBookSnapshot};

pub use binance::BinanceMarketData;
pub use bybit::BybitMarketData;

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
