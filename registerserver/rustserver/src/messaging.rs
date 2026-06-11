use axum::extract::ws::Message;
use serde_json::Value;
use tokio::sync::mpsc::error::TrySendError;

use crate::state::{Sender, ServerState};

pub(crate) fn broadcast_web_locked(inner: &ServerState, payload: &Value) {
    for web_client in inner.web_clients.values() {
        send_json(&web_client.sender, payload);
    }
}

pub(crate) fn send_json(tx: &Sender, payload: &Value) -> bool {
    send_message(tx, Message::Text(payload.to_string().into()))
}

pub(crate) fn send_message(tx: &Sender, message: Message) -> bool {
    match tx.try_send(message) {
        Ok(()) => true,
        Err(TrySendError::Closed(_)) | Err(TrySendError::Full(_)) => false,
    }
}
