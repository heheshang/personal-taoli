use std::env;

use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use reqwest::{Client, Url};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{config::VenueConfig, market::unix_timestamp_ms};

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
    fn fallback(symbol: &str, rate: Decimal) -> Self {
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

struct Credentials {
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
    let permission = binance_permissions(client, config, &credentials, recv_window_ms).await?;
    let fee = binance_fee(
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
    let permission = bybit_permissions(client, config, &credentials, recv_window_ms).await?;
    let fee = bybit_fee(
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

#[derive(Deserialize)]
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

async fn binance_permissions(
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceCommission {
    symbol: String,
    standard_commission: BinanceRateParts,
    #[serde(default)]
    special_commission: BinanceRateParts,
    tax_commission: BinanceRateParts,
}

#[derive(Default, Deserialize)]
struct BinanceRateParts {
    taker: Decimal,
    buyer: Decimal,
    seller: Decimal,
}

async fn binance_fee(
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
    validate_fee_rate("Binance buy", buy_taker_rate)?;
    validate_fee_rate("Binance sell", sell_taker_rate)?;
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

fn effective_binance_rate(payload: &BinanceCommission, buy: bool) -> Decimal {
    let side = |parts: &BinanceRateParts| if buy { parts.buyer } else { parts.seller };
    payload.standard_commission.taker
        + side(&payload.standard_commission)
        + payload.special_commission.taker
        + side(&payload.special_commission)
        + payload.tax_commission.taker
        + side(&payload.tax_commission)
}

async fn binance_get<T: for<'de> Deserialize<'de>>(
    client: &Client,
    config: &VenueConfig,
    credentials: &Credentials,
    path: &str,
    query: &str,
    operation: &str,
) -> Result<T> {
    let signature = hmac_hex(&credentials.secret, query.as_bytes())?;
    let mut url = config.base_url.parse::<Url>()?.join(path)?;
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
struct AuthWindow {
    timestamp_ms: u64,
    recv_window_ms: u64,
}

async fn bybit_permissions(
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

async fn bybit_fee(
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
    validate_fee_rate("Bybit taker", fee.taker_fee_rate)?;
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

async fn bybit_get<T: for<'de> Deserialize<'de>>(
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
    let signature = hmac_hex(&credentials.secret, signing_payload.as_bytes())?;
    let mut url = config.base_url.parse::<Url>()?.join(path)?;
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

fn validate_fee_rate(name: &str, rate: Decimal) -> Result<()> {
    if rate < Decimal::ZERO || rate >= Decimal::ONE {
        bail!("{name} fee rate is outside [0, 1)");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread::{self, JoinHandle},
        time::Duration,
    };

    use super::*;

    const TEST_API_KEY: &str = "test-api-key";
    const TEST_SECRET: &str = "test-secret";

    fn venue(base_url: String) -> VenueConfig {
        VenueConfig {
            base_url,
            websocket_url: "wss://example.invalid".into(),
            fallback_taker_fee_rate: "0.001".parse().unwrap(),
            api_key_env: "TEST_API_KEY".into(),
            api_secret_env: "TEST_API_SECRET".into(),
            region_eligible_confirmed: true,
            account_eligible_confirmed: true,
        }
    }

    fn credentials() -> Credentials {
        Credentials {
            api_key: Zeroizing::new(TEST_API_KEY.into()),
            secret: Zeroizing::new(TEST_SECRET.into()),
        }
    }

    fn spawn_json_server(bodies: Vec<&'static str>) -> (String, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            bodies
                .into_iter()
                .map(|body| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    loop {
                        let count = stream.read(&mut buffer).unwrap();
                        assert!(count > 0, "client closed before completing request headers");
                        request.extend_from_slice(&buffer[..count]);
                        if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                            break;
                        }
                    }
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    String::from_utf8(request).unwrap()
                })
                .collect()
        });
        (format!("http://{address}/"), handle)
    }

    fn request_target(request: &str) -> &str {
        request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap()
    }

    fn request_header<'a>(request: &'a str, expected_name: &str) -> &'a str {
        request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case(expected_name)
                    .then(|| value.trim())
            })
            .unwrap()
    }

    fn assert_binance_signature(request: &str) {
        let query = request_target(request).split_once('?').unwrap().1;
        let (payload, signature) = query.rsplit_once("&signature=").unwrap();
        assert_eq!(
            signature,
            hmac_hex(TEST_SECRET, payload.as_bytes()).unwrap()
        );
        assert_eq!(request_header(request, "X-MBX-APIKEY"), TEST_API_KEY);
    }

    fn assert_bybit_signature(request: &str) {
        let target = request_target(request);
        let query = target.split_once('?').map_or("", |(_, query)| query);
        let timestamp = request_header(request, "X-BAPI-TIMESTAMP");
        let recv_window = request_header(request, "X-BAPI-RECV-WINDOW");
        let payload = format!("{timestamp}{TEST_API_KEY}{recv_window}{query}");
        assert_eq!(
            request_header(request, "X-BAPI-SIGN"),
            hmac_hex(TEST_SECRET, payload.as_bytes()).unwrap()
        );
        assert_eq!(request_header(request, "X-BAPI-API-KEY"), TEST_API_KEY);
    }

    #[test]
    fn binance_documented_hmac_vector_matches() {
        let payload = b"symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        assert_eq!(
            hmac_hex(
                "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j",
                payload,
            )
            .unwrap(),
            "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
        );
    }

    #[test]
    fn binance_side_components_change_effective_fee() {
        let payload: BinanceCommission = serde_json::from_str(
            r#"{
                "symbol":"BTCUSDT",
                "standardCommission":{"taker":"0.001","buyer":"0.0001","seller":"0.0002"},
                "specialCommission":{"taker":"0","buyer":"0","seller":"0"},
                "taxCommission":{"taker":"0.00001","buyer":"0.000001","seller":"0.000002"}
            }"#,
        )
        .unwrap();
        assert_eq!(
            effective_binance_rate(&payload, true),
            "0.001111".parse().unwrap()
        );
        assert_eq!(
            effective_binance_rate(&payload, false),
            "0.001212".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn binance_adapters_sign_requests_and_parse_account_responses() {
        let (base_url, server) = spawn_json_server(vec![
            r#"{
                "ipRestrict":true,
                "enableReading":true,
                "enableSpotAndMarginTrading":false,
                "enableWithdrawals":false,
                "enableInternalTransfer":false,
                "permitsUniversalTransfer":false,
                "enableFutures":false,
                "enableMargin":false,
                "enableVanillaOptions":false,
                "enablePortfolioMarginTrading":false
            }"#,
            r#"{
                "symbol":"BTCUSDT",
                "standardCommission":{"maker":"0","taker":"0.001","buyer":"0.0001","seller":"0.0002"},
                "specialCommission":{"maker":"0","taker":"0","buyer":"0","seller":"0"},
                "taxCommission":{"maker":"0","taker":"0.00001","buyer":"0.000001","seller":"0.000002"},
                "discount":{"enabledForAccount":false,"enabledForSymbol":false,"discountAsset":"BNB","discount":"0"}
            }"#,
        ]);
        let config = venue(base_url);
        let credentials = credentials();
        let client = Client::new();

        let permission = binance_permissions(&client, &config, &credentials, 5_000)
            .await
            .unwrap();
        let fee = binance_fee(&client, "BTCUSDT", &config, &credentials, 5_000, 60_000)
            .await
            .unwrap();
        let requests = server.join().unwrap();

        assert_eq!(permission.read_only, Some(true));
        assert_eq!(permission.ip_restricted, Some(true));
        assert!(permission.scopes.is_empty());
        assert_eq!(fee.buy_taker_rate, "0.001111".parse().unwrap());
        assert_eq!(fee.sell_taker_rate, "0.001212".parse().unwrap());
        assert!(fee.actual_account_rate);
        assert!(
            request_target(&requests[0])
                .starts_with("/sapi/v1/account/apiRestrictions?recvWindow=5000&timestamp=")
        );
        assert!(
            request_target(&requests[1]).starts_with(
                "/api/v3/account/commission?symbol=BTCUSDT&recvWindow=5000&timestamp="
            )
        );
        requests
            .iter()
            .for_each(|request| assert_binance_signature(request));
    }

    #[tokio::test]
    async fn bybit_adapters_sign_requests_and_parse_account_responses() {
        let (base_url, server) = spawn_json_server(vec![
            r#"{
                "retCode":0,
                "retMsg":"OK",
                "result":{"readOnly":1,"permissions":{"ContractTrade":[],"Spot":[],"Wallet":[]},"ips":["192.0.2.1"]},
                "retExtInfo":{},"time":1
            }"#,
            r#"{
                "retCode":0,
                "retMsg":"OK",
                "result":{"category":"spot","list":[{"symbol":"BTCUSDT","takerFeeRate":"0.0008","makerFeeRate":"0.0007"}]},
                "retExtInfo":{},"time":1
            }"#,
        ]);
        let config = venue(base_url);
        let credentials = credentials();
        let client = Client::new();

        let permission = bybit_permissions(&client, &config, &credentials, 5_000)
            .await
            .unwrap();
        let fee = bybit_fee(&client, "BTCUSDT", &config, &credentials, 5_000, 60_000)
            .await
            .unwrap();
        let requests = server.join().unwrap();

        assert_eq!(permission.read_only, Some(true));
        assert_eq!(permission.ip_restricted, Some(true));
        assert!(permission.scopes.is_empty());
        assert_eq!(fee.buy_taker_rate, "0.0008".parse().unwrap());
        assert_eq!(fee.sell_taker_rate, "0.0008".parse().unwrap());
        assert!(fee.actual_account_rate);
        assert_eq!(request_target(&requests[0]), "/v5/user/query-api");
        assert_eq!(
            request_target(&requests[1]),
            "/v5/account/fee-rate?category=spot&symbol=BTCUSDT"
        );
        requests
            .iter()
            .for_each(|request| assert_bybit_signature(request));
    }

    #[tokio::test]
    async fn signed_transport_error_does_not_expose_request_or_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let config = venue(base_url);
        let credentials = credentials();
        let error = binance_get::<serde_json::Value>(
            &Client::new(),
            &config,
            &credentials,
            "api/v3/account/commission",
            "symbol=SENSITIVE&timestamp=1",
            "commission",
        )
        .await
        .unwrap_err()
        .to_string();

        assert_eq!(error, "Binance commission request failed");
        assert!(!error.contains(TEST_API_KEY));
        assert!(!error.contains(TEST_SECRET));
        assert!(!error.contains("SENSITIVE"));
        assert!(!error.contains("signature="));
    }

    #[tokio::test]
    async fn signed_parse_errors_do_not_expose_request_or_credentials() {
        let (base_url, server) = spawn_json_server(vec!["not-json", "not-json"]);
        let config = venue(base_url);
        let credentials = credentials();
        let client = Client::new();

        let binance_error = binance_get::<serde_json::Value>(
            &client,
            &config,
            &credentials,
            "api/v3/account/commission",
            "symbol=SENSITIVE&timestamp=1",
            "commission",
        )
        .await
        .unwrap_err()
        .to_string();
        let bybit_error = bybit_get::<serde_json::Value>(
            &client,
            &config,
            &credentials,
            "v5/account/fee-rate",
            "category=spot&symbol=SENSITIVE",
            AuthWindow {
                timestamp_ms: 1,
                recv_window_ms: 5_000,
            },
            "fee",
        )
        .await
        .unwrap_err()
        .to_string();
        server.join().unwrap();

        assert_eq!(binance_error, "invalid Binance commission response");
        assert_eq!(bybit_error, "invalid Bybit fee response");
        for error in [binance_error, bybit_error] {
            assert!(!error.contains(TEST_API_KEY));
            assert!(!error.contains(TEST_SECRET));
            assert!(!error.contains("SENSITIVE"));
            assert!(!error.contains("signature="));
        }
    }

    #[test]
    fn expired_actual_fee_fails_closed() {
        let data = AccountData {
            venue: "test",
            capability: CapabilityCard {
                region_eligible_confirmed: true,
                account_eligible_confirmed: true,
                order_recovery: "",
                rate_limits: "",
                client_order_id: "",
                ioc_fok: "",
                fee_currency: "",
                history_window: "",
                sources: &[],
            },
            permission: PermissionAssessment {
                credentials_configured: true,
                verified_at_ms: Some(1),
                read_only: Some(true),
                ip_restricted: Some(true),
                scopes: Vec::new(),
            },
            fee: FeeSchedule {
                symbol: "BTCUSDT".into(),
                buy_taker_rate: Decimal::ZERO,
                sell_taker_rate: Decimal::ZERO,
                source: "test",
                loaded_at_ms: Some(1),
                expires_at_ms: Some(10),
                actual_account_rate: true,
            },
            rejection_reasons: Vec::new(),
        };
        assert!(data.admission_rejections(10).is_empty());
        assert_eq!(
            data.admission_rejections(11),
            ["test actual account fee is expired"]
        );
    }
}
