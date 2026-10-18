//! Coinbase Advanced Trade broker (Set E 3.5) — **everything up to the signed
//! request; the live submit is `BLOCKED:HUMAN`.**
//!
//! # What exists
//!
//! * ES256 JWT signing per request ([`CoinbaseCredentials::sign_jwt`]) over the
//!   CDP key format: an EC P-256 key as PEM (`EC PRIVATE KEY` SEC1 or `PRIVATE
//!   KEY` PKCS#8), the JWT carrying `sub`, `iss = "cdp"`, `nbf`, `exp` (two
//!   minutes) and a `uri` claim of `"METHOD host/path"`, plus a random `nonce`
//!   header. Signed with `ring`; no new dependency tree.
//! * Request builders: [`build_create_order`] (market IOC, limit GTC/FOK, with
//!   `client_order_id` = the idempotency key), [`CoinbaseBroker::prepare_submit`]
//!   and [`CoinbaseBroker::prepare_cancel`] which return the fully signed
//!   [`PreparedRequest`] without sending it.
//! * Read-only calls that do run: `query_order`
//!   (`GET /orders/historical/{id}`) and `query_open_orders`, with the status
//!   mapping `OPEN→New`, `FILLED→Filled`, `CANCELLED→Cancelled`, partial by
//!   `filled_size`, and a side parsed strictly (`BUY`/`SELL` or an error).
//!
//! # What is blocked, and why
//!
//! [`Broker::submit`] and [`Broker::cancel`] build and sign the request and then
//! **refuse to send it**. This is the real-money path (locked decision 8): it
//! needs a live API key and a `/security-review` of key handling before it is
//! enabled, and no key exists in this build. Migration 0121 independently pins
//! `orders.execution_mode = 'paper'`, so an order routed here could not be
//! journalled either. Enabling the send is a code change in this file's two
//! `blocked_*` functions plus that review — not a flag.
//!
//! Private keys are held as a parsed `ring` key pair; `Debug` prints nothing of
//! them, and they are never logged.

use std::str::FromStr;

use async_trait::async_trait;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use domain::money::Price;
use domain::order::{OrderIntent, OrderType, Side, TimeInForce};
use reqwest::Client;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use risk::ApprovedOrder;
use rust_decimal::Decimal;
use serde_json::{json, Value};

use crate::broker::{Broker, BrokerError, BrokerOrderState, BrokerOrderStatus, BrokerPosition};

pub const BASE_URL: &str = "https://api.coinbase.com";
pub const HOST: &str = "api.coinbase.com";
const ORDERS_PATH: &str = "/api/v3/brokerage/orders";
/// How long a signed JWT is valid. Coinbase's documented maximum is 2 minutes.
pub const JWT_TTL_SECS: i64 = 120;

const BLOCKED: &str = "live Coinbase order flow is BLOCKED:HUMAN (Set E 3.5): it needs a live API \
                       key and a security review of key handling. The request was built and \
                       signed and not sent";

// ── DER helpers (SEC1 → the PKCS#8 ring accepts) ────────────────────────────

fn der_len(n: usize) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else if n < 0x100 {
        vec![0x81, n as u8]
    } else {
        vec![0x82, (n >> 8) as u8, (n & 0xff) as u8]
    }
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    v.extend(der_len(content.len()));
    v.extend_from_slice(content);
    v
}

/// Read one TLV at the start of `b`: (tag, content, rest).
fn read_tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *b.first()?;
    let l0 = *b.get(1)? as usize;
    let (len, hdr) = if l0 < 0x80 {
        (l0, 2)
    } else {
        let n = l0 & 0x7f;
        if n == 0 || n > 2 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *b.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let content = b.get(hdr..hdr + len)?;
    Some((tag, content, &b[hdr + len..]))
}

/// A SEC1 `ECPrivateKey` → the minimal PKCS#8 (private scalar + public point)
/// that `ring` accepts. CDP keys carry an extra `[0] parameters` field ring does
/// not take, so the key is rebuilt rather than wrapped.
fn sec1_to_pkcs8(sec1: &[u8]) -> Result<Vec<u8>, String> {
    let (tag, body, _) = read_tlv(sec1).ok_or("SEC1 key is not DER")?;
    if tag != 0x30 {
        return Err("SEC1 key is not a SEQUENCE".into());
    }
    let (t, version, rest) = read_tlv(body).ok_or("SEC1 key has no version")?;
    if t != 0x02 || version != [1] {
        return Err("SEC1 key is not version 1".into());
    }
    let (t, scalar, mut rest) = read_tlv(rest).ok_or("SEC1 key has no private scalar")?;
    if t != 0x04 || scalar.len() != 32 {
        return Err("only 32-byte (P-256) private keys are supported".into());
    }
    let mut public: Option<Vec<u8>> = None;
    while let Some((t, content, r)) = read_tlv(rest) {
        if t == 0xa1 {
            let (bt, bits, _) = read_tlv(content).ok_or("bad public key field")?;
            if bt != 0x03 || bits.first() != Some(&0) {
                return Err("bad public key bit string".into());
            }
            public = Some(bits[1..].to_vec());
        }
        rest = r;
    }
    let public = public.ok_or("SEC1 key carries no public point; ring cannot derive one")?;
    let mut ec_private = Vec::new();
    ec_private.extend(tlv(0x02, &[1]));
    ec_private.extend(tlv(0x04, scalar));
    let mut bits = vec![0u8];
    bits.extend(&public);
    ec_private.extend(tlv(0xa1, &tlv(0x03, &bits)));
    let ec_private = tlv(0x30, &ec_private);

    let oid_ec = tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]);
    let oid_p256 = tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]);
    let mut alg = oid_ec;
    alg.extend(oid_p256);
    let mut pkcs8 = tlv(0x02, &[0]);
    pkcs8.extend(tlv(0x30, &alg));
    pkcs8.extend(tlv(0x04, &ec_private));
    Ok(tlv(0x30, &pkcs8))
}

/// Decode a PEM block, returning (label, DER).
fn decode_pem(pem: &str) -> Result<(String, Vec<u8>), String> {
    // CDP keys downloaded as JSON carry literal `\n` escapes.
    let pem = pem.replace("\\n", "\n");
    let begin = pem.find("-----BEGIN ").ok_or("no PEM header")?;
    let after = &pem[begin + 11..];
    let label_end = after.find("-----").ok_or("unterminated PEM header")?;
    let label = after[..label_end].to_owned();
    let body_start = begin + 11 + label_end + 5;
    let end = pem.find("-----END ").ok_or("no PEM footer")?;
    let b64: String = pem[body_start..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let der = STANDARD
        .decode(b64)
        .map_err(|e| format!("PEM body is not base64: {e}"))?;
    Ok((label, der))
}

// ── Credentials and JWT ──────────────────────────────────────────────────────

/// A CDP API key: its name and its EC P-256 signing key.
pub struct CoinbaseCredentials {
    key_name: String,
    key_pair: EcdsaKeyPair,
}

impl std::fmt::Debug for CoinbaseCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CoinbaseCredentials([REDACTED])")
    }
}

impl CoinbaseCredentials {
    /// Parse a CDP key from its name and PEM private key.
    ///
    /// # Errors
    /// A malformed or non-P-256 key. The message never includes key material.
    pub fn from_pem(key_name: impl Into<String>, pem: &str) -> Result<Self, BrokerError> {
        let bad = |m: String| BrokerError::Rejected(format!("coinbase private key rejected: {m}"));
        let (label, der) = decode_pem(pem).map_err(bad)?;
        let pkcs8 = match label.as_str() {
            "EC PRIVATE KEY" => sec1_to_pkcs8(&der).map_err(bad)?,
            "PRIVATE KEY" => der,
            other => return Err(bad(format!("unsupported PEM label {other:?}"))),
        };
        let key_pair = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &pkcs8,
            &SystemRandom::new(),
        )
        .map_err(|e| bad(format!("not a usable P-256 key ({e})")))?;
        let key_name = key_name.into();
        if key_name.is_empty() {
            return Err(bad("the key name is empty".into()));
        }
        Ok(Self { key_name, key_pair })
    }

    /// The public point (uncompressed SEC1), for verifying signatures in tests.
    #[must_use]
    pub fn public_key(&self) -> Vec<u8> {
        self.key_pair.public_key().as_ref().to_vec()
    }

    /// A fresh 16-byte hex nonce.
    fn nonce() -> Result<String, BrokerError> {
        let mut b = [0u8; 16];
        SystemRandom::new()
            .fill(&mut b)
            .map_err(|_| BrokerError::Rejected("the system RNG failed".into()))?;
        Ok(b.iter().map(|x| format!("{x:02x}")).collect())
    }

    /// Sign the per-request JWT.
    ///
    /// `uri` is `"METHOD host/path"` with **no scheme and no query string** —
    /// exactly what Coinbase checks the token against, so a token for one
    /// endpoint cannot be replayed on another.
    ///
    /// # Errors
    /// An RNG or signing failure.
    pub fn sign_jwt(
        &self,
        method: &str,
        host: &str,
        path: &str,
        now_unix: i64,
        nonce: &str,
    ) -> Result<String, BrokerError> {
        let header = json!({"alg": "ES256", "typ": "JWT", "kid": self.key_name, "nonce": nonce});
        let claims = json!({
            "sub": self.key_name,
            "iss": "cdp",
            "nbf": now_unix,
            "exp": now_unix + JWT_TTL_SECS,
            "uri": format!("{method} {host}{path}"),
        });
        let enc = |v: &Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let signing_input = format!("{}.{}", enc(&header), enc(&claims));
        let sig = self
            .key_pair
            .sign(&SystemRandom::new(), signing_input.as_bytes())
            .map_err(|_| BrokerError::Rejected("JWT signing failed".into()))?;
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(sig.as_ref())
        ))
    }

    /// [`Self::sign_jwt`] for "now" with a fresh nonce.
    pub fn sign_jwt_now(
        &self,
        method: &str,
        host: &str,
        path: &str,
    ) -> Result<String, BrokerError> {
        self.sign_jwt(
            method,
            host,
            path,
            chrono::Utc::now().timestamp(),
            &Self::nonce()?,
        )
    }
}

// ── Request builders (pure) ──────────────────────────────────────────────────

fn d(x: Decimal) -> String {
    x.normalize().to_string()
}

/// The `POST /api/v3/brokerage/orders` body for an intent.
///
/// `client_order_id` is the idempotency key, so a retried submit is the same
/// order at Coinbase. Supported: market (immediate-or-cancel), limit GTC, limit
/// FOK, limit IOC (the smart-order-routed form). Stop-type orders, `Day`
/// expiry (which needs an end time) and OCO groups are refused by name rather
/// than approximated — an approximate order is a different order.
///
/// # Errors
/// An intent Coinbase cannot express exactly.
pub fn build_create_order(intent: &OrderIntent) -> Result<Value, BrokerError> {
    let refuse = |why: &str| {
        Err(BrokerError::Rejected(format!(
            "coinbase cannot express this order exactly: {why}"
        )))
    };
    if intent.order_type.is_triggered() {
        return refuse("stop-type orders are not built here yet");
    }
    if intent.oco_group.is_some() {
        return refuse("OCO groups are not supported");
    }
    intent
        .check_shape()
        .map_err(|e| BrokerError::Rejected(e.to_string()))?;
    let size = d(intent.size.inner());
    let configuration = match intent.order_type {
        OrderType::Market => {
            if intent.post_only {
                return refuse("a market order cannot be post-only");
            }
            json!({ "market_market_ioc": { "base_size": size } })
        }
        OrderType::Limit => {
            let limit = intent
                .limit_price
                .ok_or_else(|| BrokerError::Rejected("a limit order needs a limit price".into()))?;
            let price = d(limit.inner());
            match intent.time_in_force {
                TimeInForce::Gtc => json!({ "limit_limit_gtc": {
                    "base_size": size, "limit_price": price, "post_only": intent.post_only } }),
                TimeInForce::Fok => json!({ "limit_limit_fok": {
                    "base_size": size, "limit_price": price } }),
                TimeInForce::Ioc => json!({ "sor_limit_ioc": {
                    "base_size": size, "limit_price": price } }),
                TimeInForce::Day => return refuse("Day expiry needs an end time (not built)"),
            }
        }
        _ => return refuse("unsupported order type"),
    };
    Ok(json!({
        "client_order_id": intent.idempotency_key.to_string(),
        "product_id": intent.instrument_id,
        "side": match intent.side { Side::Buy => "BUY", Side::Sell => "SELL" },
        "order_configuration": configuration,
    }))
}

/// A fully built, signed request that has **not** been sent.
#[derive(Debug, Clone)]
pub struct PreparedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl PreparedRequest {
    /// The bearer JWT, for inspection in tests.
    #[must_use]
    pub fn bearer(&self) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .and_then(|(_, v)| v.strip_prefix("Bearer "))
    }
}

// ── Response parsing (pure) ─────────────────────────────────────────────────

/// Coinbase order `status` → [`BrokerOrderState`]. A partial fill is decided by
/// the caller from `filled_size`.
#[must_use]
pub fn map_status(s: &str) -> BrokerOrderState {
    match s {
        "OPEN" | "PENDING" | "QUEUED" | "CANCEL_QUEUED" | "EDIT_QUEUED" => BrokerOrderState::New,
        "FILLED" => BrokerOrderState::Filled,
        "CANCELLED" | "EXPIRED" => BrokerOrderState::Cancelled,
        "FAILED" => BrokerOrderState::Rejected,
        other => BrokerOrderState::Unknown(other.to_owned()),
    }
}

fn dec(v: &Value) -> Option<Decimal> {
    v.as_str().and_then(|s| Decimal::from_str(s).ok())
}

/// One Coinbase order object → a status.
///
/// The side must be exactly `BUY` or `SELL` (Set E audit #9) and the order type
/// one of `MARKET`/`LIMIT`/`STOP_LIMIT`. The submitted size is read from the
/// order configuration's `base_size`; an order sized in quote currency cannot be
/// reported honestly and is an error.
///
/// # Errors
/// Anything unreadable.
pub fn parse_order(o: &Value) -> Result<BrokerOrderStatus, BrokerError> {
    let bad = |m: String| BrokerError::Serialization(format!("coinbase order: {m}"));
    let id = o["order_id"]
        .as_str()
        .ok_or_else(|| bad("no order_id".into()))?;
    let side = match o["side"].as_str() {
        Some("BUY") => Side::Buy,
        Some("SELL") => Side::Sell,
        other => {
            return Err(bad(format!(
                "side {other:?}; refusing to guess a direction"
            )))
        }
    };
    let order_type = match o["order_type"].as_str() {
        Some("MARKET") => OrderType::Market,
        Some("LIMIT") => OrderType::Limit,
        Some("STOP_LIMIT") | Some("STOP") => OrderType::StopLimit,
        other => return Err(bad(format!("order_type {other:?}"))),
    };
    let submitted = o["order_configuration"]
        .as_object()
        .and_then(|m| m.values().next())
        .and_then(|cfg| dec(&cfg["base_size"]))
        .ok_or_else(|| bad("no base_size in the order configuration".into()))?;
    let filled = dec(&o["filled_size"]).unwrap_or(Decimal::ZERO);
    let avg = dec(&o["average_filled_price"])
        .filter(|p| *p > Decimal::ZERO)
        .map(Price::from_decimal);
    let mut state = map_status(o["status"].as_str().unwrap_or(""));
    if filled > Decimal::ZERO && filled < submitted && state == BrokerOrderState::New {
        state = BrokerOrderState::PartiallyFilled;
    }
    Ok(BrokerOrderStatus {
        broker_order_id: id.to_owned(),
        instrument_id: o["product_id"].as_str().unwrap_or_default().to_owned(),
        side,
        order_type,
        submitted_qty: submitted,
        filled_qty: filled,
        avg_fill_price: avg,
        // Coinbase reports `total_fees`; absent stays `None`, never zero.
        commission: dec(&o["total_fees"]),
        state,
    })
}

// ── The broker ───────────────────────────────────────────────────────────────

pub struct CoinbaseBroker {
    client: Client,
    creds: CoinbaseCredentials,
    pub base_url: String,
    host: String,
}

impl CoinbaseBroker {
    pub fn new(creds: CoinbaseCredentials) -> Self {
        Self {
            client: Client::new(),
            creds,
            base_url: BASE_URL.to_owned(),
            host: HOST.to_owned(),
        }
    }

    /// Point at another root (a test server). The `uri` claim is built from the
    /// URL's host, so a token minted for the test server is not valid on
    /// `api.coinbase.com`.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.host = url
            .split("://")
            .nth(1)
            .unwrap_or(&url)
            .trim_end_matches('/')
            .to_owned();
        self.base_url = url;
        self
    }

    /// Build and sign a request without sending it.
    ///
    /// # Errors
    /// A signing failure.
    pub fn prepare(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<PreparedRequest, BrokerError> {
        let jwt = self.creds.sign_jwt_now(method, &self.host, path)?;
        let mut url = format!("{}{path}", self.base_url);
        if !query.is_empty() {
            let qs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
            url = format!("{url}?{}", qs.join("&"));
        }
        let mut headers = vec![("Authorization".to_owned(), format!("Bearer {jwt}"))];
        if body.is_some() {
            headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
        }
        Ok(PreparedRequest {
            method: method.to_owned(),
            url,
            headers,
            body: body.map(Value::to_string),
        })
    }

    /// The signed create-order request for `order`, **not sent**.
    ///
    /// # Errors
    /// An order Coinbase cannot express, or a signing failure.
    pub fn prepare_submit(&self, order: &ApprovedOrder) -> Result<PreparedRequest, BrokerError> {
        let body = build_create_order(&order.intent)?;
        self.prepare("POST", ORDERS_PATH, &[], Some(&body))
    }

    /// The signed batch-cancel request for one order, **not sent**.
    ///
    /// # Errors
    /// A signing failure.
    pub fn prepare_cancel(&self, broker_order_id: &str) -> Result<PreparedRequest, BrokerError> {
        let body = json!({ "order_ids": [broker_order_id] });
        self.prepare(
            "POST",
            "/api/v3/brokerage/orders/batch_cancel",
            &[],
            Some(&body),
        )
    }

    async fn send_get(
        &self,
        req: PreparedRequest,
    ) -> Result<(reqwest::StatusCode, Value), BrokerError> {
        let mut r = self.client.get(&req.url);
        for (k, v) in &req.headers {
            r = r.header(k, v);
        }
        let resp = r
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .json::<Value>()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;
        Ok((status, body))
    }
}

#[async_trait]
impl Broker for CoinbaseBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        // Built and signed so a malformed order fails here exactly as it would
        // live; never sent (BLOCKED:HUMAN).
        let _signed = self.prepare_submit(order)?;
        Err(BrokerError::Rejected(BLOCKED.to_owned()))
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        let _signed = self.prepare_cancel(broker_order_id)?;
        Err(BrokerError::Rejected(BLOCKED.to_owned()))
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        let path = format!("/api/v3/brokerage/orders/historical/{broker_order_id}");
        let (status, body) = self
            .send_get(self.prepare("GET", &path, &[], None)?)
            .await?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(BrokerError::OrderNotFound(broker_order_id.to_owned()));
        }
        if !status.is_success() {
            return Err(BrokerError::Rejected(body.to_string()));
        }
        parse_order(&body["order"])
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        let (status, body) = self
            .send_get(self.prepare(
                "GET",
                "/api/v3/brokerage/orders/historical/batch",
                &[("order_status", "OPEN")],
                None,
            )?)
            .await?;
        if !status.is_success() {
            return Err(BrokerError::Http(body.to_string()));
        }
        // One unreadable order fails the listing: dropping it would hide an
        // open order from reconciliation.
        body["orders"]
            .as_array()
            .map(|a| a.iter().map(parse_order).collect())
            .unwrap_or_else(|| Ok(vec![]))
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        Err(BrokerError::Serialization(
            "coinbase spot has no position record to read; balances come from the \
             AccountSource, and an average entry price is not available from Coinbase"
                .to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::Size;
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

    /// A throwaway key generated for the test, as a PKCS#8 PEM. Nothing is
    /// committed: the key exists only for the life of the test.
    fn test_key() -> (String, Vec<u8>) {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            STANDARD.encode(pkcs8.as_ref())
        );
        (pem, pkcs8.as_ref().to_vec())
    }

    fn verify(jwt: &str, public: &[u8]) -> (Value, Value) {
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert_eq!(sig.len(), 64, "ES256 is the raw 64-byte r||s");
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public)
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .expect("the signature must verify under the key's public point");
        let dec = |p: &str| -> Value {
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(p).unwrap()).unwrap()
        };
        (dec(parts[0]), dec(parts[1]))
    }

    #[test]
    fn a_jwt_carries_the_cdp_claims_and_verifies() {
        let (pem, _) = test_key();
        let creds = CoinbaseCredentials::from_pem("organizations/o/apiKeys/k", &pem).unwrap();
        let jwt = creds
            .sign_jwt(
                "GET",
                "api.coinbase.com",
                "/api/v3/brokerage/accounts",
                1_780_000_000,
                "abc123",
            )
            .unwrap();
        let (h, c) = verify(&jwt, &creds.public_key());
        assert_eq!(h["alg"], "ES256");
        assert_eq!(h["typ"], "JWT");
        assert_eq!(h["kid"], "organizations/o/apiKeys/k");
        assert_eq!(h["nonce"], "abc123");
        assert_eq!(c["iss"], "cdp");
        assert_eq!(c["sub"], "organizations/o/apiKeys/k");
        assert_eq!(c["nbf"], 1_780_000_000);
        assert_eq!(c["exp"], 1_780_000_000 + JWT_TTL_SECS);
        assert_eq!(c["uri"], "GET api.coinbase.com/api/v3/brokerage/accounts");
    }

    #[test]
    fn a_tampered_token_does_not_verify() {
        let (pem, _) = test_key();
        let creds = CoinbaseCredentials::from_pem("k", &pem).unwrap();
        let jwt = creds.sign_jwt("GET", HOST, "/a", 1, "n").unwrap();
        let mut parts: Vec<String> = jwt.split('.').map(str::to_owned).collect();
        parts[1] =
            URL_SAFE_NO_PAD.encode(r#"{"uri":"POST api.coinbase.com/api/v3/brokerage/orders"}"#);
        let sig = URL_SAFE_NO_PAD.decode(&parts[2]).unwrap();
        assert!(
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, creds.public_key())
                .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
                .is_err()
        );
    }

    #[test]
    fn an_sec1_cdp_key_with_parameters_loads_and_signs_for_the_same_public_key() {
        // Build a CDP-shaped SEC1 key (with the [0] parameters field ring does
        // not accept) from a generated key, then load it through the PEM path.
        let (_, pkcs8) = test_key();
        let (_, body, _) = read_tlv(&pkcs8).unwrap();
        let (_, _, rest) = read_tlv(body).unwrap(); // version
        let (_, _, rest) = read_tlv(rest).unwrap(); // algorithm
        let (_, octets, _) = read_tlv(rest).unwrap(); // OCTET STRING { ECPrivateKey }
        let (_, ec_body, _) = read_tlv(octets).unwrap();
        let (_, _, rest) = read_tlv(ec_body).unwrap(); // version 1
        let (_, scalar, rest) = read_tlv(rest).unwrap();
        let public_field = rest.to_vec(); // [1] publicKey
        let mut sec1_body = tlv(0x02, &[1]);
        sec1_body.extend(tlv(0x04, scalar));
        sec1_body.extend(tlv(
            0xa0,
            &tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
        ));
        sec1_body.extend(public_field);
        let sec1 = tlv(0x30, &sec1_body);
        let pem = format!(
            "-----BEGIN EC PRIVATE KEY-----\n{}\n-----END EC PRIVATE KEY-----\n",
            STANDARD.encode(&sec1)
        );
        let creds = CoinbaseCredentials::from_pem("k", &pem).expect("a CDP SEC1 key loads");
        let jwt = creds.sign_jwt("GET", HOST, "/x", 5, "n").unwrap();
        verify(&jwt, &creds.public_key());
        // And literal "\n" escapes, as in the JSON the key is downloaded in.
        let escaped = pem.replace('\n', "\\n");
        assert!(CoinbaseCredentials::from_pem("k", &escaped).is_ok());
    }

    #[test]
    fn bad_keys_are_refused_without_echoing_them() {
        for bad in [
            "",
            "not a pem",
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----",
        ] {
            assert!(CoinbaseCredentials::from_pem("k", bad).is_err());
        }
        let (pem, _) = test_key();
        assert!(
            CoinbaseCredentials::from_pem("", &pem).is_err(),
            "empty key name"
        );
        let err = CoinbaseCredentials::from_pem(
            "k",
            "-----BEGIN PRIVATE KEY-----\nQUJD\n-----END PRIVATE KEY-----",
        )
        .unwrap_err()
        .to_string();
        assert!(!err.contains("QUJD"), "{err}");
        let (pem, _) = test_key();
        let c = CoinbaseCredentials::from_pem("k", &pem).unwrap();
        assert_eq!(format!("{c:?}"), "CoinbaseCredentials([REDACTED])");
    }

    fn intent(t: OrderType, side: Side, limit: Option<i64>, tif: TimeInForce) -> OrderIntent {
        let mut i = OrderIntent::new(
            "BTC-USD",
            side,
            t,
            Size::from_decimal(Decimal::new(15, 1)),
            limit.map(|p| Price::from_decimal(Decimal::from(p))),
            None,
        );
        i.time_in_force = tif;
        i
    }

    #[test]
    fn order_bodies_match_coinbases_shapes_and_carry_the_idempotency_key() {
        let m = intent(OrderType::Market, Side::Buy, None, TimeInForce::Gtc);
        let b = build_create_order(&m).unwrap();
        assert_eq!(b["client_order_id"], m.idempotency_key.to_string());
        assert_eq!(b["product_id"], "BTC-USD");
        assert_eq!(b["side"], "BUY");
        assert_eq!(
            b["order_configuration"]["market_market_ioc"]["base_size"],
            "1.5"
        );

        let l = intent(OrderType::Limit, Side::Sell, Some(60000), TimeInForce::Gtc);
        let b = build_create_order(&l).unwrap();
        assert_eq!(b["side"], "SELL");
        assert_eq!(
            b["order_configuration"]["limit_limit_gtc"]["limit_price"],
            "60000"
        );
        assert_eq!(
            b["order_configuration"]["limit_limit_gtc"]["post_only"],
            false
        );

        let f = intent(OrderType::Limit, Side::Buy, Some(59000), TimeInForce::Fok);
        assert!(
            build_create_order(&f).unwrap()["order_configuration"]["limit_limit_fok"].is_object()
        );
    }

    #[test]
    fn what_coinbase_cannot_express_is_refused_by_name() {
        let stop = {
            let mut i = intent(OrderType::Stop, Side::Sell, None, TimeInForce::Gtc);
            i.stop_price = Some(Price::from_decimal(Decimal::from(50000)));
            i
        };
        assert!(build_create_order(&stop).is_err());
        assert!(build_create_order(&intent(
            OrderType::Limit,
            Side::Buy,
            Some(1),
            TimeInForce::Day
        ))
        .is_err());
        assert!(
            build_create_order(&intent(OrderType::Limit, Side::Buy, None, TimeInForce::Gtc))
                .is_err(),
            "no limit price"
        );
        let mut oco = intent(OrderType::Limit, Side::Buy, Some(1), TimeInForce::Gtc);
        oco.oco_group = Some("g".into());
        assert!(build_create_order(&oco).is_err());
    }

    #[test]
    fn order_status_mapping_and_strict_side_parsing() {
        assert_eq!(map_status("OPEN"), BrokerOrderState::New);
        assert_eq!(map_status("FILLED"), BrokerOrderState::Filled);
        assert_eq!(map_status("CANCELLED"), BrokerOrderState::Cancelled);
        assert_eq!(map_status("FAILED"), BrokerOrderState::Rejected);
        assert!(matches!(map_status("???"), BrokerOrderState::Unknown(_)));

        let mut o = json!({
            "order_id": "abc", "product_id": "BTC-USD", "side": "BUY", "order_type": "LIMIT",
            "status": "OPEN", "filled_size": "0.5", "average_filled_price": "60000.1",
            "total_fees": "1.25",
            "order_configuration": {"limit_limit_gtc": {"base_size": "1.5", "limit_price": "60000"}}
        });
        let s = parse_order(&o).unwrap();
        assert_eq!(
            s.state,
            BrokerOrderState::PartiallyFilled,
            "partial is decided by filled_size"
        );
        assert_eq!(s.submitted_qty, Decimal::new(15, 1));
        assert_eq!(s.commission, Some(Decimal::new(125, 2)));
        for bad in ["", "buy", "LONG"] {
            o["side"] = json!(bad);
            assert!(parse_order(&o).is_err(), "side {bad:?} must be refused");
        }
        o["side"] = json!("SELL");
        o.as_object_mut().unwrap().remove("total_fees");
        assert_eq!(
            parse_order(&o).unwrap().commission,
            None,
            "absent fees are None, not zero"
        );
    }
}
