//! Tradier REST `AccountSource` adapter (Set E 3.1).
//!
//! Credentials are the packed `api_token:account_id` form, already decrypted.
//! Endpoints and Bearer auth are the ones the companion `venues/tradier.rs`
//! broker uses; the parsing is split into pure functions so it is tested against
//! recorded response shapes without a network.
//!
//! # Tradier's JSON quirks, handled here
//!
//! * An empty collection comes back as the **string** `"null"`, not `[]`.
//! * A collection of one is a bare **object**, not a one-element array.
//! * Numbers arrive as JSON numbers in some endpoints and strings in others.
//!
//! # What is not verified
//!
//! The history endpoint's paging parameters follow Tradier's published docs and
//! have not been exercised against a live account (no credentials in this
//! build); the fixtures are the documented response shapes.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{header, Client};
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

use domain::money::{Price, Size};

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

/// Production API root.
pub const BASE_URL: &str = "https://api.tradier.com/v1";
/// Most history events asked for per page, and pages followed.
const HISTORY_PAGE_SIZE: usize = 1000;
const HISTORY_MAX_PAGES: usize = 10;

pub struct TradierAccountSource {
    client: Client,
    base_url: String,
}

impl Default for TradierAccountSource {
    fn default() -> Self {
        Self {
            client: Client::new(),
            base_url: BASE_URL.to_owned(),
        }
    }
}

impl TradierAccountSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Point at another root (the sandbox, or a test server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn parse_creds(creds: &VenueCredentials) -> Result<(String, String), AccountSourceError> {
        let text = std::str::from_utf8(&creds.plaintext).map_err(|_| {
            AccountSourceError::Credentials("credentials are not valid UTF-8".to_owned())
        })?;
        let mut parts = text.splitn(2, ':');
        let token = parts.next().unwrap_or("").to_owned();
        let account_id = parts
            .next()
            .ok_or_else(|| {
                AccountSourceError::Credentials("expected api_token:account_id".to_owned())
            })?
            .to_owned();
        if token.is_empty() || account_id.is_empty() {
            return Err(AccountSourceError::Credentials(
                "expected api_token:account_id, both non-empty".to_owned(),
            ));
        }
        Ok((token, account_id))
    }

    fn auth_headers(token: &str) -> header::HeaderMap {
        let mut h = header::HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(&format!("Bearer {token}")) {
            h.insert(header::AUTHORIZATION, v);
        }
        h.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        h
    }

    async fn get_json(
        &self,
        token: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, AccountSourceError> {
        let resp = self
            .client
            .get(format!("{}{path}", self.base_url))
            .headers(Self::auth_headers(token))
            .query(query)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }
        Ok(resp.json().await?)
    }
}

// ── pure parsing ─────────────────────────────────────────────────────────────

/// A JSON number or numeric string as a `Decimal`.
fn dec(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => Decimal::from_str(&n.to_string())
            .ok()
            .or_else(|| Decimal::from_scientific(&n.to_string()).ok()),
        Value::String(s) => Decimal::from_str(s.trim()).ok(),
        _ => None,
    }
}

/// Tradier's collection quirk: absent / `"null"` → empty, one object → one
/// element, array → itself.
pub(crate) fn as_list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Object(_) => vec![v.clone()],
        _ => vec![],
    }
}

/// `GET /accounts/{id}/balances` → one USD [`Balance`].
///
/// `available` is the cash available to trade (`cash.cash_available` on a cash
/// account, else `total_cash`); `locked` is whatever of the cash balance that
/// is not available; `usd_value` is `total_equity`. A body with neither a cash
/// figure nor an equity figure is an error, not a zero balance.
pub(crate) fn parse_balances(body: &Value) -> Result<Vec<Balance>, AccountSourceError> {
    let b = &body["balances"];
    if !b.is_object() {
        return Err(AccountSourceError::Parse(
            "tradier balances response has no `balances` object".to_owned(),
        ));
    }
    let total_cash = dec(&b["total_cash"]);
    let available = dec(&b["cash"]["cash_available"]).or(total_cash);
    let equity = dec(&b["total_equity"]);
    let Some(available) = available else {
        return Err(AccountSourceError::Parse(
            "tradier balances response carries no cash figure".to_owned(),
        ));
    };
    let locked = total_cash
        .map(|t| (t - available).max(Decimal::ZERO))
        .unwrap_or(Decimal::ZERO);
    Ok(vec![Balance {
        asset: "USD".to_owned(),
        available: Size::from_decimal(available),
        locked: Size::from_decimal(locked),
        usd_value: equity.map(Price::from_decimal),
    }])
}

/// An OCC option symbol is root + 6-digit date + C/P + 8-digit strike.
fn is_option_symbol(symbol: &str) -> bool {
    symbol.len() >= 15 && symbol.chars().rev().take(8).all(|c| c.is_ascii_digit())
}

/// `GET /accounts/{id}/positions` → positions.
///
/// Tradier reports `cost_basis` as the **total** paid, so the average entry
/// price is `cost_basis / (|quantity| × multiplier)` (multiplier 100 for an
/// option). A position with zero quantity is not a position and is skipped; one
/// whose numbers cannot be read is an error rather than a silently missing row.
pub(crate) fn parse_positions(body: &Value) -> Result<Vec<VenuePosition>, AccountSourceError> {
    let mut out = Vec::new();
    for p in as_list(&body["positions"]["position"]) {
        let symbol = p["symbol"].as_str().ok_or_else(|| {
            AccountSourceError::Parse("tradier position has no symbol".to_owned())
        })?;
        let qty = dec(&p["quantity"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradier position {symbol} has no quantity"))
        })?;
        if qty.is_zero() {
            continue;
        }
        let cost = dec(&p["cost_basis"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradier position {symbol} has no cost_basis"))
        })?;
        let multiplier = if is_option_symbol(symbol) {
            Decimal::from(100)
        } else {
            Decimal::ONE
        };
        let avg = (cost / (qty.abs() * multiplier)).abs();
        out.push(VenuePosition {
            instrument_id: symbol.to_owned(),
            quantity: qty,
            avg_entry_price: Price::from_decimal(avg),
            unrealized_pnl_usd: None,
        });
    }
    Ok(out)
}

/// `GET /accounts/{id}/history` events → transactions.
///
/// Tradier gives history events no id, so one is derived from the event's own
/// content (`date|type|amount|symbol`) plus its occurrence index among
/// identical ones — stable across calls, which is what a dedup key must be.
pub(crate) fn parse_history(body: &Value) -> Result<Vec<VenueTransaction>, AccountSourceError> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for e in as_list(&body["history"]["event"]) {
        let kind = e["type"].as_str().ok_or_else(|| {
            AccountSourceError::Parse("tradier history event has no type".to_owned())
        })?;
        let amount = dec(&e["amount"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradier {kind} event has no amount"))
        })?;
        let date = e["date"].as_str().ok_or_else(|| {
            AccountSourceError::Parse(format!("tradier {kind} event has no date"))
        })?;
        let occurred_at = date
            .parse::<DateTime<Utc>>()
            .map_err(|e| AccountSourceError::Parse(format!("tradier event date {date:?}: {e}")))?;
        let detail = &e[kind];
        let symbol = detail["symbol"].as_str().map(str::to_owned);
        let key = format!("{date}|{kind}|{amount}|{}", symbol.as_deref().unwrap_or(""));
        let n = seen.entry(key.clone()).or_insert(0);
        let id = format!("{key}|{n}");
        *n += 1;
        out.push(VenueTransaction {
            id,
            transaction_type: kind.to_owned(),
            instrument_id: symbol,
            amount,
            currency: "USD",
            occurred_at,
        });
    }
    Ok(out)
}

#[async_trait]
impl AccountSource for TradierAccountSource {
    fn venue_id(&self) -> &str {
        "tradier"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let (token, account_id) = Self::parse_creds(creds)?;
        let body = self
            .get_json(&token, &format!("/accounts/{account_id}/balances"), &[])
            .await?;
        parse_balances(&body)
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let (token, account_id) = Self::parse_creds(creds)?;
        let body = self
            .get_json(&token, &format!("/accounts/{account_id}/positions"), &[])
            .await?;
        parse_positions(&body)
    }

    async fn fetch_transactions(
        &self,
        creds: &VenueCredentials,
        _user_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        let (token, account_id) = Self::parse_creds(creds)?;
        let mut all = Vec::new();
        for page in 1..=HISTORY_MAX_PAGES {
            let mut q = vec![
                ("limit", HISTORY_PAGE_SIZE.to_string()),
                ("page", page.to_string()),
            ];
            if let Some(s) = since {
                q.push(("start", s.format("%Y-%m-%d").to_string()));
            }
            let body = self
                .get_json(&token, &format!("/accounts/{account_id}/history"), &q)
                .await?;
            let n = as_list(&body["history"]["event"]).len();
            all.extend(parse_history(&body)?);
            if n < HISTORY_PAGE_SIZE {
                break;
            }
        }
        if let Some(s) = since {
            all.retain(|t| t.occurred_at >= s);
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_empty_string_null_and_the_single_object_are_both_lists() {
        assert!(as_list(&json!("null")).is_empty());
        assert!(as_list(&Value::Null).is_empty());
        assert_eq!(as_list(&json!({"a": 1})).len(), 1);
        assert_eq!(as_list(&json!([{"a": 1}, {"a": 2}])).len(), 2);
    }

    #[test]
    fn a_cash_account_balance_reads_available_locked_and_equity() {
        let body = json!({"balances": {
            "total_cash": 1000.5, "total_equity": 2500.25,
            "cash": {"cash_available": "800.5", "sweep": 0, "unsettled_funds": 0}
        }});
        let b = parse_balances(&body).unwrap();
        assert_eq!(b[0].asset, "USD");
        assert_eq!(b[0].available.inner(), Decimal::from_str("800.5").unwrap());
        assert_eq!(b[0].locked.inner(), Decimal::from(200));
        assert_eq!(
            b[0].usd_value.unwrap().inner(),
            Decimal::from_str("2500.25").unwrap()
        );
        assert!(
            parse_balances(&json!({"balances": {}})).is_err(),
            "no figure is not zero"
        );
        assert!(parse_balances(&json!({})).is_err());
    }

    #[test]
    fn positions_handle_the_null_string_the_single_object_and_option_multipliers() {
        assert!(parse_positions(&json!({"positions": "null"}))
            .unwrap()
            .is_empty());
        let one = json!({"positions": {"position": {
            "symbol": "AAPL", "quantity": 10, "cost_basis": 1500.0
        }}});
        let p = parse_positions(&one).unwrap();
        assert_eq!(p[0].avg_entry_price.inner(), Decimal::from(150));
        let many = json!({"positions": {"position": [
            {"symbol": "AAPL250117C00150000", "quantity": 2, "cost_basis": 600.0},
            {"symbol": "TSLA", "quantity": -5, "cost_basis": -1000.0},
            {"symbol": "DEAD", "quantity": 0, "cost_basis": 0.0}
        ]}});
        let p = parse_positions(&many).unwrap();
        assert_eq!(p.len(), 2, "the zero-quantity row is skipped");
        assert_eq!(
            p[0].avg_entry_price.inner(),
            Decimal::from(3),
            "600 / (2 x 100)"
        );
        assert_eq!(p[1].quantity, Decimal::from(-5));
        assert_eq!(
            p[1].avg_entry_price.inner(),
            Decimal::from(200),
            "short cost is a magnitude"
        );
        assert!(
            parse_positions(&json!({"positions": {"position": {"symbol": "X", "quantity": 1}}}))
                .is_err(),
            "a position with no cost_basis is an error, not a free one"
        );
    }

    #[test]
    fn history_events_get_stable_ids_and_identical_events_stay_distinct() {
        let body = json!({"history": {"event": [
            {"amount": -150.0, "date": "2026-09-01T00:00:00Z", "type": "trade",
             "trade": {"symbol": "AAPL", "quantity": 1, "price": 150.0}},
            {"amount": -150.0, "date": "2026-09-01T00:00:00Z", "type": "trade",
             "trade": {"symbol": "AAPL", "quantity": 1, "price": 150.0}},
            {"amount": 5.0, "date": "2026-09-02T00:00:00Z", "type": "dividend",
             "dividend": {"symbol": "AAPL"}},
        ]}});
        let a = parse_history(&body).unwrap();
        let b = parse_history(&body).unwrap();
        assert_eq!(a.len(), 3);
        assert_ne!(a[0].id, a[1].id);
        assert_eq!(a[0].id, b[0].id, "ids are stable across calls");
        assert_eq!(a[2].transaction_type, "dividend");
        assert_eq!(a[0].instrument_id.as_deref(), Some("AAPL"));
        assert!(parse_history(&json!({"history": "null"}))
            .unwrap()
            .is_empty());
    }
}
