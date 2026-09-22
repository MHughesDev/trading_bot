//! GET /ws/live — WebSocket upgrade for live data streaming.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tracing::{debug, error};
use ui_gateway::{
    transport::{ClientMessage, WsOutMessage},
    SubscriptionRegistry,
};

use crate::state::AppState;

#[derive(Deserialize)]
pub struct WsQuery {
    /// Bearer token passed as a query parameter (browser WS API doesn't support headers).
    token: Option<String>,
}

/// GET /ws/live — WebSocket upgrade for live data streaming.
pub async fn ws_live(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Response {
    let user_id = match query.token.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => t.to_owned(),
        None => return (StatusCode::UNAUTHORIZED, "missing token query param").into_response(),
    };
    let live = state.live.clone();
    ws.on_upgrade(move |socket| handle_ws(socket, state.gateway, live, user_id))
}

async fn handle_ws(
    mut socket: WebSocket,
    gateway: Arc<SubscriptionRegistry>,
    live: crate::live_bus::LiveSender,
    user_id: String,
) {
    // Subscribe before the first client message so no frame published during
    // the handshake is missed.
    let mut frames = live.subscribe();

    // Panels opened by THIS socket. The subscription registry is keyed by user
    // because that is the right scope for authorization — but two tabs share a
    // user, and a tab must not receive frames the other tab asked for.
    let mut my_panels: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Send initial heartbeat to confirm connection.
    if let Some(m) = json_msg(&WsOutMessage::Heartbeat { ts: now_iso() }) {
        if socket.send(m).await.is_err() {
            return;
        }
    }

    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
    heartbeat.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_client_message(&mut socket, &gateway, &user_id, &text, &mut my_panels).await;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }

            // Live data. One copy per socket; this socket forwards only the
            // frames its own panels asked for.
            frame = frames.recv() => {
                match frame {
                    Ok(f) => {
                        if !forward_frame(&mut socket, &gateway, &user_id, &my_panels, &f).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        // A slow socket skips ahead rather than replaying: a
                        // late price is worse than no price.
                        debug!(user_id, skipped, "ws lagged, skipping stale frames");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            _ = heartbeat.tick() => {
                if let Some(m) = json_msg(&WsOutMessage::Heartbeat { ts: now_iso() }) {
                    if socket.send(m).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    // Remove only this socket's panels. Another tab on the same account keeps
    // its own subscriptions when this one closes.
    for panel in &my_panels {
        gateway.remove_panel(panel, &user_id);
    }
    debug!(user_id, panels = my_panels.len(), "ws connection closed");
}

/// Render one outgoing `frame` message for a single subscription, reusing the
/// body the bus already encoded (Set C #14, #15).
///
/// The only part of the message that varies between subscriptions is `sub_id` —
/// `lane` and `instrument` are, by the filter in [`forward_frame`], necessarily
/// equal to the frame's own. So the whole message is `{"type":"frame","sub_id":"`
/// + the uuid + the shared body. No clone of the payload, no second encode.
///
/// `ws_frame_matches_the_typed_encoding` proves this produces byte-for-byte what
/// `serde_json::to_vec(&WsOutMessage::Frame { .. })` would have produced.
fn render_frame(body: &str, sub_id: uuid::Uuid) -> Message {
    const HEAD: &str = r#"{"type":"frame","sub_id":""#;
    let mut s = String::with_capacity(HEAD.len() + 37 + body.len());
    s.push_str(HEAD);
    let mut buf = uuid::Uuid::encode_buffer();
    s.push_str(sub_id.hyphenated().encode_lower(&mut buf));
    s.push('"');
    s.push(',');
    s.push_str(body);
    Message::Text(s.into())
}

/// Select the subscriptions on this socket that want `frame` and render one
/// outgoing message for each, reusing the single encoded body.
///
/// Split out from [`forward_frame`] so the fan-out can be tested without a live
/// TCP socket: `fan_out_to_many_subscriptions_costs_one_encode` drives this with
/// a real [`SubscriptionRegistry`] and counts
/// [`crate::live_bus::PAYLOAD_ENCODES`].
fn render_frames(
    gateway: &SubscriptionRegistry,
    user_id: &str,
    my_panels: &std::collections::HashSet<String>,
    frame: &crate::live_bus::LiveFrame,
) -> Vec<Message> {
    // Encoded once, at publish time, for the whole platform. Every subscription
    // below appends this same buffer rather than re-serialising it.
    let body = frame.body();
    gateway
        .list_for_user(user_id)
        .into_iter()
        .filter(|sub| my_panels.contains(&sub.panel_id))
        .filter(|sub| {
            sub.lane.as_str() == &*frame.lane && sub.instrument.as_str() == &*frame.instrument
        })
        .map(|sub| render_frame(body, sub.id))
        .collect()
}

/// Forward one live frame to every subscription on this socket that wants it.
///
/// Returns `false` when the socket has gone away, which ends the connection.
async fn forward_frame(
    socket: &mut WebSocket,
    gateway: &SubscriptionRegistry,
    user_id: &str,
    my_panels: &std::collections::HashSet<String>,
    frame: &crate::live_bus::LiveFrame,
) -> bool {
    for m in render_frames(gateway, user_id, my_panels, frame) {
        if socket.send(m).await.is_err() {
            return false;
        }
    }
    true
}

async fn handle_client_message(
    socket: &mut WebSocket,
    gateway: &SubscriptionRegistry,
    user_id: &str,
    text: &str,
    my_panels: &mut std::collections::HashSet<String>,
) {
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            if let Some(m) = json_msg(&WsOutMessage::Error {
                code: "parse_error".to_owned(),
                message: e.to_string(),
            }) {
                let _ = socket.send(m).await;
            }
            return;
        }
    };

    if msg.unsubscribe {
        gateway.remove_panel(&msg.panel_id, user_id);
        my_panels.remove(&msg.panel_id);
        return;
    }

    for spec in &msg.subscribe {
        match gateway.subscribe(
            &msg.panel_id,
            user_id,
            &spec.lane,
            &spec.instrument,
            user_id,
            spec.depth,
            spec.max_fps,
        ) {
            Ok(sub) => {
                my_panels.insert(sub.panel_id.clone());
                if let Some(m) = json_msg(&WsOutMessage::Subscribed {
                    sub_id: sub.id,
                    panel_id: sub.panel_id.clone(),
                    lane: sub.lane.clone(),
                    instrument: sub.instrument.clone(),
                }) {
                    let _ = socket.send(m).await;
                }
            }
            Err(e) => {
                if let Some(m) = json_msg(&WsOutMessage::Error {
                    code: "subscription_error".to_owned(),
                    message: e.to_string(),
                }) {
                    let _ = socket.send(m).await;
                }
            }
        }
    }
}

/// Serialize a WS message to a text frame, returning `None` on failure.
/// Logs and drops the frame rather than sending an empty text frame.
fn json_msg(msg: &WsOutMessage) -> Option<Message> {
    match serde_json::to_vec(msg) {
        Ok(bytes) => {
            // SAFETY: serde_json always produces valid UTF-8.
            let s = unsafe { String::from_utf8_unchecked(bytes) };
            Some(Message::Text(s.into()))
        }
        Err(e) => {
            error!(error = %e, "ws: serialization failed; dropping frame");
            None
        }
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use demand_manager::{DemandRegistry, NoopPipelineFactory};
    use uuid::Uuid;

    use super::*;
    use crate::live_bus;

    fn registry() -> SubscriptionRegistry {
        SubscriptionRegistry::new(Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory))))
    }

    fn text(m: &Message) -> String {
        match m {
            Message::Text(t) => t.to_string(),
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    /// The shared-body rendering must produce exactly what the typed enum would
    /// have produced. Without this, "encode once" would be a licence to drift
    /// from the wire contract the React client parses.
    #[test]
    fn ws_frame_matches_the_typed_encoding() {
        let _guard = live_bus::encode_counter_guard();
        let sub_id = Uuid::parse_str("2f5a1c0e-9b3d-4a77-8e61-0c2d5f4a9b10").unwrap();
        let payload = serde_json::json!({
            "ts": 1_726_000_000_000_i64,
            "price": "63145.27",
            "side": "buy",
            "nested": { "a": [1, 2, {"b": null}] },
            "unicode": "naïve \"quoted\" \\ back",
        });

        let expected = serde_json::to_string(&WsOutMessage::Frame {
            sub_id,
            lane: "market.trades".to_owned(),
            instrument: "BTC-USD".to_owned(),
            payload: payload.clone(),
        })
        .unwrap();

        let tx = live_bus::channel();
        let mut rx = tx.subscribe();
        live_bus::publish(&tx, "market.trades", "BTC-USD", payload);
        let frame = rx.try_recv().unwrap();

        assert_eq!(text(&render_frame(frame.body(), sub_id)), expected);
    }

    /// Set C #14 + #15. Fanning one frame out to many subscriptions must cost
    /// **one** JSON encode in total, not one per subscription.
    #[test]
    fn fan_out_to_many_subscriptions_costs_one_encode() {
        let _guard = live_bus::encode_counter_guard();

        let gateway = registry();
        let mut panels = HashSet::new();
        const SUBSCRIBERS: usize = 64;
        for i in 0..SUBSCRIBERS {
            let panel = format!("panel-{i}");
            let sub = gateway
                .subscribe(&panel, "alice", "market.trades", "BTC-USD", "alice", None, None)
                .expect("subscribe");
            panels.insert(sub.panel_id.clone());
        }

        let tx = live_bus::channel();
        let mut rx = tx.subscribe();

        let before = live_bus::PAYLOAD_ENCODES.load(Ordering::Relaxed);
        live_bus::publish(&tx, "market.trades", "BTC-USD", serde_json::json!({"p": "1"}));
        let frame = rx.try_recv().expect("frame delivered");
        let msgs = render_frames(&gateway, "alice", &panels, &frame);
        let after = live_bus::PAYLOAD_ENCODES.load(Ordering::Relaxed);

        assert_eq!(msgs.len(), SUBSCRIBERS, "every subscription must be served");
        assert_eq!(
            after - before,
            1,
            "{SUBSCRIBERS} subscribers must cost 1 encode, not {SUBSCRIBERS}"
        );

        // Distinct sub_ids, identical payload bytes — proof the bodies are shared
        // rather than independently produced.
        let ids: HashSet<String> = msgs
            .iter()
            .map(|m| text(m).split("\"sub_id\":\"").nth(1).unwrap()[..36].to_owned())
            .collect();
        assert_eq!(ids.len(), SUBSCRIBERS, "sub_id must still vary per subscription");
        let suffixes: HashSet<String> = msgs
            .iter()
            .map(|m| text(m).split_once("\",\"lane\"").unwrap().1.to_owned())
            .collect();
        assert_eq!(suffixes.len(), 1, "the body must be identical for all");
    }

    /// The filter is load-bearing: a socket must not receive another panel's
    /// lane, another instrument, or another tab's panel.
    #[test]
    fn fan_out_refuses_frames_this_socket_did_not_ask_for() {
        let _guard = live_bus::encode_counter_guard();
        let gateway = registry();
        let mine = gateway
            .subscribe("mine", "alice", "market.trades", "BTC-USD", "alice", None, None)
            .unwrap();
        // Same user, different tab — not in `my_panels`.
        gateway
            .subscribe("other-tab", "alice", "market.trades", "BTC-USD", "alice", None, None)
            .unwrap();
        // Same panel, different instrument.
        gateway
            .subscribe("mine", "alice", "market.trades", "ETH-USD", "alice", None, None)
            .unwrap();
        let panels: HashSet<String> = std::iter::once(mine.panel_id.clone()).collect();

        let tx = live_bus::channel();
        let mut rx = tx.subscribe();
        live_bus::publish(&tx, "market.trades", "BTC-USD", serde_json::json!(1));
        let frame = rx.try_recv().unwrap();

        let msgs = render_frames(&gateway, "alice", &panels, &frame);
        assert_eq!(msgs.len(), 1, "only this socket's matching subscription");
        assert!(text(&msgs[0]).contains(&mine.id.to_string()));

        // A frame on a lane nobody on this socket wants produces nothing at all.
        live_bus::publish(&tx, "market.bars.1m", "BTC-USD", serde_json::json!(1));
        let other = rx.try_recv().unwrap();
        assert!(render_frames(&gateway, "alice", &panels, &other).is_empty());
    }

    /// Item 1's measurement, kept runnable: `cargo test -p api --lib
    /// ws::live::tests::bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark; run explicitly with --nocapture"]
    fn bench_fan_out_encode_once_vs_encode_per_subscriber() {
        use std::time::Instant;

        let _guard = live_bus::encode_counter_guard();

        const FRAMES: usize = 20_000;
        const SUBSCRIBERS: usize = 32;

        let payload = serde_json::json!({
            "ts": 1_726_000_000_000_i64,
            "price": "63145.27",
            "size": "0.0142",
            "side": "buy",
        });
        let ids: Vec<Uuid> = (0..SUBSCRIBERS).map(|_| Uuid::new_v4()).collect();

        // BEFORE: the shipped-until-now path — deep-clone the Value per
        // subscription, then serialise the whole typed message per subscription.
        let t0 = Instant::now();
        let mut before_bytes = 0usize;
        for _ in 0..FRAMES {
            for id in &ids {
                let out = WsOutMessage::Frame {
                    sub_id: *id,
                    lane: "market.trades".to_owned(),
                    instrument: "BTC-USD".to_owned(),
                    payload: payload.clone(),
                };
                before_bytes += serde_json::to_string(&out).unwrap().len();
            }
        }
        let before = t0.elapsed();

        // AFTER: encode once per frame at publish time, append per subscription.
        let tx = live_bus::channel();
        let mut rx = tx.subscribe();
        let t1 = Instant::now();
        let mut after_bytes = 0usize;
        for _ in 0..FRAMES {
            live_bus::publish(&tx, "market.trades", "BTC-USD", payload.clone());
            let frame = rx.try_recv().unwrap();
            for id in &ids {
                after_bytes += text(&render_frame(frame.body(), *id)).len();
            }
        }
        let after = t1.elapsed();

        assert_eq!(before_bytes, after_bytes, "identical bytes on the wire");
        println!(
            "ws fan-out {FRAMES} frames x {SUBSCRIBERS} subs:\n  \
             before (encode per subscriber): {before:?}\n  \
             after  (encode once per frame): {after:?}\n  \
             speedup: {:.2}x",
            before.as_secs_f64() / after.as_secs_f64()
        );
    }
}
