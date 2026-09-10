use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;
use tokio::{sync::mpsc, time::sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    local_book::{BookFeed, FeedCommand, FeedPublisher, LocalOrderBook, StaleFeed},
    market::{OrderBookSnapshot, parse_levels, parse_updates, unix_timestamp_ms},
};

#[derive(Debug, Deserialize)]
struct OrderBookEvent {
    topic: String,
    #[serde(rename = "type")]
    message_type: EventType,
    ts: u64,
    data: OrderBookData,
    cts: Option<u64>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum EventType {
    Snapshot,
    Delta,
}

#[derive(Debug, Deserialize)]
struct OrderBookData {
    s: String,
    b: Vec<[String; 2]>,
    a: Vec<[String; 2]>,
    u: u64,
    seq: u64,
}

pub(super) fn subscribe(
    websocket_url: Url,
    symbol: String,
    depth: u16,
    stale_after: Duration,
    reconnect_delay: Duration,
) -> BookFeed {
    let (mut publisher, receiver) = FeedPublisher::channel("bybit", &symbol);
    let (commands, mut command_receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        let mut reconnect = false;
        loop {
            publisher.syncing(reconnect);
            let result = run_connection(
                &websocket_url,
                &symbol,
                depth,
                stale_after,
                &mut command_receiver,
                &mut publisher,
            )
            .await;
            match result {
                Ok(()) => publisher.invalid("Bybit stream ended"),
                Err(error) if error.downcast_ref::<StaleFeed>().is_some() => {
                    publisher.stale(format!("Bybit stream stale: {error:#}"));
                }
                Err(error) => publisher.invalid(format!("Bybit stream invalid: {error:#}")),
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
    websocket_url: &Url,
    symbol: &str,
    depth: u16,
    stale_after: Duration,
    commands: &mut mpsc::Receiver<FeedCommand>,
    publisher: &mut FeedPublisher,
) -> Result<()> {
    let (stream, _) = connect_async(websocket_url.as_str())
        .await
        .context("Bybit WebSocket connection failed")?;
    let (mut sink, mut source) = stream.split();
    let topic = format!("orderbook.{depth}.{symbol}");
    sink.send(Message::Text(
        json!({"op":"subscribe","args":[topic]}).to_string().into(),
    ))
    .await?;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.tick().await;
    let mut book: Option<LocalOrderBook> = None;
    let mut last_cross_sequence: Option<u64> = None;
    let stale_deadline = tokio::time::sleep(stale_after);
    tokio::pin!(stale_deadline);

    loop {
        let message = tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(FeedCommand::Reconnect) => bail!("forced reconnect"),
                    None => return Ok(()),
                }
            }
            _ = heartbeat.tick() => {
                sink.send(Message::Text(json!({"op":"ping"}).to_string().into())).await?;
                continue;
            }
            _ = &mut stale_deadline => {
                return Err(StaleFeed("Bybit order book received no snapshot or delta").into());
            }
            message = source.next() => message,
        };
        match message {
            Some(Ok(Message::Text(text))) => {
                let value: serde_json::Value = serde_json::from_str(text.as_ref())?;
                if value.get("topic").is_none() {
                    if value.get("success") == Some(&serde_json::Value::Bool(false)) {
                        bail!("Bybit control message rejected: {value}");
                    }
                    continue;
                }
                let event: OrderBookEvent =
                    serde_json::from_value(value).context("invalid Bybit order book event")?;
                if event.topic != topic || event.data.s != symbol {
                    bail!("Bybit stream topic or symbol mismatch");
                }
                if !advances_cross_sequence(last_cross_sequence, event.data.seq) {
                    continue;
                }
                match event.message_type {
                    EventType::Snapshot => {
                        let snapshot = snapshot_from_event(event)?;
                        last_cross_sequence = Some(snapshot.0);
                        book = Some(LocalOrderBook::from_snapshot(snapshot.1)?);
                    }
                    EventType::Delta => {
                        last_cross_sequence.context("Bybit delta arrived before snapshot")?;
                        let local = book
                            .as_mut()
                            .context("Bybit delta arrived before snapshot")?;
                        local.apply(
                            &parse_updates(event.data.b).context("invalid Bybit bid update")?,
                            &parse_updates(event.data.a).context("invalid Bybit ask update")?,
                            event.data.u,
                            event.cts.or(Some(event.ts)),
                            unix_timestamp_ms()?,
                        )?;
                        last_cross_sequence = Some(event.data.seq);
                    }
                }
                publisher.valid(
                    book.as_ref()
                        .expect("book initialized")
                        .snapshot(depth.into())?,
                );
                stale_deadline
                    .as_mut()
                    .reset(tokio::time::Instant::now() + stale_after);
            }
            Some(Ok(Message::Ping(payload))) => sink.send(Message::Pong(payload)).await?,
            Some(Ok(Message::Close(_))) | None => return Ok(()),
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error.into()),
        }
    }
}

fn advances_cross_sequence(previous: Option<u64>, next: u64) -> bool {
    previous.is_none_or(|previous| next > previous)
}

fn snapshot_from_event(event: OrderBookEvent) -> Result<(u64, OrderBookSnapshot)> {
    let received_timestamp_ms = unix_timestamp_ms()?;
    let snapshot = OrderBookSnapshot {
        venue: "bybit".into(),
        symbol: event.data.s,
        bids: parse_levels(event.data.b).context("invalid Bybit snapshot bid")?,
        asks: parse_levels(event.data.a).context("invalid Bybit snapshot ask")?,
        sequence: event.data.u,
        source_timestamp_ms: event.cts.or(Some(event.ts)),
        received_timestamp_ms,
    };
    snapshot.validate()?;
    Ok((event.data.seq, snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_uses_matching_engine_time_and_update_id() {
        let event: OrderBookEvent = serde_json::from_str(
            r#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":10,"data":{"s":"BTCUSDT","b":[["99","1"]],"a":[["101","2"]],"u":8,"seq":20},"cts":9}"#,
        )
        .unwrap();
        let (cross_sequence, snapshot) = snapshot_from_event(event).unwrap();
        assert_eq!(cross_sequence, 20);
        assert_eq!(snapshot.sequence, 8);
        assert_eq!(snapshot.source_timestamp_ms, Some(9));
    }

    #[test]
    fn restart_snapshot_can_replace_higher_update_id() {
        let initial: OrderBookEvent = serde_json::from_str(
            r#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":10,"data":{"s":"BTCUSDT","b":[["99","1"]],"a":[["101","2"]],"u":100,"seq":200}}"#,
        )
        .unwrap();
        let restart: OrderBookEvent = serde_json::from_str(
            r#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":11,"data":{"s":"BTCUSDT","b":[["98","1"]],"a":[["102","2"]],"u":1,"seq":201}}"#,
        )
        .unwrap();
        let (_, first) = snapshot_from_event(initial).unwrap();
        let (cross_sequence, second) = snapshot_from_event(restart).unwrap();
        assert_eq!(first.sequence, 100);
        assert_eq!(cross_sequence, 201);
        assert_eq!(second.sequence, 1);
    }

    #[test]
    fn stale_cross_sequence_is_rejected() {
        assert!(advances_cross_sequence(Some(200), 201));
        assert!(!advances_cross_sequence(Some(200), 200));
        assert!(!advances_cross_sequence(Some(200), 199));
    }
}
