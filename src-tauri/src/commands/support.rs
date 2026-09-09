use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use reqwest::Client;

use personal_taoli_core::config::ObserverConfig;

use crate::error::{ApiError, ApiResponse, ErrorCode};

pub(super) fn config_path(path: Option<String>) -> PathBuf {
    if let Some(path) = path {
        return PathBuf::from(path);
    }
    let relative = PathBuf::from("config/observer.json");
    if relative.is_file() {
        return relative;
    }
    if let Ok(executable) = env::current_exe() {
        for ancestor in executable.ancestors().skip(1) {
            let candidate = ancestor.join(&relative);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    relative
}

pub(super) fn load_config(path: Option<String>) -> Result<(PathBuf, ObserverConfig)> {
    let path = config_path(path);
    if path.exists() {
        let config = ObserverConfig::load_from_json(&path).with_context(|| {
            format!(
                "failed to load config {}; launch from project root or provide a config path",
                path.display()
            )
        })?;
        Ok((path, config))
    } else {
        // Fallback: try TOML for backward compatibility
        let toml_path = path.with_extension("toml");
        if toml_path.exists() {
            let config = ObserverConfig::load(&toml_path).with_context(|| {
                format!(
                    "failed to load config {}; launch from project root or provide a config path",
                    toml_path.display()
                )
            })?;
            return Ok((toml_path, config));
        }
        // Use default config
        let config = ObserverConfig::default_config();
        Ok((path, config))
    }
}

pub(super) fn http_client(config: &ObserverConfig) -> Result<Client> {
    Client::builder()
        .timeout(config.http_timeout())
        .user_agent(concat!("personal-taoli/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")
}

/// Reads `TAOLI_DATABASE_URL` for smoke commands; missing and blank values
/// both count as absent.
pub(super) fn database_url() -> Option<String> {
    env::var("TAOLI_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Direct failure response for validation and early-return paths.
pub(super) fn api_fail<T>(
    code: ErrorCode,
    message: impl std::fmt::Display,
    retryable: bool,
) -> ApiResponse<T> {
    ApiResponse::fail(ApiError::new(code, format!("{message}"), retryable))
}

/// Classifies a command result: success passes through unchanged, failure is
/// reported under `code` with the full error chain and retryability.
pub(super) fn api_map<T, E: std::fmt::Display>(
    result: Result<T, E>,
    code: ErrorCode,
    retryable: bool,
) -> ApiResponse<T> {
    match result {
        Ok(value) => ApiResponse::ok(value),
        Err(error) => ApiResponse::fail(ApiError::new(code, format!("{error:#}"), retryable)),
    }
}
