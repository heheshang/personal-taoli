mod commands;
mod error;

use std::{
    env,
    sync::atomic::{AtomicBool, Ordering},
};

use commands::{
    SessionController, account_status, continuous_observation_status, desktop_status,
    load_app_config, load_config_summary, observe_once, replay_observations,
    run_accounting_control_smoke, run_paper_smoke, run_reconnect_smoke, save_app_config,
    start_continuous_observation, stop_continuous_observation,
};
use tauri::Manager;

static EXIT_FLUSH_STARTED: AtomicBool = AtomicBool::new(false);

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
    tauri::Builder::default()
        .manage(SessionController::default())
        .setup(|app| {
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
            start_continuous_observation,
            stop_continuous_observation,
            continuous_observation_status,
            run_reconnect_smoke,
            load_app_config,
            save_app_config,
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
