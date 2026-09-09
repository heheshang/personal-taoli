use anyhow::{Context, Result, bail};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{config::VenueConfig, market::unix_timestamp_ms};

use super::{Credentials, FeeSchedule, PermissionAssessment};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitEnvelope<T> {
    ret_code: i64,
    result: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitPermissions {
    read_only: u8,
    permissions: serde_json::Map<String, serde_json::Value>,
    ips: Vec<String>,
}

#[derive(Clone, Copy)]
pub(super) struct AuthWindow {
    pub(super) timestamp_ms: u64,
    pub(super) recv_window_ms: u64,
}

pub(super) async fn bybit_permissions(
    client: &Client,
    config: &VenueConfig,
    credentials: &Credentials,
    recv_window_ms: u64,
) -> Result<PermissionAssessment> {
    let auth = AuthWindow {
        timestamp_ms: unix_timestamp_ms()?,
        recv_window_ms,
    };
    let payload: BybitPermissions = bybit_get(
        client,
        config,
        credentials,
        "v5/user/query-api",
        "",
        auth,
        "permission",
    )
    .await?;
    let mut scopes = Vec::new();
    for (category, values) in payload.permissions {
        let Some(values) = values.as_array() else {
            continue;
        };
        for value in values.iter().filter_map(|value| value.as_str()) {
            scopes.push(format!("{category}:{value}"));
        }
    }
    scopes.sort();
    Ok(PermissionAssessment {
        credentials_configured: true,
        verified_at_ms: Some(auth.timestamp_ms),
        read_only: Some(payload.read_only == 1),
        ip_restricted: Some(!payload.ips.is_empty()),
        scopes,
    })
}

#[derive(Deserialize)]
struct BybitFeeResult {
    list: Vec<BybitFee>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitFee {
    symbol: String,
    taker_fee_rate: Decimal,
}

pub(super) async fn bybit_fee(
    client: &Client,
    symbol: &str,
    config: &VenueConfig,
    credentials: &Credentials,
    recv_window_ms: u64,
    max_fee_age_ms: u64,
) -> Result<FeeSchedule> {
    let auth = AuthWindow {
        timestamp_ms: unix_timestamp_ms()?,
        recv_window_ms,
    };
    let query = format!("category=spot&symbol={symbol}");
    let payload: BybitFeeResult = bybit_get(
        client,
        config,
        credentials,
        "v5/account/fee-rate",
        &query,
        auth,
        "fee",
    )
    .await?;
    let [fee]: [BybitFee; 1] = payload.list.try_into().map_err(|fees: Vec<_>| {
        anyhow::anyhow!("Bybit fee response returned {} rows", fees.len())
    })?;
    if fee.symbol != symbol {
        bail!("Bybit fee response returned a different symbol");
    }
    super::validate_fee_rate("Bybit taker", fee.taker_fee_rate)?;
    Ok(FeeSchedule {
        symbol: symbol.to_owned(),
        buy_taker_rate: fee.taker_fee_rate,
        sell_taker_rate: fee.taker_fee_rate,
        source: "bybit:/v5/account/fee-rate",
        loaded_at_ms: Some(auth.timestamp_ms),
        expires_at_ms: Some(auth.timestamp_ms.saturating_add(max_fee_age_ms)),
        actual_account_rate: true,
    })
}

pub(super) async fn bybit_get<T: for<'de> Deserialize<'de>>(
    client: &Client,
    config: &VenueConfig,
    credentials: &Credentials,
    path: &str,
    query: &str,
    auth: AuthWindow,
    operation: &str,
) -> Result<T> {
    let signing_payload = format!(
        "{}{}{}{query}",
        auth.timestamp_ms,
        credentials.api_key.as_str(),
        auth.recv_window_ms,
    );
    let signature = super::hmac_hex(&credentials.secret, signing_payload.as_bytes())?;
    let mut url = config.base_url.parse::<reqwest::Url>()?.join(path)?;
    if !query.is_empty() {
        url.set_query(Some(query));
    }
    let response = client
        .get(url)
        .header("X-BAPI-API-KEY", credentials.api_key.as_str())
        .header("X-BAPI-TIMESTAMP", auth.timestamp_ms)
        .header("X-BAPI-RECV-WINDOW", auth.recv_window_ms)
        .header("X-BAPI-SIGN", signature)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Bybit {operation} request failed"))?;
    if !response.status().is_success() {
        bail!(
            "Bybit {operation} request returned HTTP {}",
            response.status()
        );
    }
    let envelope: BybitEnvelope<T> = response
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("invalid Bybit {operation} response"))?;
    if envelope.ret_code != 0 {
        bail!(
            "Bybit {operation} request was rejected with code {}",
            envelope.ret_code
        );
    }
    envelope
        .result
        .with_context(|| format!("Bybit {operation} response omitted result"))
}
