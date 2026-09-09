mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread::{self, JoinHandle},
        time::Duration,
    };

    use reqwest::Client;
    use rust_decimal::Decimal;
    use zeroize::Zeroizing;

    use crate::account::{Credentials, binance, bybit, hmac_hex};
    use crate::{
        account::{AccountData, CapabilityCard, FeeSchedule, PermissionAssessment},
        config::VenueConfig,
    };

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
        let payload: binance::BinanceCommission = serde_json::from_str(
            r#"{
                "symbol":"BTCUSDT",
                "standardCommission":{"taker":"0.001","buyer":"0.0001","seller":"0.0002"},
                "specialCommission":{"taker":"0","buyer":"0","seller":"0"},
                "taxCommission":{"taker":"0.00001","buyer":"0.000001","seller":"0.000002"}
            }"#,
        )
        .unwrap();
        assert_eq!(
            binance::effective_binance_rate(&payload, true),
            "0.001111".parse().unwrap()
        );
        assert_eq!(
            binance::effective_binance_rate(&payload, false),
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

        let permission = binance::binance_permissions(&client, &config, &credentials, 5_000)
            .await
            .unwrap();
        let fee = binance::binance_fee(&client, "BTCUSDT", &config, &credentials, 5_000, 60_000)
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

        let permission = bybit::bybit_permissions(&client, &config, &credentials, 5_000)
            .await
            .unwrap();
        let fee = bybit::bybit_fee(&client, "BTCUSDT", &config, &credentials, 5_000, 60_000)
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
        let error = binance::binance_get::<serde_json::Value>(
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

        let binance_error = binance::binance_get::<serde_json::Value>(
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
        let bybit_error = bybit::bybit_get::<serde_json::Value>(
            &client,
            &config,
            &credentials,
            "v5/account/fee-rate",
            "category=spot&symbol=SENSITIVE",
            bybit::AuthWindow {
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
