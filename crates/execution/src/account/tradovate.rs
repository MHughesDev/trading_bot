//! Tradovate REST `AccountSource` adapter (Set E 3.2, locked decision 6).
//!
//! # Credentials
//!
//! The decrypted blob is `username:password[:appId[:cid[:sec]]]` — the form the
//! frontend collects. The adapter performs Tradovate's **token exchange
//! in-adapter** (`POST /auth/accesstoken`), caches the access token per
//! username, renews it five minutes before it expires, and — if a call is still
//! answered 401 — drops the cached token, exchanges again and retries **once**.
//! A password containing `:` is not supported in this packed form.
//!
//! # Endpoints
//!
//! * `GET /account/list` → the user's accounts;
//! * `POST /cashBalance/getcashbalancesnapshot {accountId}` → balance;
//! * `GET /position/list` → positions, with numeric `contractId`s resolved to
//!   symbols through `GET /contract/item?id=` (cached);
//! * `GET /fill/list` → fills, as transactions.
//!
//! Unverified against a live account (no credentials in this build): the shapes
//! below follow Tradovate's published API and are covered by fixtures and a
//! local HTTP stand-in (`tests/account_http.rs`).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use reqwest::{header, Client, Method, StatusCode};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use uuid::Uuid;

use domain::money::{Price, Size};

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

/// Demo API root. Live is `https://live.tradovateapi.com/v1`.
pub const DEMO_BASE_URL: &str = "https://demo.tradovateapi.com/v1";
/// Renew a token this long before it expires.
const RENEW_BEFORE: Duration = Duration::minutes(5);

/// Parsed credential blob.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TradovateCreds {
    pub username: String,
    pub password: String,
    pub app_id: String,
    pub cid: Option<String>,
    pub sec: Option<String>,
}

impl std::fmt::Debug for TradovateCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TradovateCreds([REDACTED])")
    }
}

pub struct TradovateAccountSource {
    client: Client,
    base_url: String,
    /// username → (access token, expiry)
    tokens: Mutex<HashMap<String, (String, DateTime<Utc>)>>,
    /// contractId → symbol
    contracts: Mutex<HashMap<i64, String>>,
}

impl Default for TradovateAccountSource {
    fn default() -> Self {
        Self {
            client: Client::new(),
            base_url: DEMO_BASE_URL.to_owned(),
            tokens: Mutex::new(HashMap::new()),
            contracts: Mutex::new(HashMap::new()),
        }
    }
}

impl TradovateAccountSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Point at another root (live, or a test server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub(crate) fn parse_creds(
        creds: &VenueCredentials,
    ) -> Result<TradovateCreds, AccountSourceError> {
        let text = std::str::from_utf8(&creds.plaintext).map_err(|_| {
            AccountSourceError::Credentials("credentials are not valid UTF-8".to_owned())
        })?;
        let mut parts = text.splitn(5, ':');
        let username = parts.next().unwrap_or("").to_owned();
        let password = parts
            .next()
            .ok_or_else(|| {
                AccountSourceError::Credentials(
                    "expected username:password[:appId[:cid[:sec]]]".to_owned(),
                )
            })?
            .to_owned();
        if username.is_empty() || password.is_empty() {
            return Err(AccountSourceError::Credentials(
                "expected username:password, both non-empty".to_owned(),
            ));
        }
        let app_id = parts
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("trading-platform")
            .to_owned();
        let cid = parts.next().filter(|s| !s.is_empty()).map(str::to_owned);
        let sec = parts.next().filter(|s| !s.is_empty()).map(str::to_owned);
        Ok(TradovateCreds {
            username,
            password,
            app_id,
            cid,
            sec,
        })
    }

    fn cached_token(&self, username: &str) -> Option<String> {
        let tokens = self.tokens.lock().unwrap_or_else(|p| p.into_inner());
        tokens
            .get(username)
            .filter(|(_, exp)| *exp - Utc::now() > RENEW_BEFORE)
            .map(|(t, _)| t.clone())
    }

    fn forget_token(&self, username: &str) {
        self.tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(username);
    }

    /// The access token for `creds`: cached, or exchanged now.
    async fn access_token(&self, creds: &TradovateCreds) -> Result<String, AccountSourceError> {
        if let Some(t) = self.cached_token(&creds.username) {
            return Ok(t);
        }
        let mut body = json!({
            "name": creds.username,
            "password": creds.password,
            "appId": creds.app_id,
            "appVersion": "1.0",
        });
        if let (Some(cid), Some(sec)) = (&creds.cid, &creds.sec) {
            body["cid"] = json!(cid.parse::<i64>().unwrap_or(0));
            body["sec"] = json!(sec);
        }
        let resp = self
            .client
            .post(format!("{}/auth/accesstoken", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }
        let parsed: Value = resp.json().await?;
        let (token, expires) = parse_token_response(&parsed, Utc::now())?;
        self.tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(creds.username.clone(), (token.clone(), expires));
        Ok(token)
    }

    /// An authenticated call. A 401 drops the cached token, exchanges again and
    /// retries once; a second 401 is an error.
    async fn call(
        &self,
        creds: &TradovateCreds,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, AccountSourceError> {
        for attempt in 0..2 {
            let token = self.access_token(creds).await?;
            let mut req = self
                .client
                .request(method.clone(), format!("{}{path}", self.base_url))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .query(query);
            if let Some(b) = &body {
                req = req.json(b);
            }
            let resp = req.send().await?;
            if resp.status() == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.forget_token(&creds.username);
                continue;
            }
            if !resp.status().is_success() {
                return Err(AccountSourceError::HttpStatus(
                    resp.text().await.unwrap_or_default(),
                ));
            }
            return Ok(resp.json().await?);
        }
        Err(AccountSourceError::HttpStatus(
            "tradovate rejected a freshly exchanged token".to_owned(),
        ))
    }

    async fn symbol_for(
        &self,
        creds: &TradovateCreds,
        contract_id: i64,
    ) -> Result<String, AccountSourceError> {
        if let Some(s) = self
            .contracts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&contract_id)
        {
            return Ok(s.clone());
        }
        let item = self
            .call(
                creds,
                Method::GET,
                "/contract/item",
                &[("id", contract_id.to_string())],
                None,
            )
            .await?;
        let name = item["name"].as_str().ok_or_else(|| {
            AccountSourceError::Parse(format!("tradovate contract {contract_id} has no name"))
        })?;
        self.contracts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(contract_id, name.to_owned());
        Ok(name.to_owned())
    }
}

// ── pure parsing ─────────────────────────────────────────────────────────────

fn dec(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => Decimal::from_str(&n.to_string())
            .ok()
            .or_else(|| Decimal::from_scientific(&n.to_string()).ok()),
        Value::String(s) => Decimal::from_str(s.trim()).ok(),
        _ => None,
    }
}

/// The `/auth/accesstoken` response → (token, expiry).
///
/// Tradovate answers a throttled login with a **200** carrying `p-ticket` /
/// `p-time` and no token; that is surfaced as an error naming the wait, never
/// as an empty token.
pub(crate) fn parse_token_response(
    body: &Value,
    now: DateTime<Utc>,
) -> Result<(String, DateTime<Utc>), AccountSourceError> {
    if let Some(wait) = body.get("p-time") {
        return Err(AccountSourceError::HttpStatus(format!(
            "tradovate is throttling this login; retry in {wait} s"
        )));
    }
    if let Some(err) = body["errorText"].as_str() {
        return Err(AccountSourceError::HttpStatus(err.to_owned()));
    }
    let token = body["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AccountSourceError::Parse("tradovate token response has no accessToken".to_owned())
        })?;
    // No stated expiry → a conservative 60 minutes (Tradovate's documented
    // lifetime is 90), so a renewal happens rather than a stale token lingering.
    let expires = body["expirationTime"]
        .as_str()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(now + Duration::minutes(60));
    Ok((token.to_owned(), expires))
}

/// `/cashBalance/getcashbalancesnapshot` → a USD [`Balance`].
pub(crate) fn parse_cash_snapshot(body: &Value) -> Result<Balance, AccountSourceError> {
    let cash = dec(&body["totalCashValue"]).or_else(|| dec(&body["amount"])).ok_or_else(|| {
        AccountSourceError::Parse("tradovate cash snapshot carries no cash figure".to_owned())
    })?;
    let locked = dec(&body["initialMargin"]).unwrap_or(Decimal::ZERO).max(Decimal::ZERO);
    let equity = dec(&body["openPnL"]).map(|p| cash + p);
    Ok(Balance {
        asset: "USD".to_owned(),
        available: Size::from_decimal((cash - locked).max(Decimal::ZERO)),
        locked: Size::from_decimal(locked),
        usd_value: equity.map(Price::from_decimal),
    })
}

/// A position row with its contract still numeric.
#[derive(Debug, PartialEq)]
pub(crate) struct RawPosition {
    pub contract_id: i64,
    pub net_pos: Decimal,
    pub net_price: Decimal,
}

/// `/position/list` → open positions (closed rows, `netPos == 0`, are dropped).
pub(crate) fn parse_positions(body: &Value) -> Result<Vec<RawPosition>, AccountSourceError> {
    let rows = body
        .as_array()
        .ok_or_else(|| AccountSourceError::Parse("tradovate position list is not an array".to_owned()))?;
    let mut out = Vec::new();
    for p in rows {
        let contract_id = p["contractId"].as_i64().ok_or_else(|| {
            AccountSourceError::Parse("tradovate position has no contractId".to_owned())
        })?;
        let net_pos = dec(&p["netPos"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradovate position {contract_id} has no netPos"))
        })?;
        if net_pos.is_zero() {
            continue;
        }
        let net_price = dec(&p["netPrice"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradovate position {contract_id} has no netPrice"))
        })?;
        out.push(RawPosition {
            contract_id,
            net_pos,
            net_price,
        });
    }
    Ok(out)
}

/// `/fill/list` rows → transactions, newest data first as Tradovate sends it.
/// Contract ids are resolved by the caller; `symbols` maps them.
pub(crate) fn parse_fills(
    body: &Value,
    symbols: &HashMap<i64, String>,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<VenueTransaction>, AccountSourceError> {
    let rows = body
        .as_array()
        .ok_or_else(|| AccountSourceError::Parse("tradovate fill list is not an array".to_owned()))?;
    let mut out = Vec::new();
    for f in rows {
        let id = f["id"].as_i64().ok_or_else(|| {
            AccountSourceError::Parse("tradovate fill has no id".to_owned())
        })?;
        let kind = match f["action"].as_str() {
            Some("Buy") => "fill_buy",
            Some("Sell") => "fill_sell",
            other => {
                return Err(AccountSourceError::Parse(format!(
                    "tradovate fill {id} has action {other:?}; refusing to guess a direction"
                )))
            }
        };
        let qty = dec(&f["qty"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradovate fill {id} has no qty"))
        })?;
        let price = dec(&f["price"]).ok_or_else(|| {
            AccountSourceError::Parse(format!("tradovate fill {id} has no price"))
        })?;
        let at = f["timestamp"]
            .as_str()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .ok_or_else(|| {
                AccountSourceError::Parse(format!("tradovate fill {id} has no readable timestamp"))
            })?;
        if since.is_some_and(|s| at < s) {
            continue;
        }
        let contract_id = f["contractId"].as_i64();
        out.push(VenueTransaction {
            id: id.to_string(),
            transaction_type: kind.to_owned(),
            instrument_id: contract_id.and_then(|c| symbols.get(&c).cloned()),
            amount: qty * price,
            currency: "USD",
            occurred_at: at,
        });
    }
    Ok(out)
}

#[async_trait]
impl AccountSource for TradovateAccountSource {
    fn venue_id(&self) -> &str {
        "tradovate"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let c = Self::parse_creds(creds)?;
        let accounts = self.call(&c, Method::GET, "/account/list", &[], None).await?;
        let mut out = Vec::new();
        for a in accounts.as_array().cloned().unwrap_or_default() {
            let Some(id) = a["id"].as_i64() else { continue };
            let snap = self
                .call(
                    &c,
                    Method::POST,
                    "/cashBalance/getcashbalancesnapshot",
                    &[],
                    Some(json!({ "accountId": id })),
                )
                .await?;
            out.push(parse_cash_snapshot(&snap)?);
        }
        Ok(out)
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let c = Self::parse_creds(creds)?;
        let body = self.call(&c, Method::GET, "/position/list", &[], None).await?;
        let mut out = Vec::new();
        for p in parse_positions(&body)? {
            let symbol = self.symbol_for(&c, p.contract_id).await?;
            out.push(VenuePosition {
                instrument_id: symbol,
                quantity: p.net_pos,
                avg_entry_price: Price::from_decimal(p.net_price),
                unrealized_pnl_usd: None,
            });
        }
        Ok(out)
    }

    async fn fetch_transactions(
        &self,
        creds: &VenueCredentials,
        _user_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        let c = Self::parse_creds(creds)?;
        let body = self.call(&c, Method::GET, "/fill/list", &[], None).await?;
        let mut symbols = HashMap::new();
        for f in body.as_array().cloned().unwrap_or_default() {
            if let Some(cid) = f["contractId"].as_i64() {
                if let std::collections::hash_map::Entry::Vacant(e) = symbols.entry(cid) {
                    e.insert(self.symbol_for(&c, cid).await?);
                }
            }
        }
        parse_fills(&body, &symbols, since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(s: &str) -> VenueCredentials {
        VenueCredentials {
            venue: "tradovate".into(),
            plaintext: s.as_bytes().to_vec(),
        }
    }

    #[test]
    fn the_credential_blob_parses_with_defaults_and_refuses_the_malformed() {
        let c = TradovateAccountSource::parse_creds(&creds("alice:pw")).unwrap();
        assert_eq!((c.username.as_str(), c.password.as_str()), ("alice", "pw"));
        assert_eq!(c.app_id, "trading-platform");
        let c = TradovateAccountSource::parse_creds(&creds("alice:pw:myapp:123:secret")).unwrap();
        assert_eq!(c.app_id, "myapp");
        assert_eq!((c.cid.as_deref(), c.sec.as_deref()), (Some("123"), Some("secret")));
        for bad in ["", "alice", ":pw", "alice:"] {
            assert!(
                matches!(
                    TradovateAccountSource::parse_creds(&creds(bad)),
                    Err(AccountSourceError::Credentials(_))
                ),
                "{bad:?}"
            );
        }
        assert!(!format!("{c:?}").contains("secret"), "Debug must not print credentials");
    }

    #[test]
    fn a_throttled_login_is_an_error_not_an_empty_token() {
        let now = Utc::now();
        assert!(parse_token_response(&json!({"p-ticket": "t", "p-time": 30}), now).is_err());
        assert!(parse_token_response(&json!({"errorText": "Incorrect username or password"}), now).is_err());
        assert!(parse_token_response(&json!({}), now).is_err());
        let (t, exp) = parse_token_response(
            &json!({"accessToken": "abc", "expirationTime": "2030-01-01T00:00:00Z"}),
            now,
        )
        .unwrap();
        assert_eq!(t, "abc");
        assert_eq!(exp.to_rfc3339(), "2030-01-01T00:00:00+00:00");
        let (_, exp) = parse_token_response(&json!({"accessToken": "abc"}), now).unwrap();
        assert_eq!(exp, now + Duration::minutes(60));
    }

    #[test]
    fn a_cash_snapshot_splits_margin_from_available() {
        let b = parse_cash_snapshot(&json!({
            "totalCashValue": 10000.0, "initialMargin": 2500.0, "openPnL": 120.5
        }))
        .unwrap();
        assert_eq!(b.available.inner(), Decimal::from(7500));
        assert_eq!(b.locked.inner(), Decimal::from(2500));
        assert_eq!(b.usd_value.unwrap().inner(), Decimal::from_str("10120.5").unwrap());
        assert!(parse_cash_snapshot(&json!({})).is_err());
    }

    #[test]
    fn closed_position_rows_are_dropped_and_malformed_ones_are_errors() {
        let rows = parse_positions(&json!([
            {"contractId": 11, "netPos": 2, "netPrice": 5000.25},
            {"contractId": 12, "netPos": 0, "netPrice": 0},
            {"contractId": 13, "netPos": -1, "netPrice": 70.5},
        ]))
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].net_pos, Decimal::from(-1));
        assert!(parse_positions(&json!([{"contractId": 1, "netPos": 1}])).is_err());
        assert!(parse_positions(&json!({"x": 1})).is_err());
    }

    #[test]
    fn fills_map_to_transactions_and_an_unreadable_action_is_refused() {
        let mut syms = HashMap::new();
        syms.insert(11, "ESM6".to_owned());
        let body = json!([
            {"id": 1, "contractId": 11, "action": "Buy", "qty": 2, "price": 5000.5,
             "timestamp": "2026-09-30T14:00:00Z"},
            {"id": 2, "contractId": 11, "action": "Sell", "qty": 1, "price": 5001.0,
             "timestamp": "2026-09-30T15:00:00Z"},
        ]);
        let tx = parse_fills(&body, &syms, None).unwrap();
        assert_eq!(tx[0].transaction_type, "fill_buy");
        assert_eq!(tx[0].instrument_id.as_deref(), Some("ESM6"));
        assert_eq!(tx[0].amount, Decimal::from(10001));
        let since: DateTime<Utc> = "2026-09-30T14:30:00Z".parse().unwrap();
        assert_eq!(parse_fills(&body, &syms, Some(since)).unwrap().len(), 1);
        let bad = json!([{"id": 3, "action": "Short", "qty": 1, "price": 1,
                          "timestamp": "2026-09-30T14:00:00Z"}]);
        assert!(parse_fills(&bad, &syms, None).is_err());
    }
}
