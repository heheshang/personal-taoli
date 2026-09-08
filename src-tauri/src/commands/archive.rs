use std::{fs, path::Path};

use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::archive::{ShadowReport, default_gap_path, replay_archive};

use super::support::api_error;

#[tauri::command]
pub fn replay_observations(path: String) -> ApiResponse<ShadowReport> {
    let path = path.trim();
    if path.is_empty() {
        return ApiResponse::fail(api_error(
            ErrorCode::InvalidRequest,
            "archive path is required",
            false,
        ));
    }

    let archive_path = Path::new(path);
    match fs::metadata(archive_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return ApiResponse::fail(api_error(
                ErrorCode::ArchiveError,
                format!("archive path is not a file: {}", archive_path.display()),
                false,
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ApiResponse::fail(api_error(
                ErrorCode::ArchiveNotFound,
                format!(
                    "archive file not found: {}; run OBSERVE ONCE first or select an existing archive",
                    archive_path.display()
                ),
                false,
            ));
        }
        Err(error) => {
            return ApiResponse::fail(api_error(
                ErrorCode::ArchiveError,
                format!(
                    "failed to inspect archive {}: {error}",
                    archive_path.display()
                ),
                false,
            ));
        }
    }

    match replay_archive(archive_path, default_gap_path(archive_path)) {
        Ok(report) => ApiResponse::ok(report),
        Err(error) => ApiResponse::fail(api_error(ErrorCode::ArchiveError, error, false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_reports_missing_archive_before_opening_gap_journal() {
        let path = std::env::temp_dir().join(format!(
            "personal-taoli-missing-archive-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_file(&path);

        let response = replay_observations(path.to_string_lossy().into_owned());

        assert!(!response.success);
        assert_eq!(response.data, None);
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::ArchiveNotFound)
        );
        assert!(
            response
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("run OBSERVE ONCE first"))
        );
    }
}
