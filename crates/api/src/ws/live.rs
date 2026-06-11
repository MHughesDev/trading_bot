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
use tracing::debug;
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
    ws.on_upgrade(move |socket| handle_ws(socket, state.gateway, user_id))
}

async fn handle_ws(mut socket: WebSocket, gateway: Arc<SubscriptionRegistry>, user_id: String) {
    // Send initial heartbeat to confirm connection.
    if socket
        .send(json_msg(&WsOutMessage::Heartbeat { ts: now_iso() }))
        .await
        .is_err()
    {
        return;
    }

    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
    heartbeat.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_client_message(&mut socket, &gateway, &user_id, &text).await;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                if socket
                    .send(json_msg(&WsOutMessage::Heartbeat { ts: now_iso() }))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    gateway.remove_all_for_user(&user_id);
    debug!(user_id, "ws connection closed");
}

async fn handle_client_message(
    socket: &mut WebSocket,
    gateway: &SubscriptionRegistry,
    user_id: &str,
    text: &str,
) {
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            let _ = socket
                .send(json_msg(&WsOutMessage::Error {
                    code: "parse_error".to_owned(),
                    message: e.to_string(),
                }))
                .await;
            return;
        }
    };

    if msg.unsubscribe {
        gateway.remove_panel(&msg.panel_id, user_id);
        return;
    }

    for spec in &msg.subscribe {
        match gateway.subscribe(
            msg.panel_id.clone(),
            user_id.to_owned(),
            &spec.lane,
            spec.instrument.clone(),
            user_id,
            spec.depth,
            spec.max_fps,
        ) {
            Ok(sub) => {
                let _ = socket
                    .send(json_msg(&WsOutMessage::Subscribed {
                        sub_id: sub.id,
                        panel_id: sub.panel_id.clone(),
                        lane: sub.lane.clone(),
                        instrument: sub.instrument.clone(),
                    }))
                    .await;
            }
            Err(e) => {
                let _ = socket
                    .send(json_msg(&WsOutMessage::Error {
                        code: "subscription_error".to_owned(),
                        message: e.to_string(),
                    }))
                    .await;
            }
        }
    }
}

fn json_msg(msg: &WsOutMessage) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap_or_default().into())
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}
