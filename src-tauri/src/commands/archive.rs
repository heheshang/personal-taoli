use std::path::Path;

use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::archive::{ShadowReport, default_gap_path, replay_archive};

use super::support::api_error;

#[tauri::command]
pub fn replay_observations(path: String) -> ApiResponse<ShadowReport> {
    if path.trim().is_empty() {
        return ApiResponse::fail(api_error(
            ErrorCode::InvalidRequest,
            "archive path is required",
            false,
        ));
    }
    match replay_archive(Path::new(&path), default_gap_path(Path::new(&path))) {
        Ok(report) => ApiResponse::ok(report),
        Err(error) => ApiResponse::fail(api_error(ErrorCode::ArchiveError, error, false)),
    }
}
