use std::{env, fs, path::PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::error::{ApiResponse, ErrorCode};

use super::support::api_fail;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub database_url: String,
    pub binance_api_key: String,
    pub binance_api_secret: String,
    pub bybit_api_key: String,
    pub bybit_api_secret: String,
}

fn config_file_path() -> PathBuf {
    let candidates = ["config/app-config.json", "app-config.json"];
    for candidate in &candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("config/app-config.json")
}

#[tauri::command]
pub async fn load_app_config() -> ApiResponse<AppConfig> {
    let path = config_file_path();

    let mut config = AppConfig {
        database_url: env::var("TAOLI_DATABASE_URL").unwrap_or_default(),
        binance_api_key: env::var("TAOLI_BINANCE_API_KEY").unwrap_or_default(),
        binance_api_secret: env::var("TAOLI_BINANCE_API_SECRET").unwrap_or_default(),
        bybit_api_key: env::var("TAOLI_BYBIT_API_KEY").unwrap_or_default(),
        bybit_api_secret: env::var("TAOLI_BYBIT_API_SECRET").unwrap_or_default(),
    };

    if path.exists() {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(saved) = serde_json::from_str::<AppConfig>(&raw) {
                if !saved.database_url.is_empty() {
                    config.database_url = saved.database_url;
                }
                if !saved.binance_api_key.is_empty() {
                    config.binance_api_key = saved.binance_api_key;
                }
                if !saved.binance_api_secret.is_empty() {
                    config.binance_api_secret = saved.binance_api_secret;
                }
                if !saved.bybit_api_key.is_empty() {
                    config.bybit_api_key = saved.bybit_api_key;
                }
                if !saved.bybit_api_secret.is_empty() {
                    config.bybit_api_secret = saved.bybit_api_secret;
                }
            }
        }
    }

    ApiResponse::ok(config)
}

#[tauri::command]
pub async fn save_app_config(config: AppConfig) -> ApiResponse<()> {
    let path = config_file_path();

    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))
        {
            return api_fail(
                ErrorCode::ConfigError,
                format!("failed to create config directory: {e}"),
                false,
            );
        }
    }

    let json = match serde_json::to_string_pretty(&config) {
        Ok(json) => json,
        Err(e) => {
            return api_fail(
                ErrorCode::ConfigError,
                format!("failed to serialize config: {e}"),
                false,
            );
        }
    };

    if let Err(e) = fs::write(&path, &json)
        .with_context(|| format!("failed to write config to {}", path.display()))
    {
        return api_fail(
            ErrorCode::ConfigError,
            format!("failed to write config: {e}"),
            false,
        );
    }

    // Environment variables will be loaded from JSON config on next startup
    // For current session, they are already set via load_app_config

    ApiResponse::ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_file_path() {
        let path = config_file_path();
        assert!(path.to_string_lossy().contains("app-config.json"));
    }
}
