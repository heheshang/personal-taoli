use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use personal_taoli_core::{
    account::{AccountData, load_account_data},
    archive::{default_gap_path, replay_archive},
    config::ObserverConfig,
    local_book::{BookFeed, BookFeedStatus, BookState},
    observer::{observe_continuously, observe_once},
    venues::{BinanceMarketData, BybitMarketData, MarketDataVenue},
};
use serde_json::json;
use tokio::time::timeout;

#[derive(Debug, Parser)]
#[command(
    name = "personal-taoli-observer",
    about = "Read-only cryptocurrency arbitrage observer"
)]
struct Args {
    #[arg(long, default_value = "config/observer.toml")]
    config: PathBuf,
    #[arg(long)]
    archive: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["continuous", "account_check", "replay", "reconnect_smoke"])]
    once: bool,
    #[arg(long, conflicts_with_all = ["once", "account_check", "replay", "reconnect_smoke"])]
    continuous: bool,
    #[arg(long = "account-check", conflicts_with_all = ["once", "continuous", "replay", "reconnect_smoke"])]
    account_check: bool,
    #[arg(long, conflicts_with_all = ["once", "continuous", "account_check", "reconnect_smoke"], value_name = "PATH")]
    replay: Option<PathBuf>,
    #[arg(long = "reconnect-smoke", conflicts_with_all = ["once", "continuous", "account_check", "replay"])]
    reconnect_smoke: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    output: OutputFormat,
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(path) = args.replay {
        return run_replay(&path, args.output);
    }

    let config_path = resolve_config_path(&args.config);
    let config = ObserverConfig::load(&config_path).with_context(|| {
        format!(
            "failed to load config {}; provide --config or run from the project root",
            config_path.display()
        )
    })?;
    let archive_path = args.archive.unwrap_or_else(|| config.archive.path.clone());

    if args.account_check {
        return run_account_check(&config, args.output).await;
    }
    if args.reconnect_smoke {
        return run_reconnect_smoke(&config, args.output).await;
    }
    if args.once || !args.continuous {
        eprintln!("OBSERVE mode: once; no orders");
        eprintln!(
            "ARCHIVE path={} gaps={}",
            archive_path.display(),
            default_gap_path(&archive_path).display()
        );
        let result = observe_once(&config, &archive_path).await?;
        return print_observation(&result, args.output);
    }

    eprintln!("OBSERVE mode: continuous; no orders");
    eprintln!(
        "ARCHIVE path={} gaps={}",
        archive_path.display(),
        default_gap_path(&archive_path).display()
    );
    if !args.quiet {
        eprintln!("observer started; press Ctrl-C or send SIGTERM to stop");
    }
    let quiet = args.quiet;
    observe_continuously(
        &config,
        &archive_path,
        move |report| {
            if !quiet {
                println!(
                    "{}",
                    serde_json::to_string(report).expect("scan report is serializable")
                );
            }
        },
        shutdown_signal(),
    )
    .await?;
    eprintln!("observer stopped; flushing archive");
    Ok(())
}

fn resolve_config_path(config: &Path) -> PathBuf {
    if config.is_file() {
        return config.to_owned();
    }
    if let Ok(executable) = env::current_exe() {
        for ancestor in executable.ancestors().skip(1) {
            let candidate = ancestor.join(config);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    config.to_owned()
}

async fn run_account_check(config: &ObserverConfig, output: OutputFormat) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(config.http_timeout())
        .user_agent(concat!(
            "personal-taoli-observer/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;
    let accounts = load_account_data(
        &client,
        &config.symbol,
        &config.binance,
        &config.bybit,
        config.auth_recv_window_ms,
        config.max_fee_age_ms,
    )
    .await;
    print_accounts(&accounts, output)
}

async fn run_reconnect_smoke(config: &ObserverConfig, output: OutputFormat) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(config.http_timeout())
        .user_agent(concat!(
            "personal-taoli-observer/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;
    let binance = BinanceMarketData::new(
        client.clone(),
        &config.binance.base_url,
        &config.binance.websocket_url,
    )?;
    let bybit = BybitMarketData::new(client, &config.bybit.base_url, &config.bybit.websocket_url)?;
    let stale_after = Duration::from_millis(config.max_snapshot_age_ms);
    let mut first = binance.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    let mut second = bybit.subscribe_order_book(
        &config.symbol,
        config.orderbook_depth,
        stale_after,
        config.reconnect_delay(),
    )?;
    wait_for_valid_pair(&mut first, &mut second, config.stream_start_timeout()).await?;
    let before = [first.status(), second.status()];
    first.force_reconnect().await?;
    second.force_reconnect().await?;
    timeout(config.stream_start_timeout(), async {
        loop {
            let current = [first.status(), second.status()];
            if current
                .iter()
                .zip(&before)
                .all(|(now, old)| now.state == BookState::Valid && now.generation > old.generation)
            {
                return Ok::<_, anyhow::Error>(current);
            }
            tokio::select! {
                _ = first.changed() => {}
                _ = second.changed() => {}
            }
        }
    })
    .await
    .context("timed out waiting for both feeds to recover after reconnect")??;
    let result = json!({
        "success": true,
        "binance": feed_summary(&first.status()),
        "bybit": feed_summary(&second.status()),
        "no_orders": true,
    });
    print_value(&result, output)
}

async fn wait_for_valid_pair(
    first: &mut BookFeed,
    second: &mut BookFeed,
    start_timeout: Duration,
) -> Result<()> {
    timeout(start_timeout, async {
        loop {
            if first.status().state == BookState::Valid && second.status().state == BookState::Valid
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::select! {
                status = first.changed() => { status?; }
                status = second.changed() => { status?; }
            }
        }
    })
    .await
    .context("timed out waiting for both live order books")??;
    Ok(())
}

fn run_replay(path: &Path, output: OutputFormat) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect archive {}", path.display()))?;
    if !metadata.is_file() {
        bail!("archive path is not a file: {}", path.display());
    }
    let report = replay_archive(path, default_gap_path(path))?;
    print_value(&report, output)
}

fn print_accounts(accounts: &[AccountData; 2], output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(accounts)?),
        OutputFormat::Text => {
            for account in accounts {
                let rejections = if account.rejection_reasons.is_empty() {
                    "none".to_owned()
                } else {
                    account.rejection_reasons.join("; ")
                };
                println!(
                    "{} fee_source={} buy_taker_rate={} sell_taker_rate={} read_only={:?} eligible={} rejections={}",
                    account.venue,
                    account.fee.source,
                    account.fee.buy_taker_rate,
                    account.fee.sell_taker_rate,
                    account.permission.read_only,
                    account.rejection_reasons.is_empty(),
                    rejections,
                );
            }
        }
    }
    Ok(())
}

fn print_observation(
    result: &personal_taoli_core::observer::ObservationRun,
    output: OutputFormat,
) -> Result<()> {
    let value = json!({
        "report": result.report,
        "accounts": result.accounts,
        "feeds": result.feeds.iter().map(feed_summary).collect::<Vec<_>>(),
        "archived": result.archived,
        "no_orders": true,
    });
    print_value(&value, output)
}

fn print_value<T: serde::Serialize>(value: &T, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
        OutputFormat::Text => println!("{}", serde_json::to_string(value)?),
    }
    Ok(())
}

fn feed_summary(status: &BookFeedStatus) -> serde_json::Value {
    json!({
        "venue": status.venue,
        "symbol": status.symbol,
        "state": status.state,
        "generation": status.generation,
        "reconnects": status.reconnects,
        "applied_updates": status.applied_updates,
        "reason": status.reason,
    })
}

#[cfg(unix)]
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl-C handler");
}
