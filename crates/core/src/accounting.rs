use anyhow::{Context, Result, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::Row;

use crate::db::{DomainConnection, to_i64, to_u64, validate_id};
use crate::market::unix_timestamp_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LedgerDirection {
    Debit,
    Credit,
}

impl LedgerDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debit => "DEBIT",
            Self::Credit => "CREDIT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerLineInput {
    pub account_id: String,
    pub venue: String,
    pub asset: String,
    pub direction: LedgerDirection,
    pub amount: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEventInput {
    pub event_id: String,
    pub business_key: String,
    pub event_type: String,
    pub account_id: String,
    pub venue: String,
    pub occurred_at_ms: u64,
    pub payload: Value,
    pub lines: Vec<LedgerLineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerEvent {
    pub event_id: String,
    pub business_key: String,
    pub event_type: String,
    pub account_id: String,
    pub venue: String,
    pub occurred_at_ms: u64,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerBalance {
    pub account_id: String,
    pub venue: String,
    pub asset: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub net: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountingSmokeReport {
    pub schema_version: i64,
    pub balanced_event_recorded: bool,
    pub idempotent_replay: bool,
    pub unbalanced_event_rejected: bool,
    pub immutable_history: bool,
    pub external_order_calls: usize,
}

pub struct LedgerCore {
    inner: DomainConnection,
}

impl LedgerCore {
    pub async fn acquire(database_url: &str) -> Result<Self> {
        let inner =
            DomainConnection::acquire(database_url, "accounting", "ledger", "accounting").await?;
        Ok(Self { inner })
    }

    pub async fn record_event(&mut self, input: LedgerEventInput) -> Result<LedgerEvent> {
        validate_event(&input)?;
        let now = unix_timestamp_ms()?;
        let tx = self
            .inner
            .client
            .transaction()
            .await
            .context("failed to begin ledger transaction")?;
        let existing = tx
            .query_opt(
                "SELECT event_id FROM ledger_events WHERE business_key=$1",
                &[&input.business_key],
            )
            .await
            .context("failed to inspect ledger idempotency key")?;
        if let Some(row) = existing {
            let existing_id: String = row.get(0);
            if existing_id != input.event_id {
                bail!("ledger business key was reused with another event_id");
            }
            tx.commit()
                .await
                .context("failed to commit ledger replay")?;
            return self.load_event(&input.event_id).await;
        }
        let occurred_at_ms = to_i64(input.occurred_at_ms)?;
        let created_at_ms = to_i64(now)?;
        tx.execute("INSERT INTO ledger_events(event_id,business_key,event_type,account_id,venue,occurred_at_ms,created_at_ms,payload) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&input.event_id, &input.business_key, &input.event_type, &input.account_id, &input.venue, &occurred_at_ms, &created_at_ms, &input.payload]).await.context("failed to append ledger event")?;
        for line in &input.lines {
            tx.execute("INSERT INTO ledger_lines(event_id,account_id,venue,asset,direction,amount,created_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&input.event_id, &line.account_id, &line.venue, &line.asset, &line.direction.as_str(), &line.amount, &created_at_ms]).await.context("failed to append ledger line")?;
        }
        tx.commit().await.context("failed to commit ledger event")?;
        self.load_event(&input.event_id).await
    }

    pub async fn balance(
        &self,
        account_id: &str,
        venue: &str,
        asset: &str,
    ) -> Result<LedgerBalance> {
        validate_id("account_id", account_id)?;
        validate_id("venue", venue)?;
        validate_id("asset", asset)?;
        let row = self.inner.client.query_one("SELECT COALESCE(SUM(amount) FILTER (WHERE direction='DEBIT'),0), COALESCE(SUM(amount) FILTER (WHERE direction='CREDIT'),0) FROM ledger_lines WHERE account_id=$1 AND venue=$2 AND asset=$3", &[&account_id, &venue, &asset]).await.context("failed to load ledger balance")?;
        let debit: Decimal = row.get(0);
        let credit: Decimal = row.get(1);
        Ok(LedgerBalance {
            account_id: account_id.to_owned(),
            venue: venue.to_owned(),
            asset: asset.to_owned(),
            debit,
            credit,
            net: debit - credit,
        })
    }

    pub async fn load_event(&self, event_id: &str) -> Result<LedgerEvent> {
        validate_id("event_id", event_id)?;
        let row = self.inner.client.query_one("SELECT event_id,business_key,event_type,account_id,venue,occurred_at_ms,(SELECT COUNT(*) FROM ledger_lines WHERE event_id=ledger_events.event_id) FROM ledger_events WHERE event_id=$1", &[&event_id]).await.context("failed to load ledger event")?;
        event(row)
    }

    pub async fn disconnect(self) {
        self.inner.disconnect().await;
    }
}

pub async fn run_accounting_smoke(database_url: &str) -> Result<AccountingSmokeReport> {
    crate::db::migrate(database_url).await?;
    let stamp = format!("{}-{}", unix_timestamp_ms()?, std::process::id());
    let mut core = LedgerCore::acquire(database_url).await?;
    let event = LedgerEventInput {
        event_id: format!("ledger-event-{stamp}"),
        business_key: format!("fill-{stamp}"),
        event_type: "FILL_POSTED".to_owned(),
        account_id: format!("ledger-account-{stamp}"),
        venue: "paper-a".to_owned(),
        occurred_at_ms: unix_timestamp_ms()?,
        payload: json!({"source":"smoke"}),
        lines: vec![
            LedgerLineInput {
                account_id: format!("ledger-account-{stamp}"),
                venue: "paper-a".to_owned(),
                asset: "BTC".to_owned(),
                direction: LedgerDirection::Debit,
                amount: Decimal::ONE,
            },
            LedgerLineInput {
                account_id: format!("ledger-account-{stamp}"),
                venue: "paper-a".to_owned(),
                asset: "BTC".to_owned(),
                direction: LedgerDirection::Credit,
                amount: Decimal::ONE,
            },
        ],
    };
    let first = core.record_event(event.clone()).await?;
    let replay = core.record_event(event.clone()).await?;
    let unbalanced = LedgerEventInput {
        event_id: format!("bad-event-{stamp}"),
        business_key: format!("bad-{stamp}"),
        lines: vec![LedgerLineInput {
            account_id: event.account_id.clone(),
            venue: event.venue.clone(),
            asset: "USDT".to_owned(),
            direction: LedgerDirection::Debit,
            amount: Decimal::ONE,
        }],
        ..event.clone()
    };
    let unbalanced_event_rejected = core.record_event(unbalanced).await.is_err();
    let balance = core.balance(&event.account_id, &event.venue, "BTC").await?;
    let immutable_history = core
        .inner
        .client
        .execute(
            "UPDATE ledger_events SET event_type='MUTATED' WHERE event_id=$1",
            &[&first.event_id],
        )
        .await
        .is_err();
    core.disconnect().await;
    Ok(AccountingSmokeReport {
        schema_version: 4,
        balanced_event_recorded: first.line_count == 2 && balance.net == Decimal::ZERO,
        idempotent_replay: replay.event_id == first.event_id,
        unbalanced_event_rejected,
        immutable_history,
        external_order_calls: 0,
    })
}

fn validate_event(input: &LedgerEventInput) -> Result<()> {
    for (name, value) in [
        ("event_id", input.event_id.as_str()),
        ("business_key", input.business_key.as_str()),
        ("event_type", input.event_type.as_str()),
        ("account_id", input.account_id.as_str()),
        ("venue", input.venue.as_str()),
    ] {
        validate_id(name, value)?;
    }
    if input.occurred_at_ms == 0 || input.lines.len() < 2 {
        bail!("ledger event requires a timestamp and at least two lines");
    }
    let mut totals = std::collections::BTreeMap::<(&str, &str, &str), (Decimal, Decimal)>::new();
    for line in &input.lines {
        validate_id("line account_id", &line.account_id)?;
        validate_id("line venue", &line.venue)?;
        validate_id("line asset", &line.asset)?;
        if line.account_id != input.account_id
            || line.venue != input.venue
            || line.amount <= Decimal::ZERO
        {
            bail!("ledger line is outside event domain or has invalid amount");
        }
        let totals = totals
            .entry((&line.account_id, &line.venue, &line.asset))
            .or_default();
        match line.direction {
            LedgerDirection::Debit => totals.0 += line.amount,
            LedgerDirection::Credit => totals.1 += line.amount,
        }
    }
    if totals.values().any(|(debit, credit)| debit != credit) {
        bail!("ledger event is not balanced per asset");
    }
    Ok(())
}

fn event(row: Row) -> Result<LedgerEvent> {
    Ok(LedgerEvent {
        event_id: row.get(0),
        business_key: row.get(1),
        event_type: row.get(2),
        account_id: row.get(3),
        venue: row.get(4),
        occurred_at_ms: to_u64(row.get::<_, i64>(5))?,
        line_count: row.get::<_, i64>(6) as usize,
    })
}
