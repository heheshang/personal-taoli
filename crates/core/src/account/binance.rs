use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;

use crate::{config::VenueConfig, market::unix_timestamp_ms};

use super::{Credentials, FeeSchedule, PermissionAssessment};

type HmacSha256 = Hmac<Sha256>;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinancePermissions {
    ip_restrict: bool,
    enable_reading: bool,
    enable_spot_and_margin_trading: bool,
    enable_withdrawals: bool,
    enable_internal_transfer: bool,
    permits_universal_transfer: bool,
    enable_futures: bool,
    enable_margin: bool,
    enable_vanilla_options: bool,
    #[serde(default)]
    enable_portfolio_margin_trading: bool,
}

pub(super) async fn binance_permissions(
    client: &Client,
    config: &VenueConfig,
    credentials: &Credentials,
    recv_window_ms: u64,
) -> Result<PermissionAssessment> {
    let timestamp = unix_timestamp_ms()?;
    let query = format!("recvWindow={recv_window_ms}&timestamp={timestamp}");
    let payload: BinancePermissions = binance_get(
        client,
        config,
        credentials,
        "sapi/v1/account/apiRestrictions",
        &query,
        "permission",
    )
    .await?;
    let mut scopes = Vec::new();
    for (enabled, name) in [
        (payload.enable_spot_and_margin_trading, "spot_margin_trade"),
        (payload.enable_withdrawals, "withdraw"),
        (payload.enable_internal_transfer, "internal_transfer"),
        (payload.permits_universal_transfer, "universal_transfer"),
        (payload.enable_futures, "futures"),
        (payload.enable_margin, "margin"),
        (payload.enable_vanilla_options, "options"),
        (payload.enable_portfolio_margin_trading, "portfolio_margin"),
    ] {
        if enabled {
            scopes.push(name.to_owned());
        }
    }
    Ok(PermissionAssessment {
        credentials_configured: true,
        verified_at_ms: Some(timestamp),
        read_only: Some(payload.enable_reading && scopes.is_empty()),
        ip_restricted: Some(payload.ip_restrict),
        scopes,
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceCommission {
    symbol: String,
    standard_commission: BinanceRateParts,
    #[serde(default)]
    special_commission: BinanceRateParts,
    tax_commission: BinanceRateParts,
}

#[derive(serde::Deserialize, Default)]
struct BinanceRateParts {
    taker: rust_decimal::Decimal,
    buyer: rust_decimal::Decimal,
    seller: rust_decimal::Decimal,
}

pub(super) async fn binance_fee(
    client: &Client,
    symbol: &str,
    config: &VenueConfig,
    credentials: &Credentials,
    recv_window_ms: u64,
    max_fee_age_ms: u64,
) -> Result<FeeSchedule> {
    let loaded_at_ms = unix_timestamp_ms()?;
    let query = format!("symbol={symbol}&recvWindow={recv_window_ms}&timestamp={loaded_at_ms}");
    let payload: BinanceCommission = binance_get(
        client,
        config,
        credentials,
        "api/v3/account/commission",
        &query,
        "commission",
    )
    .await?;
    if payload.symbol != symbol {
        bail!("Binance commission response returned a different symbol");
    }
    let buy_taker_rate = effective_binance_rate(&payload, true);
    let sell_taker_rate = effective_binance_rate(&payload, false);
    super::validate_fee_rate("Binance buy", buy_taker_rate)?;
    super::validate_fee_rate("Binance sell", sell_taker_rate)?;
    Ok(FeeSchedule {
        symbol: symbol.to_owned(),
        buy_taker_rate,
        sell_taker_rate,
        source: "binance:/api/v3/account/commission",
        loaded_at_ms: Some(loaded_at_ms),
        expires_at_ms: Some(loaded_at_ms.saturating_add(max_fee_age_ms)),
        actual_account_rate: true,
    })
}

pub(super) fn effective_binance_rate(
    payload: &BinanceCommission,
    buy: bool,
) -> rust_decimal::Decimal {
    let side = |parts: &BinanceRateParts| if buy { parts.buyer } else { parts.seller };
    payload.standard_commission.taker
        + side(&payload.standard_commission)
        + payload.special_commission.taker
        + side(&payload.special_commission)
        + payload.tax_commission.taker
        + side(&payload.tax_commission)
}

pub(super) async fn binance_get<T: for<'de> serde::Deserialize<'de>>(
    client: &Client,
    config: &VenueConfig,
    credentials: &Credentials,
    path: &str,
    query: &str,
    operation: &str,
) -> Result<T> {
    let signature = hmac_hex(&credentials.secret, query.as_bytes())?;
    let mut url = config.base_url.parse::<reqwest::Url>()?.join(path)?;
    url.set_query(Some(&format!("{query}&signature={signature}")));
    let response = client
        .get(url)
        .header("X-MBX-APIKEY", credentials.api_key.as_str())
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Binance {operation} request failed"))?;
    if !response.status().is_success() {
        bail!(
            "Binance {operation} request returned HTTP {}",
            response.status()
        );
    }
    response
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("invalid Binance {operation} response"))
}

fn hmac_hex(secret: &str, payload: &[u8]) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .context("failed to initialize request signer")?;
    mac.update(payload);
    let bytes = mac.finalize().into_bytes();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(output)
}
