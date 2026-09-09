use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::simulation::run_simulation_smoke;

use super::{
    dto::SimulationSmokeResult,
    support::{api_fail, api_map, database_url},
};

/// F-02 模拟套利烟测（S01–S09，PostgreSQL）：合成簿驱动，无真实/测试网订单。
#[tauri::command]
pub async fn run_simulation_smoke_command(
    database_url_param: Option<String>,
) -> ApiResponse<SimulationSmokeResult> {
    let database_url = database_url_param.or_else(database_url);
    let database_url = match database_url {
        Some(value) if !value.trim().is_empty() => value,
        Some(_) | None => {
            return api_fail(
                ErrorCode::SimulationError,
                "TAOLI_DATABASE_URL is required for F-02 simulation smoke; start PostgreSQL and launch the app with this environment variable set",
                false,
            );
        }
    };
    let result = run_simulation_smoke(&database_url)
        .await
        .map(SimulationSmokeResult::from);
    match &result {
        Ok(report) => tracing::info!(
            "F-02 simulation smoke: S01-S09 all-green={} external_order_calls={}",
            [
                report.s01_full_fill_at_worst,
                report.s02_partial_fill_depth_shortfall,
                report.s03_competed_away,
                report.s04_worse_than_scan_price,
                report.s05_insufficient_funds_rejected,
                report.s06_unknown_query_recovered_no_duplicate,
                report.s07_compensation_over_budget_manual,
                report.s08_idempotent_replay,
                report.s09_fact_recovery_consistent,
            ]
            .iter()
            .all(|ok| *ok),
            report.external_order_calls
        ),
        Err(error) => tracing::error!("F-02 simulation smoke failed: {error:#}"),
    }
    api_map(result, ErrorCode::SimulationError, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulation_smoke_fails_closed_without_database_url() {
        let response = run_simulation_smoke_command(None).await;

        assert!(!response.success);
        assert!(response.data.is_none());
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::SimulationError)
        );
        assert!(
            response
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("TAOLI_DATABASE_URL"))
        );
    }
}
