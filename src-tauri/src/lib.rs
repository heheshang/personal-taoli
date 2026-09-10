mod commands;
mod error;

use std::{
    env,
    sync::atomic::{AtomicBool, Ordering},
};

use commands::{
    SessionController, UziController, account_status, continuous_observation_status,
    desktop_status, get_observer_config, get_simulation_overview_command,
    get_simulation_run_detail_command, get_simulation_runs_command, load_app_config,
    load_app_config_file, load_config_summary, observe_once, replay_observations,
    run_accounting_control_smoke, run_paper_smoke, run_reconnect_smoke,
    run_simulation_smoke_command, save_app_config, save_observer_config,
    start_continuous_observation, stop_continuous_observation, uzi_cancel, uzi_ready, uzi_start,
    uzi_status,
};
use tauri::Manager;

static EXIT_FLUSH_STARTED: AtomicBool = AtomicBool::new(false);

/// Custom scheme serving UZI reports.
///
/// Reports are single self-contained HTML documents, so they are framed rather
/// than inlined. Serving them through a scheme (instead of loosening the app's
/// CSP to admit the `asset:` origin) keeps `default-src 'self'` intact and lets
/// each document carry its own, tighter policy: the report's inline tooltip
/// script needs `unsafe-inline`, which the app itself must never allow.
///
/// The path is taken as an absolute filesystem path; that is only reachable
/// from the app's own commands, which is why arbitrary traversal is not a
/// concern here — the URL is never built from untrusted input.
const UZI_REPORT_SCHEME: &str = "taoli-uzi";

fn uzi_report_response(request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode, header};

    // Percent-decode the path (`file://`-style escapes may be present).
    let raw = request.uri().path().trim_start_matches('/');
    let mut decoded = String::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = bytes.next();
            let lo = bytes.next();
            if let (Some(hi), Some(lo)) = (hi, lo)
                && let Ok(hex) = u8::from_str_radix(&format!("{}{}", hi as char, lo as char), 16)
            {
                decoded.push(hex as char);
                continue;
            }
            decoded.push('%');
            continue;
        }
        decoded.push(byte as char);
    }

    let body = match std::fs::read(&decoded) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(html) => personal_taoli_uzi::report::sanitize(&html).into_bytes(),
            Err(error) => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(format!("report is not valid UTF-8: {error}").into_bytes())
                    .expect("static response");
            }
        },
        Err(error) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(format!("report not readable: {error}").into_bytes())
                .expect("static response");
        }
    };

    // Covers the document's own requests: no remote origins, and the inline
    // tooltip script is the only script allowed to run.
    let policy = "default-src 'none';                   img-src data: 'self';                   style-src 'unsafe-inline';                   script-src 'unsafe-inline';                   font-src 'self';                   connect-src 'none';                   form-action 'none';                   base-uri 'none'";

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header("Content-Security-Policy", policy)
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .expect("static response")
}

/// Absolute path of the last report, as a `taoli-uzi://` URL the webview can load.
#[tauri::command]
async fn uzi_report_url(controller: tauri::State<'_, UziController>) -> Result<String, String> {
    let path = controller
        .last_report()
        .ok_or_else(|| "no report has been produced yet".to_string())?;
    // A leading slash makes the path parse as the URI's authority-host position
    // rather than an opaque string; the handler trims it back off.
    Ok(format!("{UZI_REPORT_SCHEME}:///{path}"))
}

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
        .manage(UziController::default())
        .register_uri_scheme_protocol(UZI_REPORT_SCHEME, |_app, request| {
            uzi_report_response(&request)
        })
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
            uzi_ready,
            uzi_start,
            uzi_status,
            uzi_cancel,
            uzi_report_url,
            load_app_config,
            save_app_config,
            get_observer_config,
            save_observer_config,
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
