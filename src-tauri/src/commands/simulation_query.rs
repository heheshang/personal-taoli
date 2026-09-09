use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::simulation_query::{
    SimulationOverview, SimulationRunDetail, SimulationRunsPage, get_simulation_overview,
    get_simulation_run_detail, get_simulation_runs,
};

use super::support::{api_fail, api_map, database_url};

/// G-01 模拟套利仪表盘概览：聚合 + 最近列表 + 余额曲线 + 山脊 + 活动流（只读）。
#[tauri::command]
pub async fn get_simulation_overview_command(
    database_url_param: Option<String>,
) -> ApiResponse<SimulationOverview> {
    let database_url = database_url_param.or_else(database_url);
    let database_url = match database_url {
        Some(value) if !value.trim().is_empty() => value,
        Some(_) | None => {
            return api_fail(
                ErrorCode::SimulationError,
                "TAOLI_DATABASE_URL is required for G-01 simulation dashboard; start PostgreSQL and launch the app with this environment variable set",
                false,
            );
        }
    };
    let result = get_simulation_overview(&database_url).await;
    match &result {
        Ok(overview) => tracing::info!(
            "G-01 simulation overview: runs={} success={} net_profit={}",
            overview.total_runs,
            overview.total_success,
            overview.total_net_profit
        ),
        Err(error) => tracing::error!("G-01 simulation overview failed: {error:#}"),
    }
    api_map(result, ErrorCode::SimulationError, false)
}

/// G-01 模拟套利 run 分页列表（limit ≤ 200；symbol / scenario 过滤）。
#[tauri::command]
pub async fn get_simulation_runs_command(
    database_url_param: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
    symbol: Option<String>,
    scenario: Option<String>,
) -> ApiResponse<SimulationRunsPage> {
    let database_url = database_url_param.or_else(database_url);
    let database_url = match database_url {
        Some(value) if !value.trim().is_empty() => value,
        Some(_) | None => {
            return api_fail(
                ErrorCode::SimulationError,
                "TAOLI_DATABASE_URL is required for G-01 simulation dashboard; start PostgreSQL and launch the app with this environment variable set",
                false,
            );
        }
    };
    let result = get_simulation_runs(
        &database_url,
        limit.unwrap_or(100),
        offset.unwrap_or(0),
        symbol,
        scenario,
    )
    .await;
    match &result {
        Ok(page) => tracing::info!("G-01 simulation runs page: total={}", page.total),
        Err(error) => tracing::error!("G-01 simulation runs page failed: {error:#}"),
    }
    api_map(result, ErrorCode::SimulationError, false)
}

/// G-01 单 run 详情：投影行 + report 全文 + 意图/成交/执行事件/审计/余额快照。
#[tauri::command]
pub async fn get_simulation_run_detail_command(
    database_url_param: Option<String>,
    run_id: String,
) -> ApiResponse<SimulationRunDetail> {
    let database_url = database_url_param.or_else(database_url);
    let database_url = match database_url {
        Some(value) if !value.trim().is_empty() => value,
        Some(_) | None => {
            return api_fail(
                ErrorCode::SimulationError,
                "TAOLI_DATABASE_URL is required for G-01 simulation dashboard; start PostgreSQL and launch the app with this environment variable set",
                false,
            );
        }
    };
    let result = get_simulation_run_detail(&database_url, &run_id).await;
    match &result {
        Ok(_) => tracing::info!("G-01 simulation run detail: run_id={run_id}"),
        Err(error) => tracing::error!("G-01 simulation run detail failed: {error:#}"),
    }
    api_map(result, ErrorCode::SimulationError, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulation_overview_fails_closed_without_database_url() {
        let response = get_simulation_overview_command(None).await;

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

    #[tokio::test]
    async fn simulation_runs_fails_closed_without_database_url() {
        let response = get_simulation_runs_command(None, None, None, None, None).await;

        assert!(!response.success);
        assert!(response.data.is_none());
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::SimulationError)
        );
    }

    #[tokio::test]
    async fn simulation_run_detail_fails_closed_without_database_url() {
        let response = get_simulation_run_detail_command(None, "f02-1-1".into()).await;

        assert!(!response.success);
        assert!(response.data.is_none());
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::SimulationError)
        );
    }
}
