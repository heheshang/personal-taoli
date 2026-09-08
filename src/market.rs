use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Level {
    pub price: Decimal,
    pub quantity: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OrderBookSnapshot {
    pub venue: String,
    pub symbol: String,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub sequence: u64,
    pub source_timestamp_ms: Option<u64>,
    pub received_timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSide {
    Bids,
    Asks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sweep {
    pub quantity: Decimal,
    pub quote_amount: Decimal,
    pub vwap: Decimal,
    pub worst_price: Decimal,
}

impl OrderBookSnapshot {
    pub fn validate(&self) -> Result<()> {
        if self.bids.is_empty() || self.asks.is_empty() {
            bail!(
                "{} {} order book has an empty side",
                self.venue,
                self.symbol
            );
        }
        validate_levels(&self.bids, true)?;
        validate_levels(&self.asks, false)?;
        if self.bids[0].price >= self.asks[0].price {
            bail!("{} {} snapshot is crossed", self.venue, self.symbol);
        }
        Ok(())
    }

    pub fn sweep(&self, side: BookSide, target: Decimal) -> Result<Sweep> {
        if target <= Decimal::ZERO {
            bail!("sweep quantity must be positive");
        }
        let levels = match side {
            BookSide::Bids => &self.bids,
            BookSide::Asks => &self.asks,
        };
        let mut remaining = target;
        let mut quote_amount = Decimal::ZERO;
        let mut worst_price = Decimal::ZERO;
        for level in levels {
            let taken = remaining.min(level.quantity);
            quote_amount += taken * level.price;
            remaining -= taken;
            if taken > Decimal::ZERO {
                worst_price = level.price;
            }
            if remaining == Decimal::ZERO {
                return Ok(Sweep {
                    quantity: target,
                    quote_amount,
                    vwap: quote_amount / target,
                    worst_price,
                });
            }
        }
        bail!(
            "insufficient {:?} depth: requested {}, available {}",
            side,
            target,
            target - remaining
        )
    }
}

pub fn unix_timestamp_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

fn validate_levels(levels: &[Level], descending: bool) -> Result<()> {
    for level in levels {
        if level.price <= Decimal::ZERO || level.quantity <= Decimal::ZERO {
            bail!("order book levels must have positive price and quantity");
        }
    }
    for pair in levels.windows(2) {
        let sorted = if descending {
            pair[0].price > pair[1].price
        } else {
            pair[0].price < pair[1].price
        };
        if !sorted {
            bail!("order book levels are not strictly sorted or contain duplicate prices");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn sweep_uses_multiple_price_levels() {
        let book = OrderBookSnapshot {
            venue: "venue".into(),
            symbol: "BTCUSDT".into(),
            bids: vec![Level {
                price: d(99),
                quantity: d(1),
            }],
            asks: vec![
                Level {
                    price: d(100),
                    quantity: d(1),
                },
                Level {
                    price: d(102),
                    quantity: d(2),
                },
            ],
            sequence: 1,
            source_timestamp_ms: None,
            received_timestamp_ms: 1,
        };
        let sweep = book.sweep(BookSide::Asks, d(2)).unwrap();
        assert_eq!(sweep.quote_amount, d(202));
        assert_eq!(sweep.vwap, d(101));
        assert_eq!(sweep.worst_price, d(102));
    }

    #[test]
    fn sweep_rejects_insufficient_depth() {
        let book = OrderBookSnapshot {
            venue: "venue".into(),
            symbol: "BTCUSDT".into(),
            bids: vec![Level {
                price: d(99),
                quantity: d(1),
            }],
            asks: vec![Level {
                price: d(100),
                quantity: d(1),
            }],
            sequence: 1,
            source_timestamp_ms: None,
            received_timestamp_ms: 1,
        };
        assert!(book.sweep(BookSide::Asks, d(2)).is_err());
    }
}
