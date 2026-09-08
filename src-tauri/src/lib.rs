mod commands;
mod error;

use std::{env, path::PathBuf};

use commands::{
    account_status, desktop_status, load_config_summary, observe_once, replay_observations,
    run_paper_smoke,
};

fn load_project_env() {
    let relative = PathBuf::from(".env");
    let mut candidates = vec![relative.clone()];
    if let Ok(executable) = env::current_exe() {
        candidates.extend(
            executable
                .ancestors()
                .skip(1)
                .map(|ancestor| ancestor.join(&relative)),
        );
    }

    let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
        return;
    };
    if let Err(error) = dotenvy::from_path(&path) {
        eprintln!(
            "failed to load environment file {}: {error}",
            path.display()
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_project_env();
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
