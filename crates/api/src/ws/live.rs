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
    for sub in gateway.list_for_user(user_id) {
        if !my_panels.contains(&sub.panel_id) {
            continue;
        }
        if sub.lane != frame.lane || sub.instrument != frame.instrument {
            continue;
        }
        let out = WsOutMessage::Frame {
            sub_id: sub.id,
            lane: sub.lane.clone(),
            instrument: sub.instrument.clone(),
            payload: frame.payload.clone(),
        };
        if let Some(m) = json_msg(&out) {
            if socket.send(m).await.is_err() {
                return false;
            }
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
