use std::path::PathBuf;

use crate::error::{ApiResponse, ErrorCode};
use personal_taoli_core::observer::observe_once as run_observation;

use super::{
    dto::{FeedSummary, ObserveResult},
    support::{api_map, load_config},
};

#[tauri::command]
pub async fn observe_once(
    config_path: Option<String>,
    archive_path: Option<String>,
) -> ApiResponse<Vec<ObserveResult>> {
    let result = async {
        let (_, config) = load_config(config_path)?;
        let archive_path = archive_path
            .map(PathBuf::from)
            .unwrap_or_else(|| config.archive.path.clone());
        let results = run_observation(&config, &archive_path).await?;
        Ok::<_, anyhow::Error>(
            results
                .into_iter()
                .map(|result| ObserveResult {
                    report: result.report,
                    accounts: result.accounts,
                    feeds: [
                        FeedSummary::from(&result.feeds[0]),
                        FeedSummary::from(&result.feeds[1]),
                    ],
                    archived: result.archived,
                })
                .collect(),
        )
    }
    .await;
    api_map(result, ErrorCode::MarketDataError, true)
}
