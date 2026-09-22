//! GET /ws/backtest-suite — WebSocket progress lane for the backtest workbench
//! (J-5.4). Mirrors the models/jobs lane: a broadcast channel on the manager,
//! drained to the socket here. Frames are scoped to the connecting user, so a
//! user never sees another user's run/study/gate progress.
//!
//! The scoping id is the `sessions`-table user id, which is what every REST
//! caller stamps into a frame's `created_by`. It used to be a UUIDv5 derived
//! from the token string instead — so the token was never verified, *and* the
//! id it produced could not equal any real `created_by`, which is why this lane
//! forwarded nothing at all.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};

use crate::state::AppState;
use crate::ws::auth::WsBearerToken;

/// GET /ws/backtest-suite — upgrade and stream this user's progress frames.
///
/// The token arrives as a query parameter because a browser `WebSocket` cannot
/// set headers; [`WsBearerToken`] resolves it against the `sessions` table
/// before the upgrade, and refuses it when it does not resolve.
pub async fn ws_backtest_suite(
    ws: WebSocketUpgrade,
    session: WsBearerToken,
    State(state): State<AppState>,
) -> Response {
    let user_id = session.user_id().to_string();
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
