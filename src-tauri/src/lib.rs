mod commands;
mod error;

use commands::{
    account_status, desktop_status, load_config_summary, observe_once, replay_observations,
    run_paper_smoke,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            desktop_status,
            load_config_summary,
            account_status,
            observe_once,
            replay_observations,
            run_paper_smoke
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
