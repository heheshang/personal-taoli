use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Url};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    instrument::{InstrumentSpec, OrderCapability, TradingStatus},
    local_book::BookFeed,
    market::{Level, OrderBookSnapshot, unix_timestamp_ms},
    venues::{MarketDataVenue, bybit_stream},
};

pub struct BybitMarketData {
    client: Client,
    base_url: Url,
    websocket_url: Url,
}

impl BybitMarketData {
    pub fn new(client: Client, base_url: &str, websocket_url: &str) -> Result<Self> {
        Ok(Self {
            client,
            base_url: Url::parse(base_url).context("invalid Bybit base URL")?,
            websocket_url: Url::parse(websocket_url).context("invalid Bybit WebSocket URL")?,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiResponse<T> {
    ret_code: i64,
    ret_msg: String,
    result: Option<T>,
    time: u64,
}

#[derive(Deserialize)]
struct DepthResult {
    s: String,
    b: Vec<[String; 2]>,
    a: Vec<[String; 2]>,
    u: u64,
    cts: Option<u64>,
    ts: Option<u64>,
}

#[derive(Deserialize)]
struct InstrumentResult {
    category: String,
    list: Vec<BybitInstrument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitInstrument {
    symbol: String,
    status: String,
    base_coin: String,
    quote_coin: String,
    lot_size_filter: LotSizeFilter,
    price_filter: PriceFilter,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LotSizeFilter {
    min_order_qty: String,
    max_order_qty: String,
    qty_step: Option<String>,
    base_precision: Option<String>,
    min_order_amt: String,
    max_order_amt: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceFilter {
    tick_size: String,
}

#[async_trait]
impl MarketDataVenue for BybitMarketData {
    fn name(&self) -> &'static str {
        "bybit"
    }

    async fn load_instrument(&self, symbol: &str) -> Result<InstrumentSpec> {
        let mut url = self.base_url.join("v5/market/instruments-info")?;
        url.query_pairs_mut()
            .append_pair("category", "spot")
            .append_pair("symbol", symbol);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Bybit instrument request failed")?;
        let payload: ApiResponse<InstrumentResult> =
            decode_response(response, "instrument").await?;
        normalize_instrument(payload, symbol)
    }

    async fn fetch_order_book(&self, symbol: &str, depth: u16) -> Result<OrderBookSnapshot> {
        let mut url = self.base_url.join("v5/market/orderbook")?;
        url.query_pairs_mut()
            .append_pair("category", "spot")
            .append_pair("symbol", symbol)
            .append_pair("limit", &depth.to_string());
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Bybit depth request failed")?;
        let payload: ApiResponse<DepthResult> = decode_response(response, "depth").await?;
        let result = payload
            .result
            .context("Bybit depth response omitted result")?;
        if result.s != symbol {
            bail!("Bybit returned symbol {}, expected {symbol}", result.s);
        }
        let snapshot = OrderBookSnapshot {
            venue: self.name().into(),
            symbol: result.s,
            bids: parse_levels(result.b).context("invalid Bybit bid")?,
            asks: parse_levels(result.a).context("invalid Bybit ask")?,
            sequence: result.u,
            source_timestamp_ms: result.cts.or(result.ts),
            received_timestamp_ms: unix_timestamp_ms()?,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn subscribe_order_book(
        &self,
        symbol: &str,
        depth: u16,
        stale_after: Duration,
        reconnect_delay: Duration,
    ) -> Result<BookFeed> {
        if ![1, 50, 200, 1000].contains(&depth) {
            bail!("Bybit WebSocket depth must be one of 1, 50, 200, 1000");
        }
        Ok(bybit_stream::subscribe(
            self.websocket_url.clone(),
            symbol.to_owned(),
            depth,
            stale_after,
            reconnect_delay,
        ))
    }
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    operation: &str,
) -> Result<ApiResponse<T>> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Bybit {operation} returned {status}: {body}");
    }
    let payload: ApiResponse<T> = response
        .json()
        .await
        .with_context(|| format!("invalid Bybit {operation} response"))?;
    if payload.ret_code != 0 {
        bail!(
            "Bybit {operation} rejected request: {} {}",
            payload.ret_code,
            payload.ret_msg
        );
    }
    Ok(payload)
}

fn normalize_instrument(
    payload: ApiResponse<InstrumentResult>,
    expected_symbol: &str,
) -> Result<InstrumentSpec> {
    let result = payload
        .result
        .context("Bybit instrument response omitted result")?;
    if result.category != "spot" {
        bail!("Bybit returned category {}, expected spot", result.category);
    }
    let [symbol]: [BybitInstrument; 1] = result.list.try_into().map_err(|symbols: Vec<_>| {
        anyhow::anyhow!(
            "Bybit returned {} instruments for {expected_symbol}",
            symbols.len()
        )
    })?;
    if symbol.symbol != expected_symbol {
        bail!(
            "Bybit returned symbol {}, expected {expected_symbol}",
            symbol.symbol
        );
    }
    let quantity_step = symbol
        .lot_size_filter
        .qty_step
        .as_deref()
        .or(symbol.lot_size_filter.base_precision.as_deref())
        .context("Bybit instrument omitted qtyStep/basePrecision")?
        .parse()
        .context("Bybit instrument returned invalid quantity step")?;

    Ok(InstrumentSpec {
        venue: "bybit".into(),
        account_type: "spot".into(),
        venue_symbol: symbol.symbol,
        canonical_instrument_id: format!("{}/{}:SPOT", symbol.base_coin, symbol.quote_coin),
        base_asset_id: symbol.base_coin,
        quote_asset_id: symbol.quote_coin.clone(),
        settlement_asset_id: symbol.quote_coin,
        market_type: "spot".into(),
        contract_multiplier: Decimal::ONE,
        quantity_unit: "base_asset".into(),
        price_tick: parse_decimal(&symbol.price_filter.tick_size, "priceFilter.tickSize")?,
        quantity_step,
        min_quantity: parse_decimal(
            &symbol.lot_size_filter.min_order_qty,
            "lotSizeFilter.minOrderQty",
        )?,
        max_quantity: Some(parse_decimal(
            &symbol.lot_size_filter.max_order_qty,
            "lotSizeFilter.maxOrderQty",
        )?),
        min_notional: parse_decimal(
            &symbol.lot_size_filter.min_order_amt,
            "lotSizeFilter.minOrderAmt",
        )?,
        max_notional: symbol
            .lot_size_filter
            .max_order_amt
            .as_deref()
            .map(|value| parse_decimal(value, "lotSizeFilter.maxOrderAmt"))
            .transpose()?,
        trading_status: if symbol.status == "Trading" {
            TradingStatus::Trading
        } else {
            TradingStatus::Unavailable(symbol.status)
        },
        supported_order_types: vec![
            OrderCapability::Limit,
            OrderCapability::Market,
            OrderCapability::Ioc,
            OrderCapability::Fok,
            OrderCapability::PostOnly,
        ],
        metadata_version: payload.time,
    })
}

fn parse_decimal(value: &str, field: &str) -> Result<Decimal> {
    value
        .parse()
        .with_context(|| format!("Bybit instrument returned invalid {field}"))
}

fn parse_levels(raw: Vec<[String; 2]>) -> Result<Vec<Level>> {
    raw.into_iter()
        .map(|[price, quantity]| {
            Ok(Level {
                price: price.parse::<Decimal>()?,
                quantity: quantity.parse::<Decimal>()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_spot_instrument() {
        let raw = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","list":[{"symbol":"BTCUSDT","status":"Trading","baseCoin":"BTC","quoteCoin":"USDT","lotSizeFilter":{"basePrecision":"0.000001","minOrderQty":"0.000001","maxOrderQty":"230","minOrderAmt":"5","maxOrderAmt":"8000000"},"priceFilter":{"tickSize":"0.1"}}]},"time":42}"#;
        let payload: ApiResponse<InstrumentResult> = serde_json::from_str(raw).unwrap();
        let spec = normalize_instrument(payload, "BTCUSDT").unwrap();
        assert_eq!(spec.quantity_step, "0.000001".parse::<Decimal>().unwrap());
        assert_eq!(spec.min_notional, Decimal::from(5));
        assert_eq!(spec.metadata_version, 42);
    }

    #[test]
    fn rejects_duplicate_target_instruments() {
        let raw = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","list":[]},"time":42}"#;
        let payload: ApiResponse<InstrumentResult> = serde_json::from_str(raw).unwrap();
        assert!(
            normalize_instrument(payload, "BTCUSDT")
                .unwrap_err()
                .to_string()
                .contains("0 instruments")
        );
    }
}
