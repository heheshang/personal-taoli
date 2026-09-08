use anyhow::{Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingStatus {
    Trading,
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderCapability {
    Limit,
    Market,
    Ioc,
    Fok,
    PostOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct InstrumentSpec {
    pub venue: String,
    pub account_type: String,
    pub venue_symbol: String,
    pub canonical_instrument_id: String,
    pub base_asset_id: String,
    pub quote_asset_id: String,
    pub settlement_asset_id: String,
    pub market_type: String,
    pub contract_multiplier: Decimal,
    pub quantity_unit: String,
    pub price_tick: Decimal,
    pub quantity_step: Decimal,
    pub min_quantity: Decimal,
    pub max_quantity: Option<Decimal>,
    pub min_notional: Decimal,
    pub max_notional: Option<Decimal>,
    pub trading_status: TradingStatus,
    pub supported_order_types: Vec<OrderCapability>,
    pub metadata_version: u64,
}

impl InstrumentSpec {
    pub fn validate_static(
        &self,
        symbol: &str,
        base_asset: &str,
        quote_asset: &str,
        quantity: Decimal,
    ) -> Result<()> {
        if self.venue_symbol != symbol {
            bail!(
                "{} returned symbol {}, expected {symbol}",
                self.venue,
                self.venue_symbol
            );
        }
        if self.base_asset_id != base_asset || self.quote_asset_id != quote_asset {
            bail!(
                "{} asset mapping is {}/{}, expected {base_asset}/{quote_asset}",
                self.venue,
                self.base_asset_id,
                self.quote_asset_id
            );
        }
        if self.account_type != "spot" || self.market_type != "spot" {
            bail!("{} instrument is not spot", self.venue);
        }
        if self.trading_status != TradingStatus::Trading {
            bail!("{} instrument is not trading", self.venue);
        }
        if self.price_tick <= Decimal::ZERO
            || self.quantity_step <= Decimal::ZERO
            || self.min_quantity <= Decimal::ZERO
            || self.min_notional < Decimal::ZERO
        {
            bail!(
                "{} returned invalid non-positive trading filters",
                self.venue
            );
        }
        if !self.supported_order_types.contains(&OrderCapability::Limit)
            || !self.supported_order_types.contains(&OrderCapability::Ioc)
        {
            bail!("{} does not declare LIMIT and IOC support", self.venue);
        }
        if quantity < self.min_quantity {
            bail!(
                "{} quantity {quantity} is below minimum {}",
                self.venue,
                self.min_quantity
            );
        }
        if self.max_quantity.is_some_and(|maximum| quantity > maximum) {
            bail!("{} quantity {quantity} exceeds maximum", self.venue);
        }
        if quantity % self.quantity_step != Decimal::ZERO {
            bail!(
                "{} quantity {quantity} is not aligned to step {}",
                self.venue,
                self.quantity_step
            );
        }
        Ok(())
    }

    pub fn order_rejection_reasons(
        &self,
        quantity: Decimal,
        quote_notional: Decimal,
    ) -> Vec<String> {
        let mut reasons = Vec::new();
        if quantity < self.min_quantity {
            reasons.push(format!(
                "{} quantity {} is below minimum {}",
                self.venue, quantity, self.min_quantity
            ));
        }
        if self.max_quantity.is_some_and(|maximum| quantity > maximum) {
            reasons.push(format!(
                "{} quantity {} exceeds maximum {}",
                self.venue,
                quantity,
                self.max_quantity.unwrap_or_default()
            ));
        }
        if quantity % self.quantity_step != Decimal::ZERO {
            reasons.push(format!(
                "{} quantity {} is not aligned to step {}",
                self.venue, quantity, self.quantity_step
            ));
        }
        if quote_notional < self.min_notional {
            reasons.push(format!(
                "{} notional {} is below minimum {}",
                self.venue, quote_notional, self.min_notional
            ));
        }
        if self
            .max_notional
            .is_some_and(|maximum| quote_notional > maximum)
        {
            reasons.push(format!(
                "{} notional {} exceeds maximum {}",
                self.venue,
                quote_notional,
                self.max_notional.unwrap_or_default()
            ));
        }
        reasons
    }
}

pub fn validate_pair(
    first: &InstrumentSpec,
    second: &InstrumentSpec,
    symbol: &str,
    base_asset: &str,
    quote_asset: &str,
    quantity: Decimal,
) -> Result<()> {
    first.validate_static(symbol, base_asset, quote_asset, quantity)?;
    second.validate_static(symbol, base_asset, quote_asset, quantity)?;
    if first.canonical_instrument_id != second.canonical_instrument_id {
        bail!(
            "canonical instruments differ: {} and {}",
            first.canonical_instrument_id,
            second.canonical_instrument_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    fn spec(venue: &str, step: &str, min_notional: &str) -> InstrumentSpec {
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
            price_tick: dec("0.1"),
            quantity_step: dec(step),
            min_quantity: dec("0.00001"),
            max_quantity: Some(dec("100")),
            min_notional: dec(min_notional),
            max_notional: None,
            trading_status: TradingStatus::Trading,
            supported_order_types: vec![OrderCapability::Limit, OrderCapability::Ioc],
            metadata_version: 1,
        }
    }

    #[test]
    fn pair_rejects_quantity_not_shared_by_both_steps() {
        let first = spec("first", "0.001", "5");
        let second = spec("second", "0.01", "5");
        let error =
            validate_pair(&first, &second, "BTCUSDT", "BTC", "USDT", dec("0.001")).unwrap_err();
        assert!(error.to_string().contains("second quantity"));
    }

    #[test]
    fn order_rejects_notional_below_venue_minimum() {
        let instrument = spec("venue", "0.001", "5");
        let reasons = instrument.order_rejection_reasons(dec("0.001"), dec("4.99"));
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("notional"));
    }
}
