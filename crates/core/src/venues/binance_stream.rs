use std::{collections::VecDeque, time::Duration};

use anyhow::{Context, Result, bail};
use futures_util::{Sink, SinkExt, Stream, StreamExt, pin_mut};
use reqwest::{Client, Url};
use serde::Deserialize;
use tokio::{sync::mpsc, time::sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    local_book::{BookFeed, FeedCommand, FeedPublisher, LocalOrderBook, StaleFeed},
    market::{OrderBookSnapshot, parse_updates, unix_timestamp_ms},
};

use super::binance::fetch_depth_snapshot;

#[derive(Debug, Deserialize)]
struct DepthEvent {
    #[serde(rename = "E")]
    event_time: u64,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "U")]
    first_update_id: u64,
    #[serde(rename = "u")]
    final_update_id: u64,
    #[serde(rename = "b")]
    bids: Vec<[String; 2]>,
    #[serde(rename = "a")]
    asks: Vec<[String; 2]>,
}

struct ConnectionConfig<'a> {
    client: &'a Client,
    rest_base_url: &'a Url,
    websocket_url: &'a Url,
    symbol: &'a str,
    depth: u16,
    stale_after: Duration,
}

pub(super) fn subscribe(
    client: Client,
    rest_base_url: Url,
    websocket_url: Url,
    symbol: String,
    depth: u16,
    stale_after: Duration,
    reconnect_delay: Duration,
) -> BookFeed {
    let (mut publisher, receiver) = FeedPublisher::channel("binance", &symbol);
    let (commands, mut command_receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        let mut reconnect = false;
        loop {
            publisher.syncing(reconnect);
            let result = run_connection(
                ConnectionConfig {
                    client: &client,
                    rest_base_url: &rest_base_url,
                    websocket_url: &websocket_url,
                    symbol: &symbol,
                    depth,
                    stale_after,
                },
                &mut command_receiver,
                &mut publisher,
            )
            .await;
            match result {
                Ok(()) => publisher.invalid("Binance stream ended"),
                Err(error) if error.downcast_ref::<StaleFeed>().is_some() => {
                    publisher.stale(format!("Binance stream stale: {error:#}"));
                }
                Err(error) => publisher.invalid(format!("Binance stream invalid: {error:#}")),
            }
            reconnect = true;
            tokio::select! {
                _ = sleep(reconnect_delay) => {}
                command = command_receiver.recv() => {
                    if command.is_none() {
                        break;
                    }
                }
            }
        }
    });
    BookFeed::new(receiver, commands)
}

async fn run_connection(
    config: ConnectionConfig<'_>,
    commands: &mut mpsc::Receiver<FeedCommand>,
    publisher: &mut FeedPublisher,
) -> Result<()> {
    let url = config.websocket_url.join(&format!(
        "ws/{}@depth@100ms",
        config.symbol.to_ascii_lowercase()
    ))?;
    let (stream, _) = connect_async(url.as_str())
        .await
        .context("Binance WebSocket connection failed")?;
    let (mut sink, mut source) = stream.split();
    let mut buffered = VecDeque::new();

    while buffered.is_empty() {
        buffered.push_back(
            next_event(
                &mut sink,
                &mut source,
                config.stale_after,
                commands,
                config.symbol,
            )
            .await?
            .context("Binance stream closed before first depth event")?,
        );
    }

    let snapshot_future =
        fetch_depth_snapshot(config.client, config.rest_base_url, config.symbol, 1000);
    pin_mut!(snapshot_future);
    let snapshot = loop {
        tokio::select! {
            snapshot = &mut snapshot_future => break snapshot?,
            event = next_event(&mut sink, &mut source, config.stale_after, commands, config.symbol) => {
                buffered.push_back(event?.context("Binance stream closed during snapshot fetch")?);
            }
        }
    };
    while buffered
        .back()
        .is_none_or(|event| event.final_update_id <= snapshot.sequence)
    {
        buffered.push_back(
            next_event(
                &mut sink,
                &mut source,
                config.stale_after,
                commands,
                config.symbol,
            )
            .await?
            .context("Binance stream closed before snapshot successor")?,
        );
    }
    let mut book = bridge_snapshot(snapshot, &mut buffered, config.symbol)?;
    publisher.valid(book.snapshot(config.depth.into())?);

    loop {
        let event = next_event(
            &mut sink,
            &mut source,
            config.stale_after,
            commands,
            config.symbol,
        )
        .await?
        .context("Binance stream closed")?;
        if event.final_update_id <= book.sequence() {
            continue;
        }
        if event.first_update_id > book.sequence().saturating_add(1) {
            bail!(
                "depth sequence gap: local={}, next=[{},{}]",
                book.sequence(),
                event.first_update_id,
                event.final_update_id
            );
        }
        apply_event(&mut book, event)?;
        publisher.valid(book.snapshot(config.depth.into())?);
    }
}

fn bridge_snapshot(
    snapshot: OrderBookSnapshot,
    buffered: &mut VecDeque<DepthEvent>,
    symbol: &str,
) -> Result<LocalOrderBook> {
    let mut book = LocalOrderBook::from_snapshot(snapshot)?;
    while buffered
        .front()
        .is_some_and(|event| event.final_update_id <= book.sequence())
    {
        buffered.pop_front();
    }
    let first = buffered
        .front()
        .context("Binance snapshot covered every buffered event")?;
    let expected = book.sequence().saturating_add(1);
    if first.first_update_id > expected || first.final_update_id < expected {
        bail!(
            "snapshot bridge failed: snapshot={}, event=[{},{}]",
            book.sequence(),
            first.first_update_id,
            first.final_update_id
        );
    }
    while let Some(event) = buffered.pop_front() {
        if event.final_update_id <= book.sequence() {
            continue;
        }
        if event.first_update_id > book.sequence().saturating_add(1) {
            bail!("buffered depth sequence gap");
        }
        apply_event(&mut book, event)?;
    }
    if book.snapshot(1)?.symbol != symbol {
        bail!("Binance snapshot symbol mismatch");
    }
    Ok(book)
}

fn apply_event(book: &mut LocalOrderBook, event: DepthEvent) -> Result<()> {
    let received = unix_timestamp_ms()?;
    book.apply(
        &parse_updates(event.bids).context("invalid Binance bid update")?,
        &parse_updates(event.asks).context("invalid Binance ask update")?,
        event.final_update_id,
        Some(event.event_time),
        received,
    )
}

async fn next_event<S, R>(
    sink: &mut S,
    source: &mut R,
    stale_after: Duration,
    commands: &mut mpsc::Receiver<FeedCommand>,
    expected_symbol: &str,
) -> Result<Option<DepthEvent>>
where
    S: Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
    R: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let stale_deadline = tokio::time::sleep(stale_after);
    tokio::pin!(stale_deadline);
    loop {
        let message = tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(FeedCommand::Reconnect) => bail!("forced reconnect"),
                    None => return Ok(None),
                }
            }
            _ = &mut stale_deadline => {
                return Err(StaleFeed("Binance order book received no depth event").into());
            }
            message = source.next() => message,
        };
        match message {
            Some(Ok(Message::Text(text))) => {
                let event: DepthEvent =
                    serde_json::from_str(text.as_ref()).context("invalid Binance depth event")?;
                if event.symbol != expected_symbol {
                    bail!(
                        "Binance stream returned symbol {}, expected {expected_symbol}",
                        event.symbol
                    );
                }
                return Ok(Some(event));
            }
            Some(Ok(Message::Ping(payload))) => sink.send(Message::Pong(payload)).await?,
            Some(Ok(Message::Close(_))) | None => return Ok(None),
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use crate::market::Level;

    use super::*;

    fn snapshot(sequence: u64) -> OrderBookSnapshot {
        OrderBookSnapshot {
            venue: "binance".into(),
            symbol: "BTCUSDT".into(),
            bids: vec![Level {
                price: Decimal::from(99),
                quantity: Decimal::ONE,
            }],
            asks: vec![Level {
                price: Decimal::from(101),
                quantity: Decimal::ONE,
            }],
            sequence,
            source_timestamp_ms: None,
            received_timestamp_ms: 1,
        }
    }

    fn event(first: u64, final_id: u64) -> DepthEvent {
        DepthEvent {
            event_time: 2,
            symbol: "BTCUSDT".into(),
            first_update_id: first,
            final_update_id: final_id,
            bids: vec![],
            asks: vec![],
        }
    }

    #[test]
    fn bridge_accepts_event_covering_snapshot_successor() {
        let mut events = VecDeque::from([event(9, 10), event(10, 12)]);
        let book = bridge_snapshot(snapshot(10), &mut events, "BTCUSDT").unwrap();
        assert_eq!(book.sequence(), 12);
    }

    #[test]
    fn bridge_rejects_sequence_gap() {
        let mut events = VecDeque::from([event(12, 13)]);
        assert!(bridge_snapshot(snapshot(10), &mut events, "BTCUSDT").is_err());
    }
}
