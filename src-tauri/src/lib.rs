mod commands;
mod error;

use std::{
    env,
    sync::atomic::{AtomicBool, Ordering},
};

use commands::{
    AgentController, SessionController, account_status, agent_access_levels, agent_ask,
    agent_decide, agent_default_prompt, agent_ready, agent_set_access_level, agent_start,
    agent_status, agent_stop, continuous_observation_status, desktop_status, get_observer_config,
    get_simulation_overview_command, get_simulation_run_detail_command,
    get_simulation_runs_command, load_app_config, load_app_config_file, load_config_summary,
    observe_once, replay_observations, run_accounting_control_smoke, run_paper_smoke,
    run_reconnect_smoke, run_simulation_smoke_command, save_app_config, save_observer_config,
    start_continuous_observation, stop_continuous_observation,
};
use tauri::Manager;

static EXIT_FLUSH_STARTED: AtomicBool = AtomicBool::new(false);

/// Loads the saved app config (JSON) at startup and injects non-empty values
/// into the `TAOLI_*` environment variables before any command or observer
/// task runs. Empty saved values are ignored so shell-provided environment
/// variables keep working. Runs in `setup`, single-threaded, before any
/// command can execute, so the `set_var` calls do not race with readers.
fn load_saved_env_config() {
    let config = load_app_config_file();
    let mut loaded = Vec::new();
    for (name, value) in [
        ("TAOLI_DATABASE_URL", config.database_url.as_str()),
        ("TAOLI_BINANCE_API_KEY", config.binance_api_key.as_str()),
        (
            "TAOLI_BINANCE_API_SECRET",
            config.binance_api_secret.as_str(),
        ),
        ("TAOLI_BYBIT_API_KEY", config.bybit_api_key.as_str()),
        ("TAOLI_BYBIT_API_SECRET", config.bybit_api_secret.as_str()),
    ] {
        if !value.is_empty() {
            // SAFETY: this runs in `setup` before any command or spawned
            // observer task reads these variables; no concurrent env access.
            unsafe {
                env::set_var(name, value);
            }
            loaded.push(name);
        }
    }
    if !loaded.is_empty() {
        eprintln!("injected saved config into env: {}", loaded.join(", "));
    }
}

/// Interprets `TAOLI_OBSERVER_AUTOSTART` (any non-empty value) as a request to
/// start continuous observation before the window is shown. The archive path
/// comes from `TAOLI_OBSERVER_ARCHIVE` when set, else from the config. A dead
/// observer loop exits the app so an external supervisor can restart it.
fn autostart_continuous(app: &tauri::App) {
    let autostart = env::var("TAOLI_OBSERVER_AUTOSTART")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    if !autostart {
        return;
    }
    let archive = env::var("TAOLI_OBSERVER_ARCHIVE")
        .ok()
        .filter(|value| !value.trim().is_empty());
    eprintln!("continuous observation autostart requested");
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let controller = handle.state::<SessionController>();
        match controller.start(None, archive, Some(handle.clone())).await {
            Ok(status) => {
                let archive = status.archive_path.unwrap_or_default();
                eprintln!("continuous observation running (read-only) archive={archive}");
            }
            Err(error) => {
                eprintln!("continuous observation autostart failed: {error:#}");
                handle.exit(1);
            }
        }
    });
}

#[cfg(unix)]
fn install_term_signal_handler(app: &tauri::AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
        terminate.recv().await;
        handle.exit(0);
    });
}

#[cfg(not(unix))]
fn install_term_signal_handler(_app: &tauri::AppHandle) {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 日志框架：RUST_LOG 可覆写（默认 info），F-02 节点日志经 tracing 输出。
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    tauri::Builder::default()
        .manage(SessionController::default())
        .manage(AgentController::default())
        .setup(|app| {
            load_saved_env_config();
            install_term_signal_handler(app.handle());
            autostart_continuous(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop_status,
            load_config_summary,
            account_status,
            observe_once,
            replay_observations,
            run_paper_smoke,
            run_accounting_control_smoke,
            run_simulation_smoke_command,
            get_simulation_overview_command,
            get_simulation_runs_command,
            get_simulation_run_detail_command,
            start_continuous_observation,
            stop_continuous_observation,
            continuous_observation_status,
            run_reconnect_smoke,
            load_app_config,
            save_app_config,
            get_observer_config,
            save_observer_config,
            agent_ready,
            agent_start,
            agent_ask,
            agent_decide,
            agent_status,
            agent_stop,
            agent_default_prompt,
            agent_access_levels,
            agent_set_access_level,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if EXIT_FLUSH_STARTED.swap(true, Ordering::SeqCst) {
                    return;
                }
                let exit_code = code.unwrap_or(0);
                api.prevent_exit();
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let controller = handle.state::<SessionController>();
                    match controller.stop(std::time::Duration::from_secs(30)).await {
                        Ok(_) | Err(_) => {}
                    }
                    handle.exit(exit_code);
                });
            }
        });
}
