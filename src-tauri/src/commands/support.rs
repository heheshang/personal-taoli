use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use reqwest::Client;

use personal_taoli_core::config::ObserverConfig;

use crate::error::{ApiError, ErrorCode};

pub(super) fn config_path(path: Option<String>) -> PathBuf {
    if let Some(path) = path {
        return PathBuf::from(path);
    }
    let relative = PathBuf::from("config/observer.toml");
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
    let config = ObserverConfig::load(&path).with_context(|| {
        format!(
            "failed to load config {}; launch from project root or provide a config path",
            path.display()
        )
    })?;
    Ok((path, config))
}

pub(super) fn http_client(config: &ObserverConfig) -> Result<Client> {
    Client::builder()
        .timeout(config.http_timeout())
        .user_agent(concat!("personal-taoli/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")
}

pub(super) fn api_error(
    code: ErrorCode,
    error: impl std::fmt::Display,
    retryable: bool,
) -> ApiError {
    ApiError::new(code, format!("{error:#}"), retryable)
}
