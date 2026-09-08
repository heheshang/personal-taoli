use crate::error::{ApiResponse, ErrorCode};

use super::{
    dto::{ConfigSummary, DesktopStatus},
    support::{api_error, load_config},
};

#[tauri::command]
pub fn desktop_status() -> ApiResponse<DesktopStatus> {
    ApiResponse::ok(DesktopStatus {
        mode: "READ_ONLY_AND_PAPER",
        version: env!("CARGO_PKG_VERSION"),
        real_order_capability: false,
    })
}

#[tauri::command]
pub fn load_config_summary(config_path: Option<String>) -> ApiResponse<ConfigSummary> {
    match load_config(config_path) {
        Ok((_, config)) => ApiResponse::ok(ConfigSummary {
            symbol: config.symbol,
            base_asset: config.base_asset,
            quote_asset: config.quote_asset,
            quantity: config.quantity,
            orderbook_depth: config.orderbook_depth,
            archive_path: config.archive.path.display().to_string(),
            binance_websocket_url: config.binance.websocket_url,
            bybit_websocket_url: config.bybit.websocket_url,
        }),
        Err(error) => ApiResponse::fail(api_error(ErrorCode::ConfigError, error, false)),
    }
}
