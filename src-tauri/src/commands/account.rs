use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::account::{AccountData, load_account_data};

use super::support::{api_error, http_client, load_config};

#[tauri::command]
pub async fn account_status(config_path: Option<String>) -> ApiResponse<[AccountData; 2]> {
    let result = async {
        let (_, config) = load_config(config_path)?;
        let client = http_client(&config)?;
        Ok::<_, anyhow::Error>(
            load_account_data(
                &client,
                &config.symbol,
                &config.binance,
                &config.bybit,
                config.auth_recv_window_ms,
                config.max_fee_age_ms,
            )
            .await,
        )
    }
    .await;
    match result {
        Ok(accounts) => ApiResponse::ok(accounts),
        Err(error) => ApiResponse::fail(api_error(ErrorCode::NetworkError, error, true)),
    }
}
