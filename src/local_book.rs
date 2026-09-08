use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use anyhow::{Result, bail};
use rust_decimal::Decimal;
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use crate::market::{Level, OrderBookSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BookState {
    Syncing,
    Valid,
    Stale,
    Invalid,
}

#[derive(Debug, Clone)]
pub struct BookFeedStatus {
    pub venue: String,
    pub symbol: String,
    pub state: BookState,
    pub generation: u64,
    pub reconnects: u64,
    pub applied_updates: u64,
    pub snapshot: Option<Arc<OrderBookSnapshot>>,
    pub reason: Option<String>,
}

#[derive(Debug)]
pub(crate) enum FeedCommand {
    Reconnect,
}
#[derive(Debug)]
pub(crate) struct StaleFeed(pub(crate) &'static str);

impl fmt::Display for StaleFeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for StaleFeed {}

pub struct BookFeed {
    receiver: watch::Receiver<BookFeedStatus>,
    commands: mpsc::Sender<FeedCommand>,
}

impl BookFeed {
    pub(crate) fn new(
        receiver: watch::Receiver<BookFeedStatus>,
        commands: mpsc::Sender<FeedCommand>,
    ) -> Self {
        Self { receiver, commands }
    }

    pub fn status(&self) -> BookFeedStatus {
        self.receiver.borrow().clone()
    }

    pub async fn changed(&mut self) -> Result<BookFeedStatus> {
        self.receiver.changed().await?;
        Ok(self.status())
    }

    pub async fn force_reconnect(&self) -> Result<()> {
        self.commands.send(FeedCommand::Reconnect).await?;
        Ok(())
    }
}

pub(crate) struct FeedPublisher {
    sender: watch::Sender<BookFeedStatus>,
    status: BookFeedStatus,
}

impl FeedPublisher {
    pub(crate) fn channel(venue: &str, symbol: &str) -> (Self, watch::Receiver<BookFeedStatus>) {
        let status = BookFeedStatus {
            venue: venue.to_owned(),
            symbol: symbol.to_owned(),
            state: BookState::Syncing,
            generation: 0,
            reconnects: 0,
            applied_updates: 0,
            snapshot: None,
            reason: None,
        };
        let (sender, receiver) = watch::channel(status.clone());
        (Self { sender, status }, receiver)
    }

    pub(crate) fn syncing(&mut self, reconnect: bool) {
        self.status.generation += 1;
        self.status.reconnects += u64::from(reconnect);
        self.status.state = BookState::Syncing;
        self.status.snapshot = None;
        self.status.reason = None;
        self.publish();
    }

    pub(crate) fn valid(&mut self, snapshot: OrderBookSnapshot) {
        self.status.state = BookState::Valid;
        self.status.applied_updates += 1;
        self.status.snapshot = Some(Arc::new(snapshot));
        self.status.reason = None;
        self.publish();
    }

    pub(crate) fn stale(&mut self, reason: impl Into<String>) {
        self.status.state = BookState::Stale;
        self.status.snapshot = None;
        self.status.reason = Some(reason.into());
        self.publish();
    }

    pub(crate) fn invalid(&mut self, reason: impl Into<String>) {
        self.status.state = BookState::Invalid;
        self.status.snapshot = None;
        self.status.reason = Some(reason.into());
        self.publish();
    }

    fn publish(&self) {
        self.sender.send_replace(self.status.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LevelUpdate {
    pub price: Decimal,
    pub quantity: Decimal,
}

pub(crate) struct LocalOrderBook {
    venue: String,
    symbol: String,
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
    sequence: u64,
    source_timestamp_ms: Option<u64>,
    received_timestamp_ms: u64,
}

impl LocalOrderBook {
    pub(crate) fn from_snapshot(snapshot: OrderBookSnapshot) -> Result<Self> {
        snapshot.validate()?;
        Ok(Self {
            venue: snapshot.venue,
            symbol: snapshot.symbol,
            bids: snapshot
                .bids
                .into_iter()
                .map(|level| (level.price, level.quantity))
                .collect(),
            asks: snapshot
                .asks
                .into_iter()
                .map(|level| (level.price, level.quantity))
                .collect(),
            sequence: snapshot.sequence,
            source_timestamp_ms: snapshot.source_timestamp_ms,
            received_timestamp_ms: snapshot.received_timestamp_ms,
        })
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn apply(
        &mut self,
        bids: &[LevelUpdate],
        asks: &[LevelUpdate],
        sequence: u64,
        source_timestamp_ms: Option<u64>,
        received_timestamp_ms: u64,
    ) -> Result<()> {
        apply_side(&mut self.bids, bids)?;
        apply_side(&mut self.asks, asks)?;
        self.sequence = sequence;
        self.source_timestamp_ms = source_timestamp_ms;
        self.received_timestamp_ms = received_timestamp_ms;
        self.snapshot(1)?;
        Ok(())
    }

    pub(crate) fn snapshot(&self, depth: usize) -> Result<OrderBookSnapshot> {
        if depth == 0 {
            bail!("order book snapshot depth must be positive");
        }
        let snapshot = OrderBookSnapshot {
            venue: self.venue.clone(),
            symbol: self.symbol.clone(),
            bids: self
                .bids
                .iter()
                .rev()
                .take(depth)
                .map(|(&price, &quantity)| Level { price, quantity })
                .collect(),
            asks: self
                .asks
                .iter()
                .take(depth)
                .map(|(&price, &quantity)| Level { price, quantity })
                .collect(),
            sequence: self.sequence,
            source_timestamp_ms: self.source_timestamp_ms,
            received_timestamp_ms: self.received_timestamp_ms,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn apply_side(levels: &mut BTreeMap<Decimal, Decimal>, updates: &[LevelUpdate]) -> Result<()> {
    for update in updates {
        if update.price <= Decimal::ZERO || update.quantity < Decimal::ZERO {
            bail!("order book update has invalid price or quantity");
        }
        if update.quantity == Decimal::ZERO {
            levels.remove(&update.price);
        } else {
            levels.insert(update.price, update.quantity);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal(value: i64) -> Decimal {
        Decimal::from(value)
    }

    fn snapshot() -> OrderBookSnapshot {
        OrderBookSnapshot {
            venue: "venue".into(),
            symbol: "BTCUSDT".into(),
            bids: vec![Level {
                price: decimal(99),
                quantity: decimal(2),
            }],
            asks: vec![Level {
                price: decimal(101),
                quantity: decimal(3),
            }],
            sequence: 10,
            source_timestamp_ms: Some(1),
            received_timestamp_ms: 2,
        }
    }

    #[test]
    fn absolute_updates_replace_insert_and_delete_levels() {
        let mut book = LocalOrderBook::from_snapshot(snapshot()).unwrap();
        book.apply(
            &[
                LevelUpdate {
                    price: decimal(99),
                    quantity: Decimal::ZERO,
                },
                LevelUpdate {
                    price: decimal(100),
                    quantity: decimal(4),
                },
            ],
            &[LevelUpdate {
                price: decimal(101),
                quantity: decimal(5),
            }],
            11,
            Some(3),
            4,
        )
        .unwrap();
        let updated = book.snapshot(10).unwrap();
        assert_eq!(
            updated.bids,
            vec![Level {
                price: decimal(100),
                quantity: decimal(4)
            }]
        );
        assert_eq!(
            updated.asks,
            vec![Level {
                price: decimal(101),
                quantity: decimal(5)
            }]
        );
        assert_eq!(updated.sequence, 11);
    }

    #[test]
    fn non_valid_states_remove_last_snapshot() {
        let (mut publisher, receiver) = FeedPublisher::channel("venue", "BTCUSDT");
        publisher.valid(snapshot());
        assert!(receiver.borrow().snapshot.is_some());

        publisher.stale("silent");
        assert_eq!(receiver.borrow().state, BookState::Stale);
        assert!(receiver.borrow().snapshot.is_none());

        publisher.invalid("gap");
        assert_eq!(receiver.borrow().state, BookState::Invalid);
        assert!(receiver.borrow().snapshot.is_none());
    }

    #[test]
    fn reconnect_generation_clears_snapshot() {
        let (mut publisher, receiver) = FeedPublisher::channel("venue", "BTCUSDT");
        publisher.valid(snapshot());
        publisher.syncing(true);

        let status = receiver.borrow();
        assert_eq!(status.state, BookState::Syncing);
        assert_eq!(status.generation, 1);
        assert_eq!(status.reconnects, 1);
        assert!(status.snapshot.is_none());
    }
}
