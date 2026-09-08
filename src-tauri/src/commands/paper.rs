use std::env;

use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::{
    execution::run_double_leg_smoke, order::run_order_facts_smoke, paper::run_paper_core_smoke,
};

use super::{dto::PaperSmokeResult, support::api_error};

#[tauri::command]
pub async fn run_paper_smoke(kind: String) -> ApiResponse<PaperSmokeResult> {
    let database_url = match env::var("TAOLI_DATABASE_URL") {
        Ok(value) => value,
        Err(_) => {
            return ApiResponse::fail(api_error(
                ErrorCode::PaperError,
                "TAOLI_DATABASE_URL is required for PAPER smoke tests",
                false,
            ));
        }
    };
    let result = match kind.as_str() {
        "B01" => run_paper_core_smoke(&database_url)
            .await
            .map(PaperSmokeResult::B01),
        "B02" => run_order_facts_smoke(&database_url)
            .await
            .map(PaperSmokeResult::B02),
        "B03" => run_double_leg_smoke(&database_url)
            .await
            .map(PaperSmokeResult::B03),
        _ => {
            return ApiResponse::fail(api_error(
                ErrorCode::InvalidRequest,
                format!("unsupported PAPER smoke kind {kind}"),
                false,
            ));
        }
    };
    match result {
        Ok(report) => ApiResponse::ok(report),
        Err(error) => ApiResponse::fail(api_error(ErrorCode::PaperError, error, false)),
    }
}
