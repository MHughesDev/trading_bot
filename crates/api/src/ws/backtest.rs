//! GET /ws/backtest-suite — WebSocket progress lane for the backtest workbench
//! (J-5.4). Mirrors the models/jobs lane: a broadcast channel on the manager,
//! drained to the socket here. Frames are scoped to the connecting user by the
//! same bearer-token-derived id the REST routes use, so a user never sees another
//! user's run/study/gate progress.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use backtest::suite::user_id_from_token;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct WsQuery {
    /// Bearer token as a query param (the browser WS API can't set headers).
    token: Option<String>,
}

/// GET /ws/backtest-suite — upgrade and stream this user's progress frames.
pub async fn ws_backtest_suite(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Response {
    let token = match query.token.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => t.to_owned(),
        None => return (StatusCode::UNAUTHORIZED, "missing token query param").into_response(),
    };
    let user_id = user_id_from_token(&token).to_string();
    let rx = state.suite.subscribe_progress();
    ws.on_upgrade(move |socket| handle_ws(socket, rx, user_id))
}

async fn handle_ws(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<serde_json::Value>,
    user_id: String,
) {
    // Confirm the connection immediately.
    if socket
        .send(Message::Text(
            serde_json::json!({ "type": "ready" }).to_string().into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
    heartbeat.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            frame = rx.recv() => {
                match frame {
                    Ok(payload) => {
                        // Only forward frames belonging to this user.
                        if payload.get("created_by").and_then(|v| v.as_str()) != Some(user_id.as_str()) {
                            continue;
                        }
                        let text = serde_json::json!({ "type": "progress", "payload": payload }).to_string();
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    // Lagged: skip the gap and keep going.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => { let _ = socket.send(Message::Pong(data)).await; }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                let hb = serde_json::json!({ "type": "heartbeat" }).to_string();
                if socket.send(Message::Text(hb.into())).await.is_err() {
                    break;
                }
            }
        }
    }
}
