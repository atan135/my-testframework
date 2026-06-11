use std::time::Duration;

use axum::{body::Bytes, extract::ws::Message};
use chrono::Utc;
use serde_json::json;
use tokio::time;

use crate::{
    clients::{detach_web_socket, remove_unity_client, to_public_client_locked},
    logging::LogEvent,
    messaging::{broadcast_web_locked, send_message},
    state::SharedState,
};

pub(crate) async fn heartbeat_loop(state: SharedState) {
    let mut interval = time::interval(Duration::from_millis(state.config.heartbeat_interval_ms));
    loop {
        interval.tick().await;

        let mut unity_failures = Vec::new();
        let mut web_failures = Vec::new();

        {
            let mut inner = state.inner.write().await;
            for (client_id, client) in inner.unity_clients.iter_mut() {
                if !client.is_alive {
                    unity_failures.push((client_id.clone(), client.connection_id.clone()));
                    continue;
                }

                client.is_alive = false;
                if !send_message(&client.sender, Message::Ping(Bytes::new())) {
                    unity_failures.push((client_id.clone(), client.connection_id.clone()));
                }
            }

            for (socket_id, web_client) in inner.web_clients.iter_mut() {
                if !web_client.is_alive {
                    web_failures.push(socket_id.clone());
                    continue;
                }

                web_client.is_alive = false;
                if !send_message(&web_client.sender, Message::Ping(Bytes::new())) {
                    web_failures.push(socket_id.clone());
                }
            }
        }

        for (client_id, connection_id) in unity_failures {
            remove_unity_client(
                state.clone(),
                &client_id,
                "ping_timeout",
                Some(&connection_id),
            )
            .await;
        }

        for socket_id in web_failures {
            detach_web_socket(state.clone(), &socket_id, "ping_timeout").await;
        }

        mark_stale_unity_clients(state.clone()).await;
    }
}

async fn mark_stale_unity_clients(state: SharedState) {
    let mut inner = state.inner.write().await;
    let now = Utc::now();
    let stale_after = chrono::Duration::milliseconds(state.config.unity_heartbeat_stale_ms as i64);
    let stale_client_ids = inner
        .unity_clients
        .values()
        .filter(|client| client.available && now - client.last_seen_at > stale_after)
        .map(|client| client.client_id.clone())
        .collect::<Vec<_>>();

    for client_id in stale_client_ids {
        let mut event_client_id = None;
        if let Some(client) = inner.unity_clients.get_mut(&client_id) {
            client.available = false;
            client.unavailable_reason = "heartbeat_timeout".to_string();
            client.availability_changed_at = Utc::now();
            event_client_id = Some(client.client_id.clone());
        }
        if let Some(client_id) = event_client_id
            && let Some(client) = inner.unity_clients.get(&client_id)
        {
            broadcast_web_locked(
                &inner,
                &json!({
                    "type": "unity_unavailable",
                    "client": to_public_client_locked(&inner, client),
                    "reason": "heartbeat_timeout",
                }),
            );
            LogEvent::warn("unity_unavailable")
                .client_id_str(&client_id)
                .field("reason", "heartbeat_timeout")
                .field("staleMs", state.config.unity_heartbeat_stale_ms)
                .emit();
        }
    }
}
