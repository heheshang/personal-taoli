use std::env;

use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Serialize;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::config::VenueConfig;

mod binance;
mod bybit;
#[cfg(test)]
mod test_cases;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize)]
pub struct AccountData {
    pub venue: &'static str,
    pub capability: CapabilityCard,
    pub permission: PermissionAssessment,
    pub fee: FeeSchedule,
    pub rejection_reasons: Vec<String>,
}

impl AccountData {
    pub fn admission_rejections(&self, now_ms: u64) -> Vec<String> {
        let mut reasons = self.rejection_reasons.clone();
        if !self.fee.actual_account_rate {
            reasons.push(format!(
                "{} actual account fee unavailable; configured fallback is observation-only",
                self.venue
            ));
        } else if self.fee.expires_at_ms.is_none_or(|expiry| now_ms > expiry) {
            reasons.push(format!("{} actual account fee is expired", self.venue));
        }
        reasons
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityCard {
    pub region_eligible_confirmed: bool,
    pub account_eligible_confirmed: bool,
    pub order_recovery: &'static str,
    pub rate_limits: &'static str,
    pub client_order_id: &'static str,
    pub ioc_fok: &'static str,
    pub fee_currency: &'static str,
    pub history_window: &'static str,
    pub sources: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionAssessment {
    pub credentials_configured: bool,
    pub verified_at_ms: Option<u64>,
    pub read_only: Option<bool>,
    pub ip_restricted: Option<bool>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeeSchedule {
    pub symbol: String,
    pub buy_taker_rate: Decimal,
    pub sell_taker_rate: Decimal,
    pub source: &'static str,
    pub loaded_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub actual_account_rate: bool,
}

impl FeeSchedule {
    pub(crate) fn fallback(symbol: &str, rate: Decimal) -> Self {
        Self {
            symbol: symbol.to_owned(),
            buy_taker_rate: rate,
            sell_taker_rate: rate,
            source: "configured_fallback",
            loaded_at_ms: None,
            expires_at_ms: None,
            actual_account_rate: false,
        }
    }
}

pub(crate) struct Credentials {
    api_key: Zeroizing<String>,
    secret: Zeroizing<String>,
}

impl Credentials {
    fn from_environment(config: &VenueConfig) -> Result<Option<Self>> {
        let api_key = env::var(&config.api_key_env).ok().map(Zeroizing::new);
        let secret = env::var(&config.api_secret_env).ok().map(Zeroizing::new);
        match (api_key, secret) {
            (None, None) => Ok(None),
            (Some(api_key), Some(secret)) if !api_key.is_empty() && !secret.is_empty() => {
                Ok(Some(Self { api_key, secret }))
            }
            _ => bail!("credential environment variables are incomplete"),
        }
    }
}

pub async fn load_account_data(
    client: &Client,
    symbol: &str,
    binance: &VenueConfig,
    bybit: &VenueConfig,
    recv_window_ms: u64,
    max_fee_age_ms: u64,
) -> [AccountData; 2] {
    let (binance_data, bybit_data) = tokio::join!(
        load_binance(client, symbol, binance, recv_window_ms, max_fee_age_ms),
        load_bybit(client, symbol, bybit, recv_window_ms, max_fee_age_ms),
    );
    [
        binance_data.unwrap_or_else(|error| failed_account_data("binance", symbol, binance, error)),
        bybit_data.unwrap_or_else(|error| failed_account_data("bybit", symbol, bybit, error)),
    ]
}

fn failed_account_data(
    venue: &'static str,
    symbol: &str,
    config: &VenueConfig,
    error: anyhow::Error,
) -> AccountData {
    AccountData {
        venue,
        capability: capability_card(venue, config),
        permission: PermissionAssessment {
            credentials_configured: credentials_configured(config),
            verified_at_ms: None,
            read_only: None,
            ip_restricted: None,
            scopes: Vec::new(),
        },
        fee: FeeSchedule::fallback(symbol, config.fallback_taker_fee_rate),
        rejection_reasons: vec![format!(
            "{venue} account metadata refresh failed: {error:#}"
        )],
    }
}

async fn load_binance(
    client: &Client,
    symbol: &str,
    config: &VenueConfig,
    recv_window_ms: u64,
    max_fee_age_ms: u64,
) -> Result<AccountData> {
    let capability = capability_card("binance", config);
    let Some(credentials) = Credentials::from_environment(config)? else {
        return Ok(missing_credentials("binance", symbol, config, capability));
    };
    let permission =
        binance::binance_permissions(client, config, &credentials, recv_window_ms).await?;
    let fee = binance::binance_fee(
        client,
        symbol,
        config,
        &credentials,
        recv_window_ms,
        max_fee_age_ms,
    )
    .await?;
    let mut rejection_reasons = eligibility_rejections("binance", &capability);
    if permission.read_only != Some(true) {
        rejection_reasons.push("binance API key is not read-only".into());
    }
    if !permission.scopes.is_empty() {
        rejection_reasons.push(format!(
            "binance API key has forbidden permissions: {}",
            permission.scopes.join(",")
        ));
    }
    Ok(AccountData {
        venue: "binance",
        capability,
        permission,
        fee,
        rejection_reasons,
    })
}

async fn load_bybit(
    client: &Client,
    symbol: &str,
    config: &VenueConfig,
    recv_window_ms: u64,
    max_fee_age_ms: u64,
) -> Result<AccountData> {
    let capability = capability_card("bybit", config);
    let Some(credentials) = Credentials::from_environment(config)? else {
        return Ok(missing_credentials("bybit", symbol, config, capability));
    };
    let permission = bybit::bybit_permissions(client, config, &credentials, recv_window_ms).await?;
    let fee = bybit::bybit_fee(
        client,
        symbol,
        config,
        &credentials,
        recv_window_ms,
        max_fee_age_ms,
    )
    .await?;
    let mut rejection_reasons = eligibility_rejections("bybit", &capability);
    if permission.read_only != Some(true) {
        rejection_reasons.push("bybit API key is not read-only".into());
    }
    if permission
        .scopes
        .iter()
        .any(|scope| scope == "Wallet:Withdraw")
    {
        rejection_reasons.push("bybit API key has forbidden withdrawal permission".into());
    }
    Ok(AccountData {
        venue: "bybit",
        capability,
        permission,
        fee,
        rejection_reasons,
    })
}

fn missing_credentials(
    venue: &'static str,
    symbol: &str,
    config: &VenueConfig,
    capability: CapabilityCard,
) -> AccountData {
    let mut rejection_reasons = eligibility_rejections(venue, &capability);
    rejection_reasons.push(format!("{venue} read-only credentials are not configured"));
    AccountData {
        venue,
        capability,
        permission: PermissionAssessment {
            credentials_configured: false,
            verified_at_ms: None,
            read_only: None,
            ip_restricted: None,
            scopes: Vec::new(),
        },
        fee: FeeSchedule::fallback(symbol, config.fallback_taker_fee_rate),
        rejection_reasons,
    }
}

fn eligibility_rejections(venue: &str, capability: &CapabilityCard) -> Vec<String> {
    let mut reasons = Vec::new();
    if !capability.region_eligible_confirmed {
        reasons.push(format!("{venue} region eligibility is not confirmed"));
    }
    if !capability.account_eligible_confirmed {
        reasons.push(format!("{venue} account eligibility is not confirmed"));
    }
    reasons
}

fn credentials_configured(config: &VenueConfig) -> bool {
    env::var_os(&config.api_key_env).is_some() && env::var_os(&config.api_secret_env).is_some()
}

fn capability_card(venue: &'static str, config: &VenueConfig) -> CapabilityCard {
    match venue {
        "binance" => CapabilityCard {
            region_eligible_confirmed: config.region_eligible_confirmed,
            account_eligible_confirmed: config.account_eligible_confirmed,
            order_recovery: "GET /api/v3/order by orderId or origClientOrderId; open/all orders and account trades support reconciliation",
            rate_limits: "IP request weights, UID order counts and response headers; reserve budget for query/cancel recovery",
            client_order_id: "1-36 ASCII letters, digits, '-' or '_'; unique among open orders; reuse only after terminal fill per venue rule",
            ioc_fok: "LIMIT IOC/FOK are venue capabilities; validate symbol filters before every order",
            fee_currency: "commission asset is reported per fill; estimator uses conservative quote-notional equivalent and ignores optional BNB discount",
            history_window: "allOrders and account-trade time ranges are at most 24 hours per request; paginate and archive locally",
            sources: &[
                "https://developers.binance.com/en/docs/products/spot/rest-api",
                "https://developers.binance.com/en/docs/products/spot/filters",
            ],
        },
        "bybit" => CapabilityCard {
            region_eligible_confirmed: config.region_eligible_confirmed,
            account_eligible_confirmed: config.account_eligible_confirmed,
            order_recovery: "GET /v5/order/realtime plus private order stream; /v5/order/history and /v5/execution/list reconcile delayed history",
            rate_limits: "600 requests/5s/IP default plus rolling UID/endpoint limits and X-Bapi-Limit response headers",
            client_order_id: "orderLinkId is 1-36 letters, digits, '-' or '_', and must always be unique",
            ioc_fok: "GTC/IOC/FOK/PostOnly are declared; market orders are converted to protected IOC limits",
            fee_currency: "cumFeeDetail reports actual fee assets; estimator uses account takerFeeRate against quote notional",
            history_window: "order history supports two years with status-dependent retention; each start/end query window is at most 7 days",
            sources: &[
                "https://bybit-exchange.github.io/docs/v5/order/create-order",
                "https://bybit-exchange.github.io/docs/v5/order/order-list",
                "https://bybit-exchange.github.io/docs/v5/rate-limit",
            ],
        },
        _ => unreachable!("unsupported venue"),
    }
}

fn validate_fee_rate(name: &str, rate: Decimal) -> Result<()> {
    if rate < Decimal::ZERO || rate >= Decimal::ONE {
        bail!("{name} fee rate is outside [0, 1)");
    }
    Ok(())
}

pub(crate) fn hmac_hex(secret: &str, payload: &[u8]) -> Result<String> {
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
