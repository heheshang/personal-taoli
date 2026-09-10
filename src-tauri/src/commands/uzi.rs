//! UZI-Skill analysis: a ticker in, a standalone HTML report out.
//!
//! The pipeline is split so the *analysis* runs here while data collection and
//! HTML rendering stay in the Python skill:
//!
//! ```text
//! 1  python uzi_bridge.py fetch   → .cache/<ticker>/raw_data.json  (slow, network)
//! 2  python uzi_bridge.py quant   → is_quant_factor_style          (network)
//! 3  personal_taoli_uzi::pipeline → dimensions / panel / synthesis
//! 4  python uzi_bridge.py render  → full-report-standalone.html
//! ```
//!
//! Step 1 dominates the wall clock (minutes: it runs the skill's fetchers), so
//! the whole thing is a background task with a polled status, matching the
//! continuous-observation controller next door.
//!
//! The skill directory and interpreter come from `TAOLI_UZI_DIR` and
//! `TAOLI_UZI_PYTHON`. A missing directory fails closed rather than guessing.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::error::{ApiResponse, ErrorCode};

use super::support::{api_fail, api_map};

/// Where a run currently is. Serialised for the UI's progress display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UziStage {
    Fetching,
    Analyzing,
    Rendering,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct UziStatus {
    pub running: bool,
    pub ticker: Option<String>,
    pub stage: Option<UziStage>,
    pub started_at_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    /// Set when `stage == Done`: absolute path of the rendered report.
    pub report_path: Option<String>,
    pub overall_score: Option<f64>,
    pub verdict_label: Option<String>,
    pub detected_style: Option<String>,
    pub investor_count: Option<usize>,
    pub error: Option<String>,
}

impl UziStatus {
    fn idle() -> Self {
        Self {
            running: false,
            ticker: None,
            stage: None,
            started_at_ms: None,
            elapsed_ms: None,
            report_path: None,
            overall_score: None,
            verdict_label: None,
            detected_style: None,
            investor_count: None,
            error: None,
        }
    }
}

/// Shared state the background task writes and the status command reads.
struct Progress {
    stage: Mutex<UziStage>,
    report_path: Mutex<Option<String>>,
    outcome: Mutex<Option<personal_taoli_uzi::pipeline::AnalyzeOutcome>>,
    error: Mutex<Option<String>>,
}

struct Running {
    ticker: String,
    started_at_ms: u64,
    progress: Arc<Progress>,
    /// Kept for cancellation. Whether the run is still going is read from
    /// `progress.stage`, because `tauri::async_runtime::JoinHandle` exposes no
    /// completion query.
    task: tauri::async_runtime::JoinHandle<()>,
}

/// App-managed handle over the single analysis lifecycle.
#[derive(Default)]
pub struct UziController {
    inner: Mutex<Option<Running>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A run is live until the task records a terminal stage.
fn stage_is_live(progress: &Progress) -> bool {
    !matches!(
        *lock(&progress.stage),
        UziStage::Done | UziStage::Failed | UziStage::Cancelled
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Skill checkout, from `TAOLI_UZI_DIR`.
fn skill_dir() -> Result<PathBuf> {
    let raw = std::env::var("TAOLI_UZI_DIR").unwrap_or_default();
    let path = PathBuf::from(raw.trim());
    if raw.trim().is_empty() {
        bail!(
            "TAOLI_UZI_DIR is not set; point it at a UZI-Skill checkout \
             (the directory containing run.py and skills/deep-analysis)"
        );
    }
    if !path.join("run.py").is_file() {
        bail!(
            "TAOLI_UZI_DIR does not look like a UZI-Skill checkout: {} \
             (expected run.py)",
            path.display()
        );
    }
    Ok(path)
}

/// Interpreter, from `TAOLI_UZI_PYTHON`.
fn python() -> String {
    let raw = std::env::var("TAOLI_UZI_PYTHON").unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "python3".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Path of the bridge shim, relative to the running executable's project root.
///
/// The bridge ships with the app rather than with the skill, so it is located
/// via `TAOLI_UZI_BRIDGE` when set, else by walking up from the executable the
/// same way the observer config lookup does.
fn bridge_path() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("TAOLI_UZI_BRIDGE")
        && !explicit.trim().is_empty()
    {
        let path = PathBuf::from(explicit.trim());
        if path.is_file() {
            return Ok(path);
        }
        bail!("TAOLI_UZI_BRIDGE is set but not a file: {}", path.display());
    }

    let relative = PathBuf::from("tools/uzi_bridge.py");
    if relative.is_file() {
        return Ok(relative);
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1) {
            let candidate = ancestor.join(&relative);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!(
        "uzi_bridge.py not found; set TAOLI_UZI_BRIDGE to its path \
         (expected at tools/uzi_bridge.py relative to the project root)"
    )
}

/// Runs one bridge subcommand and returns its stdout.
async fn run_bridge(script: &Path, dir: &Path, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(python())
        .arg(script)
        .args(args)
        .arg(dir)
        .output()
        .await
        .with_context(|| format!("failed to run {} {}", python(), script.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.lines().rev().take(8).collect::<Vec<_>>().join("\n");
        bail!(
            "bridge command {args:?} failed ({}):\n{tail}",
            output.status
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

impl UziController {
    /// Starts an analysis. Fails if one is already running.
    pub fn start(&self, ticker: String) -> Result<UziStatus> {
        let ticker = ticker.trim().to_string();
        if ticker.is_empty() {
            bail!("ticker must not be empty");
        }
        let dir = skill_dir()?;
        let script = bridge_path()?;

        let mut guard = lock(&self.inner);
        if let Some(running) = guard.as_ref()
            && stage_is_live(&running.progress)
        {
            bail!("an analysis is already running for {}", running.ticker);
        }

        let progress = Arc::new(Progress {
            stage: Mutex::new(UziStage::Fetching),
            report_path: Mutex::new(None),
            outcome: Mutex::new(None),
            error: Mutex::new(None),
        });
        let started_at_ms = now_ms();
        let task = spawn_analysis(
            Arc::clone(&progress),
            dir,
            script,
            ticker.clone(),
            started_at_ms,
        );

        *guard = Some(Running {
            ticker,
            started_at_ms,
            progress,
            task,
        });
        Ok(self.status())
    }

    /// Current state, including a failure recorded by the task.
    pub fn status(&self) -> UziStatus {
        let guard = lock(&self.inner);
        let Some(running) = guard.as_ref() else {
            return UziStatus::idle();
        };
        let stage = *lock(&running.progress.stage);
        let running_now = stage_is_live(&running.progress);
        let outcome = lock(&running.progress.outcome);
        UziStatus {
            running: running_now,
            ticker: Some(running.ticker.clone()),
            stage: Some(stage),
            started_at_ms: Some(running.started_at_ms),
            elapsed_ms: Some(now_ms().saturating_sub(running.started_at_ms)),
            report_path: lock(&running.progress.report_path).clone(),
            overall_score: outcome.as_ref().map(|o| o.overall_score),
            verdict_label: outcome.as_ref().map(|o| o.verdict_label.clone()),
            detected_style: outcome.as_ref().map(|o| o.detected_style.clone()),
            investor_count: outcome.as_ref().map(|o| o.investor_count),
            error: lock(&running.progress.error).clone(),
        }
    }

    /// Report path from the last successful run, if any.
    pub fn last_report(&self) -> Option<String> {
        let guard = lock(&self.inner);
        guard
            .as_ref()
            .and_then(|r| lock(&r.progress.report_path).clone())
    }

    /// Aborts a running analysis.
    pub fn cancel(&self) -> UziStatus {
        let mut guard = lock(&self.inner);
        if let Some(running) = guard.as_mut() {
            running.task.abort();
            *lock(&running.progress.stage) = UziStage::Cancelled;
        }
        self.status()
    }
}

fn spawn_analysis(
    progress: Arc<Progress>,
    dir: PathBuf,
    script: PathBuf,
    ticker: String,
    _started_at_ms: u64,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let result = run_pipeline(&progress, &dir, &script, &ticker).await;
        if let Err(error) = result {
            *lock(&progress.error) = Some(format!("{error:#}"));
            *lock(&progress.stage) = UziStage::Failed;
        }
    })
}

/// The four pipeline steps, updating `progress` as each begins.
async fn run_pipeline(progress: &Progress, dir: &Path, script: &Path, ticker: &str) -> Result<()> {
    // The closure captures owned copies so each step can borrow what it needs
    // without the caller's lifetimes leaking in.
    let dir_owned = dir.to_path_buf();
    let script_owned = script.to_path_buf();
    let bridge = move |args: Vec<String>| {
        let script = script_owned.clone();
        let dir = dir_owned.clone();
        async move {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            run_bridge(&script, &dir, &refs).await
        }
    };

    // 1 · collect market data and the institutional modeling dimensions.
    bridge(vec![
        "fetch".into(),
        ticker.to_string(),
        "--depth".into(),
        "lite".into(),
    ])
    .await?;

    // 2 · quant-factor detection (fund holdings, network).
    let quant = bridge(vec!["quant".into(), ticker.to_string()])
        .await?
        .trim()
        .eq_ignore_ascii_case("true");

    // 3 · analysis in Rust, writing the three artifacts the renderer reads.
    *lock(&progress.stage) = UziStage::Analyzing;
    let cache_dir = bridge(vec!["path".into(), ticker.to_string()]).await?;
    let cache_dir = PathBuf::from(cache_dir.trim());
    if cache_dir.as_os_str().is_empty() {
        bail!("bridge could not resolve a cache directory for {ticker}");
    }
    let outcome = personal_taoli_uzi::pipeline::analyze(&cache_dir, quant, None)?;
    *lock(&progress.outcome) = Some(outcome);

    // 4 · render
    *lock(&progress.stage) = UziStage::Rendering;
    let output = bridge(vec!["render".into(), ticker.to_string()]).await?;
    let report = output
        .lines()
        .find_map(|line| line.strip_prefix("REPORT "))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .context("render stage did not report a report path")?;
    *lock(&progress.report_path) = Some(report.to_string());
    *lock(&progress.stage) = UziStage::Done;
    Ok(())
}

#[tauri::command]
pub async fn uzi_start(
    controller: tauri::State<'_, UziController>,
    ticker: String,
) -> Result<ApiResponse<UziStatus>, ()> {
    Ok(api_map(
        controller.start(ticker),
        ErrorCode::UziError,
        false,
    ))
}

#[tauri::command]
pub async fn uzi_status(
    controller: tauri::State<'_, UziController>,
) -> Result<ApiResponse<UziStatus>, ()> {
    Ok(ApiResponse::ok(controller.status()))
}

#[tauri::command]
pub async fn uzi_cancel(
    controller: tauri::State<'_, UziController>,
) -> Result<ApiResponse<UziStatus>, ()> {
    Ok(ApiResponse::ok(controller.cancel()))
}

/// Whether the feature is usable, so the UI can explain itself instead of
/// failing at the first click.
#[derive(Debug, Clone, Serialize)]
pub struct UziReady {
    pub ready: bool,
    pub skill_dir: Option<String>,
    pub python: String,
    pub bridge: Option<String>,
    pub reason: Option<String>,
}

#[tauri::command]
pub async fn uzi_ready() -> ApiResponse<UziReady> {
    let dir = skill_dir();
    let script = bridge_path();
    let (ready, reason) = match (&dir, &script) {
        (Ok(_), Ok(_)) => (true, None),
        (Err(e), _) => (false, Some(format!("{e:#}"))),
        (_, Err(e)) => (false, Some(format!("{e:#}"))),
    };
    if !ready {
        return api_fail(
            ErrorCode::UziError,
            reason.unwrap_or_else(|| "UZI-Skill is not configured".to_string()),
            false,
        );
    }
    ApiResponse::ok(UziReady {
        ready,
        skill_dir: dir.ok().map(|p| p.display().to_string()),
        python: python(),
        bridge: script.ok().map(|p| p.display().to_string()),
        reason: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_status_reports_nothing_running() {
        let controller = UziController::default();
        let status = controller.status();
        assert!(!status.running);
        assert!(status.ticker.is_none());
        assert!(status.stage.is_none());
        assert!(controller.last_report().is_none());
    }

    #[test]
    fn empty_ticker_is_rejected() {
        let controller = UziController::default();
        assert!(controller.start("   ".to_string()).is_err());
    }

    #[test]
    fn missing_skill_dir_fails_closed_with_guidance() {
        // The test process does not set TAOLI_UZI_DIR.
        let previous = std::env::var("TAOLI_UZI_DIR").ok();
        unsafe { std::env::remove_var("TAOLI_UZI_DIR") };
        let err = skill_dir().unwrap_err();
        assert!(format!("{err:#}").contains("TAOLI_UZI_DIR"));
        if let Some(value) = previous {
            unsafe { std::env::set_var("TAOLI_UZI_DIR", value) };
        }
    }

    #[test]
    fn skill_dir_rejects_a_directory_without_run_py() {
        let previous = std::env::var("TAOLI_UZI_DIR").ok();
        unsafe { std::env::set_var("TAOLI_UZI_DIR", std::env::temp_dir()) };
        let err = skill_dir().unwrap_err();
        assert!(format!("{err:#}").contains("does not look like"));
        unsafe { std::env::remove_var("TAOLI_UZI_DIR") };
        if let Some(value) = previous {
            unsafe { std::env::set_var("TAOLI_UZI_DIR", value) };
        }
    }

    #[test]
    fn python_defaults_to_python3() {
        let previous = std::env::var("TAOLI_UZI_PYTHON").ok();
        unsafe { std::env::remove_var("TAOLI_UZI_PYTHON") };
        assert_eq!(python(), "python3");
        if let Some(value) = previous {
            unsafe { std::env::set_var("TAOLI_UZI_PYTHON", value) };
        }
    }
}
