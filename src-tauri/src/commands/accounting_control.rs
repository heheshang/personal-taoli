use std::env;

use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::{
    accounting::run_accounting_smoke, control::run_control_smoke,
    reconciliation::run_reconciliation_smoke,
};

use super::{dto::AccountingControlSmokeResult, support::api_error};

#[tauri::command]
pub async fn run_accounting_control_smoke(
    kind: String,
) -> ApiResponse<AccountingControlSmokeResult> {
    let Some(database_url) = env::var("TAOLI_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return ApiResponse::fail(api_error(
            ErrorCode::AccountingControlError,
            "TAOLI_DATABASE_URL is required for accounting, reconciliation, and control smoke tests; start PostgreSQL and launch the app with this environment variable set",
            false,
        ));
    };

    run_accounting_control_smoke_with_database_url(kind, Some(database_url)).await
}

async fn run_accounting_control_smoke_with_database_url(
    kind: String,
    database_url: Option<String>,
) -> ApiResponse<AccountingControlSmokeResult> {
    let Some(database_url) = database_url.filter(|value| !value.trim().is_empty()) else {
        return ApiResponse::fail(api_error(
            ErrorCode::AccountingControlError,
            "TAOLI_DATABASE_URL is required",
            false,
        ));
    };

    let result = match kind.as_str() {
        "ACCOUNTING" => run_accounting_smoke(&database_url)
            .await
            .map(AccountingControlSmokeResult::Accounting),
        "RECONCILIATION" => run_reconciliation_smoke(&database_url)
            .await
            .map(AccountingControlSmokeResult::Reconciliation),
        "CONTROL" => run_control_smoke(&database_url)
            .await
            .map(AccountingControlSmokeResult::Control),
        _ => {
            return ApiResponse::fail(api_error(
                ErrorCode::InvalidRequest,
                format!("unsupported accounting control smoke kind {kind}"),
                false,
            ));
        }
    };

    result.map_or_else(
        |error| ApiResponse::fail(api_error(ErrorCode::AccountingControlError, error, false)),
        ApiResponse::ok,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accounting_control_smoke_fails_closed_without_database_url() {
        let response =
            run_accounting_control_smoke_with_database_url("ACCOUNTING".to_owned(), None).await;

        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::AccountingControlError)
        );
    }
}
