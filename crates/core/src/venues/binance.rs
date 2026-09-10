use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Url};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    instrument::{InstrumentSpec, OrderCapability, TradingStatus},
    local_book::BookFeed,
    market::{OrderBookSnapshot, parse_levels, unix_timestamp_ms},
    venues::{MarketDataVenue, binance_stream},
};

pub struct BinanceMarketData {
    pub(super) client: Client,
    pub(super) base_url: Url,
    websocket_url: Url,
}

impl BinanceMarketData {
    pub fn new(client: Client, base_url: &str, websocket_url: &str) -> Result<Self> {
        Ok(Self {
            client,
            base_url: Url::parse(base_url).context("invalid Binance base URL")?,
            websocket_url: Url::parse(websocket_url).context("invalid Binance WebSocket URL")?,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepthResponse {
    last_update_id: u64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeInfoResponse {
    server_time: u64,
    symbols: Vec<SymbolInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolInfo {
    symbol: String,
    status: String,
    base_asset: String,
    quote_asset: String,
    order_types: Vec<String>,
    filters: Vec<SymbolFilter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolFilter {
    filter_type: String,
    tick_size: Option<String>,
    min_qty: Option<String>,
    max_qty: Option<String>,
    step_size: Option<String>,
    min_notional: Option<String>,
    max_notional: Option<String>,
}

#[async_trait]
impl MarketDataVenue for BinanceMarketData {
    fn name(&self) -> &'static str {
        "binance"
    }

    async fn load_instrument(&self, symbol: &str) -> Result<InstrumentSpec> {
        let mut url = self.base_url.join("api/v3/exchangeInfo")?;
        url.query_pairs_mut().append_pair("symbol", symbol);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Binance instrument request failed")?;
        let payload: ExchangeInfoResponse = decode_response(response, "instrument").await?;
        normalize_instrument(payload, symbol)
    }

    async fn fetch_order_book(&self, symbol: &str, depth: u16) -> Result<OrderBookSnapshot> {
        fetch_depth_snapshot(&self.client, &self.base_url, symbol, depth).await
    }

    fn subscribe_order_book(
        &self,
        symbol: &str,
        depth: u16,
        stale_after: Duration,
        reconnect_delay: Duration,
    ) -> Result<BookFeed> {
        Ok(binance_stream::subscribe(
            self.client.clone(),
            self.base_url.clone(),
            self.websocket_url.clone(),
            symbol.to_owned(),
            depth,
            stale_after,
            reconnect_delay,
        ))
    }
}
pub(super) async fn fetch_depth_snapshot(
    client: &Client,
    base_url: &Url,
    symbol: &str,
    depth: u16,
) -> Result<OrderBookSnapshot> {
    let mut url = base_url.join("api/v3/depth")?;
    url.query_pairs_mut()
        .append_pair("symbol", symbol)
        .append_pair("limit", &depth.to_string());
    let response = client
        .get(url)
        .send()
        .await
        .context("Binance depth request failed")?;
    let payload: DepthResponse = decode_response(response, "depth").await?;
    let snapshot = OrderBookSnapshot {
        venue: "binance".into(),
        symbol: symbol.into(),
        bids: parse_levels(payload.bids).context("invalid Binance bid")?,
        asks: parse_levels(payload.asks).context("invalid Binance ask")?,
        sequence: payload.last_update_id,
        // This REST endpoint does not expose a source timestamp.
        source_timestamp_ms: None,
        received_timestamp_ms: unix_timestamp_ms()?,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    operation: &str,
) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Binance {operation} returned {status}: {body}");
    }
    response
        .json()
        .await
        .with_context(|| format!("invalid Binance {operation} response"))
}

fn normalize_instrument(
    payload: ExchangeInfoResponse,
    expected_symbol: &str,
) -> Result<InstrumentSpec> {
    let [symbol]: [SymbolInfo; 1] = payload.symbols.try_into().map_err(|symbols: Vec<_>| {
        anyhow::anyhow!(
            "Binance returned {} instruments for {expected_symbol}",
            symbols.len()
        )
    })?;
    if symbol.symbol != expected_symbol {
        bail!(
            "Binance returned symbol {}, expected {expected_symbol}",
            symbol.symbol
        );
    }
    let price_filter = required_filter(&symbol.filters, "PRICE_FILTER")?;
    let lot_filter = required_filter(&symbol.filters, "LOT_SIZE")?;
    let notional_filter = symbol
        .filters
        .iter()
        .find(|filter| filter.filter_type == "NOTIONAL")
        .or_else(|| {
            symbol
                .filters
                .iter()
                .find(|filter| filter.filter_type == "MIN_NOTIONAL")
        })
        .context("Binance instrument omitted NOTIONAL/MIN_NOTIONAL filter")?;
    let supports_limit = symbol
        .order_types
        .iter()
        .any(|order_type| order_type == "LIMIT");
    let mut supported_order_types = Vec::new();
    if supports_limit {
        supported_order_types.extend([
            OrderCapability::Limit,
            OrderCapability::Ioc,
            OrderCapability::Fok,
        ]);
    }
    if symbol
        .order_types
        .iter()
        .any(|order_type| order_type == "MARKET")
    {
        supported_order_types.push(OrderCapability::Market);
    }
    if symbol
        .order_types
        .iter()
        .any(|order_type| order_type == "LIMIT_MAKER")
    {
        supported_order_types.push(OrderCapability::PostOnly);
    }

    Ok(InstrumentSpec {
        venue: "binance".into(),
        account_type: "spot".into(),
        venue_symbol: symbol.symbol,
        canonical_instrument_id: format!("{}/{}:SPOT", symbol.base_asset, symbol.quote_asset),
        base_asset_id: symbol.base_asset,
        quote_asset_id: symbol.quote_asset.clone(),
        settlement_asset_id: symbol.quote_asset,
        market_type: "spot".into(),
        contract_multiplier: Decimal::ONE,
        quantity_unit: "base_asset".into(),
        price_tick: required_decimal(price_filter.tick_size.as_deref(), "PRICE_FILTER.tickSize")?,
        quantity_step: required_decimal(lot_filter.step_size.as_deref(), "LOT_SIZE.stepSize")?,
        min_quantity: required_decimal(lot_filter.min_qty.as_deref(), "LOT_SIZE.minQty")?,
        max_quantity: optional_decimal(lot_filter.max_qty.as_deref(), "LOT_SIZE.maxQty")?,
        min_notional: required_decimal(
            notional_filter.min_notional.as_deref(),
            "NOTIONAL.minNotional",
        )?,
        max_notional: optional_decimal(
            notional_filter.max_notional.as_deref(),
            "NOTIONAL.maxNotional",
        )?,
        trading_status: if symbol.status == "TRADING" {
            TradingStatus::Trading
        } else {
            TradingStatus::Unavailable(symbol.status)
        },
        supported_order_types,
        metadata_version: payload.server_time,
    })
}

fn required_filter<'a>(filters: &'a [SymbolFilter], filter_type: &str) -> Result<&'a SymbolFilter> {
    filters
        .iter()
        .find(|filter| filter.filter_type == filter_type)
        .with_context(|| format!("Binance instrument omitted {filter_type} filter"))
}

fn required_decimal(value: Option<&str>, field: &str) -> Result<Decimal> {
    value
        .with_context(|| format!("Binance instrument omitted {field}"))?
        .parse()
        .with_context(|| format!("Binance instrument returned invalid {field}"))
}

fn optional_decimal(value: Option<&str>, field: &str) -> Result<Option<Decimal>> {
    value
        .map(|value| {
            value
                .parse()
                .with_context(|| format!("Binance instrument returned invalid {field}"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_required_spot_filters() {
        let raw = r#"{"serverTime":42,"symbols":[{"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT","orderTypes":["LIMIT","LIMIT_MAKER","MARKET"],"filters":[{"filterType":"PRICE_FILTER","tickSize":"0.01"},{"filterType":"LOT_SIZE","minQty":"0.00001","maxQty":"10","stepSize":"0.00001"},{"filterType":"MIN_NOTIONAL","minNotional":"5"}]}]}"#;
        let payload: ExchangeInfoResponse = serde_json::from_str(raw).unwrap();
        let spec = normalize_instrument(payload, "BTCUSDT").unwrap();
        assert_eq!(spec.price_tick, "0.01".parse::<Decimal>().unwrap());
        assert_eq!(spec.quantity_step, "0.00001".parse::<Decimal>().unwrap());
        assert_eq!(spec.min_notional, Decimal::from(5));
        assert!(spec.supported_order_types.contains(&OrderCapability::Ioc));
    }

    #[test]
    fn rejects_missing_lot_size_filter() {
        let raw = r#"{"serverTime":42,"symbols":[{"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT","orderTypes":["LIMIT"],"filters":[{"filterType":"PRICE_FILTER","tickSize":"0.01"},{"filterType":"MIN_NOTIONAL","minNotional":"5"}]}]}"#;
        let payload: ExchangeInfoResponse = serde_json::from_str(raw).unwrap();
        assert!(
            normalize_instrument(payload, "BTCUSDT")
                .unwrap_err()
                .to_string()
                .contains("LOT_SIZE")
        );
    }
}
