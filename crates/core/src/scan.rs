use anyhow::{Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    config::StrategyConfig,
    instrument::InstrumentSpec,
    market::{BookSide, OrderBookSnapshot},
};

const BPS_DENOMINATOR: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ScanReport {
    pub symbol: String,
    pub quantity: Decimal,
    pub observed_at_ms: u64,
    pub pair_received_skew_ms: u64,
    pub directions: Vec<Opportunity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Opportunity {
    pub buy_venue: String,
    pub sell_venue: String,
    pub buy_vwap: Decimal,
    pub sell_vwap: Decimal,
    pub buy_worst_price: Decimal,
    pub sell_worst_price: Decimal,
    pub buy_cost: Decimal,
    pub sell_proceeds: Decimal,
    pub gross_profit: Decimal,
    pub fees: Decimal,
    pub latency_loss_estimate: Decimal,
    pub rebalance_cost: Decimal,
    pub other_direct_cost: Decimal,
    pub expected_net_profit: Decimal,
    pub risk_buffer: Decimal,
    pub admission_profit: Decimal,
    pub admission_net_bps: Decimal,
    pub accepted: bool,
    pub rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct VenueFeeRates {
    pub buy_taker_rate: Decimal,
    pub sell_taker_rate: Decimal,
}

#[derive(Debug, Clone, Copy)]
pub struct VenueFees {
    pub first: VenueFeeRates,
    pub second: VenueFeeRates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct FreshnessLimits {
    pub max_snapshot_age_ms: u64,
    pub max_pair_skew_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct InstrumentPair<'a> {
    pub first: &'a InstrumentSpec,
    pub second: &'a InstrumentSpec,
}

#[derive(Debug, Clone, Copy)]
pub struct ScanInput<'a> {
    pub first_book: &'a OrderBookSnapshot,
    pub second_book: &'a OrderBookSnapshot,
    pub instruments: InstrumentPair<'a>,
    pub quantity: Decimal,
    pub fees: VenueFees,
    pub strategy: &'a StrategyConfig,
    pub now_ms: u64,
    pub freshness: FreshnessLimits,
    pub admission_rejections: &'a [String],
}

#[derive(Debug, Clone, Copy)]
struct DirectionInput<'a> {
    buy_book: &'a OrderBookSnapshot,
    sell_book: &'a OrderBookSnapshot,
    buy_instrument: &'a InstrumentSpec,
    sell_instrument: &'a InstrumentSpec,
    buy_fee_rate: Decimal,
    sell_fee_rate: Decimal,
}

pub fn scan_pair(input: ScanInput<'_>) -> Result<ScanReport> {
    let ScanInput {
        first_book,
        second_book,
        instruments,
        quantity,
        fees,
        strategy,
        now_ms,
        freshness,
        admission_rejections,
    } = input;
    if first_book.symbol != second_book.symbol {
        bail!(
            "cannot compare different symbols: {} and {}",
            first_book.symbol,
            second_book.symbol
        );
    }
    if instruments.first.venue != first_book.venue || instruments.second.venue != second_book.venue
    {
        bail!("instrument specifications do not match order book venues");
    }
    first_book.validate()?;
    second_book.validate()?;

    let first_age = now_ms.saturating_sub(first_book.received_timestamp_ms);
    let second_age = now_ms.saturating_sub(second_book.received_timestamp_ms);
    let pair_skew = first_book
        .received_timestamp_ms
        .abs_diff(second_book.received_timestamp_ms);

    let mut shared_rejections = freshness_rejections(first_age, second_age, pair_skew, freshness);
    shared_rejections.extend_from_slice(admission_rejections);
    let first_to_second = evaluate_direction(
        DirectionInput {
            buy_book: first_book,
            sell_book: second_book,
            buy_instrument: instruments.first,
            sell_instrument: instruments.second,
            buy_fee_rate: fees.first.buy_taker_rate,
            sell_fee_rate: fees.second.sell_taker_rate,
        },
        quantity,
        strategy,
        &shared_rejections,
    )?;
    let second_to_first = evaluate_direction(
        DirectionInput {
            buy_book: second_book,
            sell_book: first_book,
            buy_instrument: instruments.second,
            sell_instrument: instruments.first,
            buy_fee_rate: fees.second.buy_taker_rate,
            sell_fee_rate: fees.first.sell_taker_rate,
        },
        quantity,
        strategy,
        &shared_rejections,
    )?;

    Ok(ScanReport {
        symbol: first_book.symbol.clone(),
        quantity,
        observed_at_ms: now_ms,
        pair_received_skew_ms: pair_skew,
        directions: vec![first_to_second, second_to_first],
    })
}

fn evaluate_direction(
    direction: DirectionInput<'_>,
    quantity: Decimal,
    strategy: &StrategyConfig,
    shared_rejections: &[String],
) -> Result<Opportunity> {
    let buy = direction.buy_book.sweep(BookSide::Asks, quantity)?;
    let sell = direction.sell_book.sweep(BookSide::Bids, quantity)?;
    let bps_denominator = Decimal::from(BPS_DENOMINATOR);
    let gross_profit = sell.quote_amount - buy.quote_amount;
    let fees =
        buy.quote_amount * direction.buy_fee_rate + sell.quote_amount * direction.sell_fee_rate;
    let latency_loss_estimate = buy.quote_amount * strategy.latency_loss_bps / bps_denominator;
    let expected_net_profit = gross_profit
        - fees
        - latency_loss_estimate
        - strategy.rebalance_cost
        - strategy.other_direct_cost;
    let risk_buffer = buy.quote_amount * strategy.risk_buffer_bps / bps_denominator;
    let admission_profit = expected_net_profit - risk_buffer;
    let admission_net_bps = admission_profit / buy.quote_amount * bps_denominator;

    let mut rejection_reasons = shared_rejections.to_vec();
    rejection_reasons.extend(
        direction
            .buy_instrument
            .order_rejection_reasons(quantity, buy.quote_amount),
    );
    rejection_reasons.extend(
        direction
            .sell_instrument
            .order_rejection_reasons(quantity, sell.quote_amount),
    );
    if admission_profit < strategy.min_net_profit {
        rejection_reasons.push(format!(
            "admission profit {} is below minimum {}",
            admission_profit, strategy.min_net_profit
        ));
    }
    if admission_net_bps < strategy.min_net_bps {
        rejection_reasons.push(format!(
            "admission net bps {} is below minimum {}",
            admission_net_bps, strategy.min_net_bps
        ));
    }

    Ok(Opportunity {
        buy_venue: direction.buy_book.venue.clone(),
        sell_venue: direction.sell_book.venue.clone(),
        buy_vwap: buy.vwap,
        sell_vwap: sell.vwap,
        buy_worst_price: buy.worst_price,
        sell_worst_price: sell.worst_price,
        buy_cost: buy.quote_amount,
        sell_proceeds: sell.quote_amount,
        gross_profit,
        fees,
        latency_loss_estimate,
        rebalance_cost: strategy.rebalance_cost,
        other_direct_cost: strategy.other_direct_cost,
        expected_net_profit,
        risk_buffer,
        admission_profit,
        admission_net_bps,
        accepted: rejection_reasons.is_empty(),
        rejection_reasons,
    })
}

fn freshness_rejections(
    first_age: u64,
    second_age: u64,
    pair_skew: u64,
    limits: FreshnessLimits,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if first_age > limits.max_snapshot_age_ms {
        reasons.push(format!(
            "first snapshot age {first_age} ms exceeds limit {} ms",
            limits.max_snapshot_age_ms
        ));
    }
    if second_age > limits.max_snapshot_age_ms {
        reasons.push(format!(
            "second snapshot age {second_age} ms exceeds limit {} ms",
            limits.max_snapshot_age_ms
        ));
    }
    if pair_skew > limits.max_pair_skew_ms {
        reasons.push(format!(
            "snapshot receive skew {pair_skew} ms exceeds limit {} ms",
            limits.max_pair_skew_ms
        ));
    }
    reasons
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;
    use crate::{
        instrument::{InstrumentSpec, OrderCapability, TradingStatus},
        market::Level,
    };

    fn dec(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    fn book(venue: &str, bid: &str, ask: &str, received: u64) -> OrderBookSnapshot {
        OrderBookSnapshot {
            venue: venue.into(),
            symbol: "BTCUSDT".into(),
            bids: vec![Level {
                price: dec(bid),
                quantity: dec("1"),
            }],
            asks: vec![Level {
                price: dec(ask),
                quantity: dec("1"),
            }],
            sequence: 1,
            source_timestamp_ms: None,
            received_timestamp_ms: received,
        }
    }

    fn instrument(venue: &str, min_notional: &str) -> InstrumentSpec {
        InstrumentSpec {
            venue: venue.into(),
            account_type: "spot".into(),
            venue_symbol: "BTCUSDT".into(),
            canonical_instrument_id: "BTC/USDT:SPOT".into(),
            base_asset_id: "BTC".into(),
            quote_asset_id: "USDT".into(),
            settlement_asset_id: "USDT".into(),
            market_type: "spot".into(),
            contract_multiplier: Decimal::ONE,
            quantity_unit: "base_asset".into(),
            price_tick: dec("0.01"),
            quantity_step: dec("0.00001"),
            min_quantity: dec("0.00001"),
            max_quantity: Some(dec("100")),
            min_notional: dec(min_notional),
            max_notional: None,
            trading_status: TradingStatus::Trading,
            supported_order_types: vec![OrderCapability::Limit, OrderCapability::Ioc],
            metadata_version: 1,
        }
    }

    fn strategy() -> StrategyConfig {
        StrategyConfig {
            min_net_profit: dec("0"),
            min_net_bps: dec("0"),
            latency_loss_bps: dec("0"),
            risk_buffer_bps: dec("0"),
            rebalance_cost: dec("1"),
            other_direct_cost: dec("2"),
        }
    }

    #[test]
    fn matches_document_profit_example() {
        let buy_instrument = instrument("buy", "5");
        let sell_instrument = instrument("sell", "5");
        let report = scan_pair(ScanInput {
            first_book: &book("buy", "59990", "60000", 1_000),
            second_book: &book("sell", "60180", "60190", 1_000),
            instruments: InstrumentPair {
                first: &buy_instrument,
                second: &sell_instrument,
            },
            quantity: dec("0.1"),
            fees: VenueFees {
                first: VenueFeeRates {
                    buy_taker_rate: dec("0.001"),
                    sell_taker_rate: dec("0.001"),
                },
                second: VenueFeeRates {
                    buy_taker_rate: dec("0.001"),
                    sell_taker_rate: dec("0.001"),
                },
            },
            strategy: &strategy(),
            now_ms: 1_000,
            freshness: FreshnessLimits {
                max_snapshot_age_ms: 1_000,
                max_pair_skew_ms: 500,
            },
            admission_rejections: &[],
        })
        .unwrap();
        let opportunity = &report.directions[0];
        assert_eq!(opportunity.gross_profit, dec("18"));
        assert_eq!(opportunity.fees, dec("12.018"));
        assert_eq!(opportunity.expected_net_profit, dec("2.982"));
        assert_eq!(opportunity.admission_net_bps, dec("4.97"));
        assert!(opportunity.accepted);
    }

    #[test]
    fn stale_snapshot_is_rejected_even_when_profitable() {
        let buy_instrument = instrument("buy", "0");
        let sell_instrument = instrument("sell", "0");
        let report = scan_pair(ScanInput {
            first_book: &book("buy", "99", "100", 1_000),
            second_book: &book("sell", "110", "111", 1_000),
            instruments: InstrumentPair {
                first: &buy_instrument,
                second: &sell_instrument,
            },
            quantity: dec("1"),
            fees: VenueFees {
                first: VenueFeeRates {
                    buy_taker_rate: Decimal::ZERO,
                    sell_taker_rate: Decimal::ZERO,
                },
                second: VenueFeeRates {
                    buy_taker_rate: Decimal::ZERO,
                    sell_taker_rate: Decimal::ZERO,
                },
            },
            strategy: &strategy(),
            now_ms: 2_001,
            freshness: FreshnessLimits {
                max_snapshot_age_ms: 1_000,
                max_pair_skew_ms: 500,
            },
            admission_rejections: &[],
        })
        .unwrap();
        assert!(!report.directions[0].accepted);
        assert!(
            report.directions[0]
                .rejection_reasons
                .iter()
                .any(|reason| reason.contains("age"))
        );
    }

    #[test]
    fn venue_min_notional_uses_each_direction_actual_notional() {
        let first_instrument = instrument("first", "99.5");
        let second_instrument = instrument("second", "0");
        let report = scan_pair(ScanInput {
            first_book: &book("first", "99", "100", 1_000),
            second_book: &book("second", "110", "111", 1_000),
            instruments: InstrumentPair {
                first: &first_instrument,
                second: &second_instrument,
            },
            quantity: dec("1"),
            fees: VenueFees {
                first: VenueFeeRates {
                    buy_taker_rate: Decimal::ZERO,
                    sell_taker_rate: Decimal::ZERO,
                },
                second: VenueFeeRates {
                    buy_taker_rate: Decimal::ZERO,
                    sell_taker_rate: Decimal::ZERO,
                },
            },
            strategy: &strategy(),
            now_ms: 1_000,
            freshness: FreshnessLimits {
                max_snapshot_age_ms: 1_000,
                max_pair_skew_ms: 500,
            },
            admission_rejections: &[],
        })
        .unwrap();

        assert!(
            report.directions[0]
                .rejection_reasons
                .iter()
                .all(|reason| !reason.contains("first notional"))
        );
        assert!(
            report.directions[1]
                .rejection_reasons
                .iter()
                .any(|reason| reason.contains("first notional 99 is below minimum 99.5"))
        );
    }

    #[test]
    fn account_rejection_fails_closed_without_hiding_fee_estimate() {
        let first_instrument = instrument("first", "0");
        let second_instrument = instrument("second", "0");
        let rejection = "first actual account fee is expired".to_owned();
        let report = scan_pair(ScanInput {
            first_book: &book("first", "99", "100", 1_000),
            second_book: &book("second", "110", "111", 1_000),
            instruments: InstrumentPair {
                first: &first_instrument,
                second: &second_instrument,
            },
            quantity: dec("1"),
            fees: VenueFees {
                first: VenueFeeRates {
                    buy_taker_rate: dec("0.001"),
                    sell_taker_rate: dec("0.002"),
                },
                second: VenueFeeRates {
                    buy_taker_rate: dec("0.003"),
                    sell_taker_rate: dec("0.004"),
                },
            },
            strategy: &strategy(),
            now_ms: 1_000,
            freshness: FreshnessLimits {
                max_snapshot_age_ms: 1_000,
                max_pair_skew_ms: 500,
            },
            admission_rejections: std::slice::from_ref(&rejection),
        })
        .unwrap();

        assert_eq!(report.directions[0].fees, dec("0.54"));
        assert!(!report.directions[0].accepted);
        assert_eq!(report.directions[0].rejection_reasons[0], rejection);
    }

    #[test]
    fn fee_change_can_flip_admission_for_the_same_books() {
        let first_instrument = instrument("first", "0");
        let second_instrument = instrument("second", "0");
        let first_book = book("first", "99", "100", 1_000);
        let second_book = book("second", "110", "111", 1_000);
        let scan = |rate| {
            scan_pair(ScanInput {
                first_book: &first_book,
                second_book: &second_book,
                instruments: InstrumentPair {
                    first: &first_instrument,
                    second: &second_instrument,
                },
                quantity: dec("1"),
                fees: VenueFees {
                    first: VenueFeeRates {
                        buy_taker_rate: rate,
                        sell_taker_rate: rate,
                    },
                    second: VenueFeeRates {
                        buy_taker_rate: rate,
                        sell_taker_rate: rate,
                    },
                },
                strategy: &strategy(),
                now_ms: 1_000,
                freshness: FreshnessLimits {
                    max_snapshot_age_ms: 1_000,
                    max_pair_skew_ms: 500,
                },
                admission_rejections: &[],
            })
            .unwrap()
        };

        let low_fee = scan(Decimal::ZERO);
        let high_fee = scan(dec("0.05"));
        assert_eq!(low_fee.directions[0].expected_net_profit, dec("7"));
        assert!(low_fee.directions[0].accepted);
        assert_eq!(high_fee.directions[0].expected_net_profit, dec("-3.5"));
        assert!(!high_fee.directions[0].accepted);
        assert!(
            high_fee.directions[0]
                .rejection_reasons
                .iter()
                .any(|reason| reason.contains("admission profit"))
        );
    }
}
