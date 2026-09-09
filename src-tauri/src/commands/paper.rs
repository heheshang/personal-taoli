use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::{
    execution::run_double_leg_smoke, order::run_order_facts_smoke, paper::run_paper_core_smoke,
};

use super::{
    dto::PaperSmokeResult,
    support::{api_fail, api_map, database_url},
};

#[tauri::command]
pub async fn run_paper_smoke(
    kind: String,
    database_url_param: Option<String>,
) -> ApiResponse<PaperSmokeResult> {
    let database_url = database_url_param.or_else(database_url);
    run_paper_smoke_with_database_url(kind, database_url).await
}

async fn run_paper_smoke_with_database_url(
    kind: String,
    database_url: Option<String>,
) -> ApiResponse<PaperSmokeResult> {
    let database_url = match database_url {
        Some(value) if !value.trim().is_empty() => value,
        Some(_) | None => {
            return api_fail(
                ErrorCode::PaperError,
                "TAOLI_DATABASE_URL is required for PAPER smoke tests; start PostgreSQL and launch the app with this environment variable set",
                false,
            );
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
            return api_fail(
                ErrorCode::InvalidRequest,
                format!("unsupported PAPER smoke kind {kind}"),
                false,
            );
        }
    };
    api_map(result, ErrorCode::PaperError, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn paper_smoke_fails_closed_without_database_url() {
        let response = run_paper_smoke_with_database_url("B01".to_owned(), None).await;

        assert!(!response.success);
        assert!(response.data.is_none());
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::PaperError)
        );
        assert!(
            response
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("TAOLI_DATABASE_URL"))
        );
    }
}
