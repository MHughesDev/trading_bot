//! Append-only ledger writer (C-088).
//!
//! `LedgerWriter::append` inserts one immutable row.  The public API surface
//! intentionally exposes no update or delete method — the compiler enforces
//! the append-only invariant.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Unique identifier for a recorded ledger event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LedgerEventId(pub Uuid);

/// Execution mode — determines which account ledger the event is recorded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountMode {
    #[serde(rename = "PAPER")]
    Paper,
    #[serde(rename = "LIVE")]
    Live,
}

impl AccountMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "PAPER",
            Self::Live => "LIVE",
        }
    }
}

/// Asset-specific fill payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillPayload {
    pub side: String,
    pub qty: Decimal,
    pub price: Decimal,
    pub commission: Decimal,
}

/// Asset-specific fee payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeePayload {
    pub fee_type: String,
    pub amount: Decimal,
    pub currency: String,
}

/// Margin call / margin deposit payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginPayload {
    pub direction: String, // "deposit" | "withdrawal"
    pub amount: Decimal,
}

/// Perpetual-swap funding payment payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingPaymentPayload {
    pub rate: Decimal,
    pub position_qty: Decimal,
    pub payment: Decimal,
}

/// Typed event variants; each carries an asset-specific payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum LedgerEvent {
    Fill {
        id: Uuid,
        user_id: Uuid,
        account_mode: AccountMode,
        venue: String,
        asset_class: String,
        instrument_id: String,
        strategy_id: Option<Uuid>,
        payload: FillPayload,
        usd_value: Decimal,
        context: serde_json::Value,
        occurred_at: DateTime<Utc>,
    },
    Fee {
        id: Uuid,
        user_id: Uuid,
        account_mode: AccountMode,
        venue: String,
        asset_class: String,
        instrument_id: String,
        strategy_id: Option<Uuid>,
        payload: FeePayload,
        usd_value: Decimal,
        context: serde_json::Value,
        occurred_at: DateTime<Utc>,
    },
    Margin {
        id: Uuid,
        user_id: Uuid,
        account_mode: AccountMode,
        venue: String,
        asset_class: String,
        instrument_id: String,
        strategy_id: Option<Uuid>,
        payload: MarginPayload,
        usd_value: Decimal,
        context: serde_json::Value,
        occurred_at: DateTime<Utc>,
    },
    FundingPayment {
        id: Uuid,
        user_id: Uuid,
        account_mode: AccountMode,
        venue: String,
        asset_class: String,
        instrument_id: String,
        strategy_id: Option<Uuid>,
        payload: FundingPaymentPayload,
        usd_value: Decimal,
        context: serde_json::Value,
        occurred_at: DateTime<Utc>,
    },
}

impl LedgerEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Fill { .. } => "fill",
            Self::Fee { .. } => "fee",
            Self::Margin { .. } => "margin",
            Self::FundingPayment { .. } => "funding_payment",
        }
    }

    fn id(&self) -> Uuid {
        match self {
            Self::Fill { id, .. }
            | Self::Fee { id, .. }
            | Self::Margin { id, .. }
            | Self::FundingPayment { id, .. } => *id,
        }
    }

    fn user_id(&self) -> Uuid {
        match self {
            Self::Fill { user_id, .. }
            | Self::Fee { user_id, .. }
            | Self::Margin { user_id, .. }
            | Self::FundingPayment { user_id, .. } => *user_id,
        }
    }

    fn account_mode(&self) -> AccountMode {
        match self {
            Self::Fill { account_mode, .. }
            | Self::Fee { account_mode, .. }
            | Self::Margin { account_mode, .. }
            | Self::FundingPayment { account_mode, .. } => *account_mode,
        }
    }

    fn venue(&self) -> &str {
        match self {
            Self::Fill { venue, .. }
            | Self::Fee { venue, .. }
            | Self::Margin { venue, .. }
            | Self::FundingPayment { venue, .. } => venue,
        }
    }

    fn asset_class(&self) -> &str {
        match self {
            Self::Fill { asset_class, .. }
            | Self::Fee { asset_class, .. }
            | Self::Margin { asset_class, .. }
            | Self::FundingPayment { asset_class, .. } => asset_class,
        }
    }

    fn instrument_id(&self) -> &str {
        match self {
            Self::Fill { instrument_id, .. }
            | Self::Fee { instrument_id, .. }
            | Self::Margin { instrument_id, .. }
            | Self::FundingPayment { instrument_id, .. } => instrument_id,
        }
    }

    fn strategy_id(&self) -> Option<Uuid> {
        match self {
            Self::Fill { strategy_id, .. }
            | Self::Fee { strategy_id, .. }
            | Self::Margin { strategy_id, .. }
            | Self::FundingPayment { strategy_id, .. } => *strategy_id,
        }
    }

    fn payload_json(&self) -> serde_json::Value {
        match self {
            Self::Fill { payload, .. } => serde_json::to_value(payload).unwrap_or_default(),
            Self::Fee { payload, .. } => serde_json::to_value(payload).unwrap_or_default(),
            Self::Margin { payload, .. } => serde_json::to_value(payload).unwrap_or_default(),
            Self::FundingPayment { payload, .. } => {
                serde_json::to_value(payload).unwrap_or_default()
            }
        }
    }

    fn usd_value(&self) -> Decimal {
        match self {
            Self::Fill { usd_value, .. }
            | Self::Fee { usd_value, .. }
            | Self::Margin { usd_value, .. }
            | Self::FundingPayment { usd_value, .. } => *usd_value,
        }
    }

    fn context(&self) -> &serde_json::Value {
        match self {
            Self::Fill { context, .. }
            | Self::Fee { context, .. }
            | Self::Margin { context, .. }
            | Self::FundingPayment { context, .. } => context,
        }
    }

    fn occurred_at(&self) -> DateTime<Utc> {
        match self {
            Self::Fill { occurred_at, .. }
            | Self::Fee { occurred_at, .. }
            | Self::Margin { occurred_at, .. }
            | Self::FundingPayment { occurred_at, .. } => *occurred_at,
        }
    }
}

/// Errors from the ledger writer.
#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Append-only ledger writer.  Exposes no update or delete method.
pub struct LedgerWriter {
    pool: PgPool,
}

impl LedgerWriter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert one ledger event.  Returns the assigned `LedgerEventId`.
    ///
    /// This is the **only** write method on this type — the append-only
    /// invariant is enforced at compile time.
    pub async fn append(&self, event: LedgerEvent) -> Result<LedgerEventId, LedgerError> {
        let id = event.id();
        // ON CONFLICT DO NOTHING provides idempotency: a redelivered fill
        // with the same event id is a no-op (C-088).
        sqlx::query(
            "INSERT INTO ledger_events \
             (id, user_id, account_mode, venue, asset_class, instrument_id, \
              strategy_id, event_type, payload, usd_value, context, occurred_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(event.user_id())
        .bind(event.account_mode().as_str())
        .bind(event.venue())
        .bind(event.asset_class())
        .bind(event.instrument_id())
        .bind(event.strategy_id())
        .bind(event.event_type())
        .bind(event.payload_json())
        .bind(event.usd_value())
        .bind(event.context())
        .bind(event.occurred_at())
        .execute(&self.pool)
        .await?;

        Ok(LedgerEventId(id))
    }
}
