use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use personal_taoli_core::{
    archive::default_gap_path, observer::observe_continuously, scan::ScanReport,
};

use crate::error::{ApiResponse, ErrorCode};

use super::{
    dto::{ContinuousStatus, FeedSummary, PairReconnectSmokeResult, ReconnectSmokeResult},
    support::{api_fail, api_map, load_config},
};

/// millisecond timestamp of the last report and its serialized shape.
type ProgressSnapshot = (u64, Value);

struct RunningSession {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tauri::async_runtime::JoinHandle<anyhow::Result<()>>,
    archive_path: PathBuf,
    gap_path: PathBuf,
    started_at_ms: u64,
    progress: Arc<Mutex<Option<ProgressSnapshot>>>,
    finished: Arc<std::sync::atomic::AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

/// App-managed handle over the single continuous-observation lifecycle.
///
/// At most one session may run at a time. The session task owns the read-only
/// observer loop (`no_orders`); stopping signals shutdown and waits for the
/// archive writer to flush before returning.
pub struct SessionController {
    inner: Mutex<Option<RunningSession>>,
}

impl Default for SessionController {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

impl SessionController {
    /// Starts continuous observation. `config_path` falls back to the default
    /// config search; `archive_override` falls back to the config archive path.
    /// `on_failure` (used by the supervised autostart path) exits the app when
    /// the observer loop dies, so a supervisor can restart it.
    pub async fn start(
        &self,
        config_path: Option<String>,
        archive_override: Option<String>,
        on_failure: Option<AppHandle>,
    ) -> Result<ContinuousStatus> {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let mut guard = lock(&self.inner);
        if guard.is_some() {
            bail!("continuous observation is already running");
        }

        let (_, config) = load_config(config_path)?;
        let archive_path = archive_override
            .map(PathBuf::from)
            .unwrap_or_else(|| config.archive.path.clone());
        let task_archive_path = archive_path.clone();
        let gap_path = default_gap_path(&archive_path);

        let progress = Arc::new(Mutex::new(None::<ProgressSnapshot>));
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None::<String>));
        let task_progress = Arc::clone(&progress);
        let task_finished = Arc::clone(&finished);
        let task_error = Arc::clone(&error);
        let started_at_ms = now_ms();

        let task = tauri::async_runtime::spawn(async move {
            let result = observe_continuously(
                &config,
                &task_archive_path,
                |report: &ScanReport| {
                    if let Ok(mut slot) = task_progress.lock() {
                        *slot = Some((
                            now_ms(),
                            serde_json::to_value(report).unwrap_or(Value::Null),
                        ));
                    }
                },
                async move {
                    let _ = shutdown_rx.await;
                },
            )
            .await;
            if let Ok(mut slot) = task_error.lock() {
                *slot = result.as_ref().err().map(|error| format!("{error:#}"));
            }
            task_finished.store(true, std::sync::atomic::Ordering::SeqCst);
            if let (Some(handle), Err(error)) = (&on_failure, &result) {
                eprintln!("continuous observation failed: {error:#}");
                handle.exit(1);
            }
            result
        });

        *guard = Some(RunningSession {
            shutdown: shutdown_tx,
            task,
            archive_path,
            gap_path,
            started_at_ms,
            progress,
            finished,
            error,
        });
        Ok(Self::status_locked(&guard))
    }

    /// Signals the running session to stop and waits for the archive writer to
    /// flush. Returns `Ok(None)` when no session was running.
    pub async fn stop(&self, flush_timeout: Duration) -> Result<Option<ContinuousStatus>> {
        let Some(session) = lock(&self.inner).take() else {
            return Ok(None);
        };
        let RunningSession {
            shutdown,
            task,
            progress: _,
            finished: _,
            error: _,
            archive_path: _,
            gap_path: _,
            started_at_ms: _,
        } = session;
        let _ = shutdown.send(());
        match tokio::time::timeout(flush_timeout, task).await {
            Ok(Ok(result)) => result.context("continuous observation task failed")?,
            Ok(Err(error)) => bail!("continuous observation task panicked: {error}"),
            Err(_) => bail!("timed out waiting for continuous observation to stop"),
        }
        Ok(Some(Self::status_locked(&lock(&self.inner))))
    }

    /// Current lifecycle snapshot without side effects.
    pub fn status(&self) -> ContinuousStatus {
        Self::status_locked(&lock(&self.inner))
    }

    fn status_locked(
        guard: &std::sync::MutexGuard<'_, Option<RunningSession>>,
    ) -> ContinuousStatus {
        let Some(session) = guard.as_ref() else {
            return ContinuousStatus {
                running: false,
                archive_path: None,
                gap_path: None,
                started_at_ms: None,
                last_report_at_ms: None,
                last_report: None,
                error: None,
            };
        };
        let progress = lock(&session.progress).clone();
        ContinuousStatus {
            running: !session.finished.load(std::sync::atomic::Ordering::SeqCst),
            archive_path: Some(session.archive_path.to_string_lossy().into_owned()),
            gap_path: Some(session.gap_path.to_string_lossy().into_owned()),
            started_at_ms: Some(session.started_at_ms),
            last_report_at_ms: progress.as_ref().map(|(at_ms, _)| *at_ms),
            last_report: progress.map(|(_, report)| report),
            error: lock(&session.error).clone(),
        }
    }
}

#[tauri::command]
pub async fn start_continuous_observation(
    app: AppHandle,
    config_path: Option<String>,
    archive_path: Option<String>,
) -> ApiResponse<ContinuousStatus> {
    let state = app.state::<SessionController>();
    if state.status().running {
        return api_fail(
            ErrorCode::InvalidRequest,
            "continuous observation is already running",
            false,
        );
    }
    api_map(
        state.start(config_path, archive_path, None).await,
        ErrorCode::MarketDataError,
        true,
    )
}

#[tauri::command]
pub async fn stop_continuous_observation(app: AppHandle) -> ApiResponse<ContinuousStatus> {
    let state = app.state::<SessionController>();
    match state.stop(Duration::from_secs(30)).await {
        Ok(Some(status)) => ApiResponse::ok(status),
        Ok(None) => api_fail(
            ErrorCode::InvalidRequest,
            "continuous observation is not running",
            false,
        ),
        Err(error) => api_map(Err(error), ErrorCode::MarketDataError, true),
    }
}

#[tauri::command]
pub fn continuous_observation_status(app: AppHandle) -> ApiResponse<ContinuousStatus> {
    ApiResponse::ok(app.state::<SessionController>().status())
}

#[tauri::command]
pub async fn run_reconnect_smoke(config_path: Option<String>) -> ApiResponse<ReconnectSmokeResult> {
    let result = async {
        let (_, config) = load_config(config_path)?;
        let outcomes = personal_taoli_core::observer::run_reconnect_smoke(&config).await?;
        Ok::<_, anyhow::Error>(ReconnectSmokeResult {
            results: outcomes
                .into_iter()
                .map(|outcome| PairReconnectSmokeResult {
                    symbol: outcome.symbol,
                    binance: FeedSummary::from(&outcome.binance),
                    bybit: FeedSummary::from(&outcome.bybit),
                })
                .collect(),
            no_orders: true,
        })
    }
    .await;
    api_map(result, ErrorCode::MarketDataError, true)
}
