use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::account::{AccountData, load_account_data};

use super::support::{api_map, http_client, load_config};

#[tauri::command]
pub async fn account_status(config_path: Option<String>) -> ApiResponse<Vec<AccountData>> {
    let result = async {
        let (_, config) = load_config(config_path)?;
        let client = http_client(&config)?;
        let mut accounts = Vec::with_capacity(config.effective_pairs().len() * 2);
        for pair in config.effective_pairs() {
            accounts.extend(
                load_account_data(
                    &client,
                    &pair.symbol,
                    &config.binance,
                    &config.bybit,
                    config.auth_recv_window_ms,
                    config.max_fee_age_ms,
                )
                .await,
            );
        }
        Ok::<_, anyhow::Error>(accounts)
    }
    .await;
    api_map(result, ErrorCode::NetworkError, true)
}
