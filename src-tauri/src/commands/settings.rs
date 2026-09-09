use std::{env, fs, path::PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use personal_taoli_core::config::ObserverConfig;

use crate::error::{ApiResponse, ErrorCode};

use super::support::{api_fail, api_map, load_config};

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

/// Effective env domain config: shell environment values merged with
/// non-empty fields from the JSON file. Shared by the `load_app_config`
/// command and the startup loader in `lib.rs`.
pub(crate) fn load_app_config_file() -> AppConfig {
    let path = config_file_path();

    let mut config = AppConfig {
        database_url: env::var("TAOLI_DATABASE_URL").unwrap_or_default(),
        binance_api_key: env::var("TAOLI_BINANCE_API_KEY").unwrap_or_default(),
        binance_api_secret: env::var("TAOLI_BINANCE_API_SECRET").unwrap_or_default(),
        bybit_api_key: env::var("TAOLI_BYBIT_API_KEY").unwrap_or_default(),
        bybit_api_secret: env::var("TAOLI_BYBIT_API_SECRET").unwrap_or_default(),
    };

    if path.exists()
        && let Ok(raw) = fs::read_to_string(&path)
        && let Ok(saved) = serde_json::from_str::<AppConfig>(&raw)
    {
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

    config
}

#[tauri::command]
pub async fn load_app_config() -> ApiResponse<AppConfig> {
    ApiResponse::ok(load_app_config_file())
}

#[tauri::command]
pub async fn save_app_config(config: AppConfig) -> ApiResponse<()> {
    let path = config_file_path();

    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))
    {
        return api_fail(
            ErrorCode::ConfigError,
            format!("failed to create config directory: {e}"),
            false,
        );
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

    // Loaded on next startup by `lib.rs::run` setup, which injects the saved
    // values into the TAOLI_* environment variables.

    ApiResponse::ok(())
}

/// Returns the full effective observer config (JSON if present, TOML as
/// backward-compatible fallback, built-in defaults otherwise).
#[tauri::command]
pub async fn get_observer_config(config_path: Option<String>) -> ApiResponse<ObserverConfig> {
    match load_config(config_path) {
        Ok((_, config)) => ApiResponse::ok(config),
        Err(error) => api_map(Err(error), ErrorCode::ConfigError, false),
    }
}

/// Persists the full observer config as JSON. Takes effect immediately: every
/// config load (summary, observe, session, account) prefers the JSON file.
#[tauri::command]
pub async fn save_observer_config(
    config_path: Option<String>,
    config: ObserverConfig,
) -> ApiResponse<()> {
    let path = super::support::config_path(config_path);
    if let Err(error) = config.save_to_json(&path) {
        return api_fail(
            ErrorCode::ConfigError,
            format!("failed to save config to {}: {error}", path.display()),
            false,
        );
    }
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
