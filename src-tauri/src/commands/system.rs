use crate::error::{ApiResponse, ErrorCode};

use super::{
    dto::{ConfigSummary, DesktopStatus, PairSummary},
    support::{api_map, load_config},
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
            pairs: config
                .effective_pairs()
                .iter()
                .map(|pair| PairSummary {
                    symbol: pair.symbol.clone(),
                    base_asset: pair.base_asset.clone(),
                    quote_asset: pair.quote_asset.clone(),
                    quantity: pair.quantity,
                })
                .collect(),
            orderbook_depth: config.orderbook_depth,
            archive_path: config.archive.path.display().to_string(),
            binance_websocket_url: config.binance.websocket_url,
            bybit_websocket_url: config.bybit.websocket_url,
        }),
        Err(error) => api_map(Err(error), ErrorCode::ConfigError, false),
    }
}
