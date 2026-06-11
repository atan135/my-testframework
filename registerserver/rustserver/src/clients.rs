use std::collections::{HashMap, HashSet};

use axum::extract::ws::{CloseFrame, Message, close_code};
use chrono::Utc;
use serde_json::{Value, json};

use crate::{
    execution::{
        is_client_running_locked, release_client_lock_if_idle_locked,
        release_idle_locks_for_controller_locked,
    },
    logging::LogEvent,
    messaging::{broadcast_web_locked, send_message},
    state::{
        ClientLock, Controller, ControllerIdentity, Sender, ServerState, SharedState, UnityClient,
        WebClient,
    },
    util::{uuid, value_to_opt_string},
};

pub(crate) const DUPLICATE_CLIENT_NAME_ERROR_CODE: &str = "duplicate_client_name";

pub(crate) enum UnityRegisterResult {
    Registered {
        client_id: String,
    },
    Rejected {
        client_id: String,
        client_name: String,
        existing_client_id: String,
        message: String,
    },
}

pub(crate) async fn register_unity_client(
    state: SharedState,
    tx: &Sender,
    connection_id: &str,
    remote_address: &str,
    payload: &Value,
) -> UnityRegisterResult {
    let client_id = value_to_opt_string(payload.get("clientId")).unwrap_or_else(uuid);
    let client_name = parse_client_name(payload, &client_id);
    let now = Utc::now();
    let mut replaced_sender = None;
    let ignored_stale_busy;

    {
        let mut inner = state.inner.write().await;
        if let Some(existing) =
            find_duplicate_client_name_locked(&inner, &client_id, connection_id, &client_name)
        {
            let existing_client_id = existing.client_id.clone();
            let message = format!(
                "QaTest 客户端名称“{client_name}”已存在，当前连接已被拒绝。请修改 QaTestClient 的 clientName 后重试。"
            );
            LogEvent::warn("unity_duplicate_client_name_rejected")
                .client_id_str(&client_id)
                .field("connectionId", connection_id)
                .field("remoteAddress", remote_address)
                .field("clientName", client_name.clone())
                .field("existingClientId", existing_client_id.clone())
                .field("existingConnectionId", existing.connection_id.clone())
                .field("errorCode", DUPLICATE_CLIENT_NAME_ERROR_CODE)
                .field("error", message.clone())
                .emit();

            return UnityRegisterResult::Rejected {
                client_id,
                client_name,
                existing_client_id,
                message,
            };
        }

        let (connected_at, availability_changed_at, lock) =
            if let Some(existing) = inner.unity_clients.get(&client_id) {
                if existing.connection_id != connection_id && !existing.sender.is_closed() {
                    replaced_sender = Some(existing.sender.clone());
                }
                (
                    existing.connected_at,
                    existing.availability_changed_at,
                    existing.lock.clone(),
                )
            } else {
                (now, now, None)
            };

        let (client_busy, current_request_id, current_method_name, parsed_ignored_stale_busy) =
            parse_unity_execution_state_locked(&inner, payload);
        ignored_stale_busy = parsed_ignored_stale_busy;

        let ip_addresses = parse_ip_addresses(payload);
        let ip_address = value_to_opt_string(payload.get("ipAddress"))
            .filter(|value| !value.is_empty())
            .or_else(|| ip_addresses.first().cloned())
            .unwrap_or_default();

        let client = UnityClient {
            client_id: client_id.clone(),
            name: client_name.clone(),
            ip_address,
            ip_addresses,
            remote_address: remote_address.to_string(),
            platform: value_to_opt_string(payload.get("platform"))
                .unwrap_or_else(|| "unknown".to_string()),
            unity_version: value_to_opt_string(payload.get("unityVersion")).unwrap_or_default(),
            device_name: value_to_opt_string(payload.get("deviceName")).unwrap_or_default(),
            operating_system: value_to_opt_string(payload.get("operatingSystem"))
                .unwrap_or_default(),
            methods: payload
                .get("methods")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            connected_at,
            last_seen_at: now,
            availability_changed_at,
            available: true,
            unavailable_reason: String::new(),
            client_busy,
            current_request_id,
            current_method_name,
            sender: tx.clone(),
            connection_id: connection_id.to_string(),
            is_alive: true,
            lock,
        };

        inner.unity_clients.insert(client_id.clone(), client);
        if !client_busy {
            release_client_lock_if_idle_locked(&mut inner, &client_id, "unity_register_idle");
        }
        if let Some(client) = inner.unity_clients.get(&client_id) {
            broadcast_web_locked(
                &inner,
                &json!({ "type": "unity_registered", "client": to_public_client_locked(&inner, client) }),
            );
        }
    }

    let replaced = replaced_sender.is_some();
    if let Some(sender) = replaced_sender {
        let _ = send_message(
            &sender,
            Message::Close(Some(CloseFrame {
                code: close_code::NORMAL,
                reason: "Replaced by a new connection.".into(),
            })),
        );
    }

    LogEvent::new("unity_registered")
        .client_id_str(&client_id)
        .field("connectionId", connection_id)
        .field("remoteAddress", remote_address)
        .field("replaced", replaced)
        .field("name", client_name)
        .field_opt("ipAddress", value_to_opt_string(payload.get("ipAddress")))
        .field_opt("platform", value_to_opt_string(payload.get("platform")))
        .field(
            "methodCount",
            payload
                .get("methods")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        )
        .emit();

    if ignored_stale_busy {
        LogEvent::warn("unity_stale_busy_ignored")
            .request_id_str(
                &value_to_opt_string(payload.get("currentRequestId")).unwrap_or_default(),
            )
            .client_id_str(&client_id)
            .field("source", "register")
            .emit();
    }

    UnityRegisterResult::Registered { client_id }
}

fn parse_ip_addresses(payload: &Value) -> Vec<String> {
    let Some(values) = payload.get("ipAddresses").and_then(Value::as_array) else {
        return Vec::new();
    };

    values
        .iter()
        .filter_map(|value| value_to_opt_string(Some(value)))
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_client_name(payload: &Value, client_id: &str) -> String {
    value_to_opt_string(payload.get("name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| client_id.to_string())
}

fn find_duplicate_client_name_locked<'a>(
    inner: &'a ServerState,
    client_id: &str,
    connection_id: &str,
    client_name: &str,
) -> Option<&'a UnityClient> {
    inner.unity_clients.values().find(|client| {
        !(client.client_id == client_id && client.connection_id == connection_id)
            && client.name == client_name
    })
}

pub(crate) async fn touch_unity_client(
    state: SharedState,
    client_id: &str,
    connection_id: &str,
    source: &str,
) {
    let mut inner = state.inner.write().await;
    let mut available_event = None;
    if let Some(client) = inner.unity_clients.get_mut(client_id) {
        if client.connection_id != connection_id {
            return;
        }
        client.last_seen_at = Utc::now();
        client.is_alive = true;
        if !client.available {
            client.available = true;
            client.unavailable_reason.clear();
            client.availability_changed_at = Utc::now();
            available_event = Some(client.client_id.clone());
        }
    }

    if let Some(client_id) = available_event
        && let Some(client) = inner.unity_clients.get(&client_id)
    {
        broadcast_web_locked(
            &inner,
            &json!({
                "type": "unity_available",
                "client": to_public_client_locked(&inner, client),
                "source": source,
            }),
        );
        LogEvent::new("unity_available")
            .client_id_str(&client_id)
            .field("source", source)
            .emit();
    }
}

pub(crate) async fn update_unity_client_execution_state(
    state: SharedState,
    client_id: &str,
    connection_id: &str,
    payload: &Value,
) {
    let has_execution_state = payload.get("busy").is_some()
        || payload.get("currentRequestId").is_some()
        || payload.get("currentMethodName").is_some();
    if !has_execution_state {
        return;
    }

    let mut inner = state.inner.write().await;
    let (busy, current_request_id, current_method_name, ignored_stale_busy) =
        parse_unity_execution_state_locked(&inner, payload);
    let mut changed_client_id = None;
    let mut became_idle_client_id = None;
    if let Some(client) = inner.unity_clients.get_mut(client_id) {
        if client.connection_id != connection_id {
            return;
        }

        let changed = client.client_busy != busy
            || client.current_request_id != current_request_id
            || client.current_method_name != current_method_name;
        client.client_busy = busy;
        client.current_request_id = current_request_id;
        client.current_method_name = current_method_name;
        if changed {
            changed_client_id = Some(client.client_id.clone());
            if !client.client_busy {
                became_idle_client_id = Some(client.client_id.clone());
            }
        }
    }

    if let Some(client_id) = became_idle_client_id.as_deref() {
        release_client_lock_if_idle_locked(&mut inner, client_id, "unity_client_idle");
    }

    if let Some(client_id) = changed_client_id
        && let Some(client) = inner.unity_clients.get(&client_id)
    {
        broadcast_web_locked(
            &inner,
            &json!({
                "type": "unity_state_changed",
                "client": to_public_client_locked(&inner, client),
                "source": payload.get("type").and_then(Value::as_str).unwrap_or("message"),
            }),
        );
        LogEvent::new("unity_state_changed")
            .request_id_str(&client.current_request_id)
            .client_id_str(&client_id)
            .field("busy", client.client_busy)
            .field_opt(
                "currentMethodName",
                Some(client.current_method_name.clone()),
            )
            .field(
                "source",
                payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("message"),
            )
            .emit();
    }

    if ignored_stale_busy {
        LogEvent::warn("unity_stale_busy_ignored")
            .request_id_str(
                &value_to_opt_string(payload.get("currentRequestId")).unwrap_or_default(),
            )
            .client_id_str(client_id)
            .field(
                "source",
                payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("message"),
            )
            .emit();
    }
}

fn parse_unity_execution_state_locked(
    inner: &ServerState,
    payload: &Value,
) -> (bool, String, String, bool) {
    let busy = payload
        .get("busy")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let current_request_id = value_to_opt_string(payload.get("currentRequestId"))
        .filter(|value| busy && !value.is_empty())
        .unwrap_or_default();
    let current_method_name = value_to_opt_string(payload.get("currentMethodName"))
        .filter(|value| busy && !value.is_empty())
        .unwrap_or_default();

    if busy && is_stale_busy_report_locked(inner, &current_request_id) {
        return (false, String::new(), String::new(), true);
    }

    (busy, current_request_id, current_method_name, false)
}

fn is_stale_busy_report_locked(inner: &ServerState, current_request_id: &str) -> bool {
    let mut has_request_ids = false;
    for request_id in current_request_id
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        has_request_ids = true;
        if !is_terminal_execution_locked(inner, request_id) {
            return false;
        }
    }

    has_request_ids
}

fn is_terminal_execution_locked(inner: &ServerState, request_id: &str) -> bool {
    if inner.pending_executions.contains_key(request_id) {
        return false;
    }

    inner
        .execution_history
        .iter()
        .any(|item| item.request_id == request_id && item.status != "running")
}

pub(crate) async fn mark_unity_socket_alive(
    state: SharedState,
    client_id: &str,
    connection_id: &str,
) {
    if client_id.is_empty() {
        return;
    }
    let mut inner = state.inner.write().await;
    if let Some(client) = inner.unity_clients.get_mut(client_id)
        && client.connection_id == connection_id
    {
        client.is_alive = true;
    }
}

pub(crate) async fn mark_web_socket_alive(state: SharedState, socket_id: &str) {
    let mut inner = state.inner.write().await;
    if let Some(client) = inner.web_clients.get_mut(socket_id) {
        client.is_alive = true;
    }
}

pub(crate) async fn remove_unity_client(
    state: SharedState,
    client_id: &str,
    reason: &str,
    connection_id: Option<&str>,
) {
    let mut inner = state.inner.write().await;
    if let Some(client) = inner.unity_clients.get(client_id) {
        if connection_id.is_some_and(|id| client.connection_id != id) {
            return;
        }
    } else {
        return;
    }

    inner.unity_clients.remove(client_id);
    broadcast_web_locked(
        &inner,
        &json!({ "type": "unity_disconnected", "clientId": client_id, "reason": reason }),
    );
    LogEvent::warn("unity_disconnected")
        .client_id_str(client_id)
        .field("reason", reason)
        .emit();
}

pub(crate) async fn attach_controller(
    state: SharedState,
    socket_id: &str,
    tx: &Sender,
    query: &HashMap<String, String>,
) -> ControllerIdentity {
    let requested_id = query
        .get("controllerId")
        .map(|value| value.trim())
        .unwrap_or("");
    let controller_id = if requested_id.is_empty() {
        uuid()
    } else {
        requested_id.to_string()
    };
    let controller_type = query
        .get("controllerType")
        .or_else(|| query.get("controller"))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("web")
        .to_string();
    let now = Utc::now();

    let mut inner = state.inner.write().await;
    let controller = inner
        .controllers
        .entry(controller_id.clone())
        .or_insert_with(|| Controller {
            id: controller_id.clone(),
            controller_type: controller_type.clone(),
            sockets: HashSet::new(),
            last_seen_at: now,
            last_disconnected_at: None,
        });
    controller.controller_type = controller_type.clone();
    controller.last_seen_at = now;
    controller.sockets.insert(socket_id.to_string());

    inner.web_clients.insert(
        socket_id.to_string(),
        WebClient {
            sender: tx.clone(),
            controller_id: controller_id.clone(),
            is_alive: true,
        },
    );

    let identity = ControllerIdentity {
        id: controller_id,
        controller_type,
    };
    LogEvent::new("controller_connected")
        .controller(&identity)
        .field("socketId", socket_id)
        .emit();
    identity
}

pub(crate) async fn touch_controller(state: SharedState, controller_id: &str) {
    let mut inner = state.inner.write().await;
    if let Some(controller) = inner.controllers.get_mut(controller_id) {
        controller.last_seen_at = Utc::now();
    }
}

pub(crate) async fn detach_web_socket(state: SharedState, socket_id: &str, reason: &str) {
    let mut inner = state.inner.write().await;
    let Some(web_client) = inner.web_clients.remove(socket_id) else {
        return;
    };

    let mut release_owner_id = None;
    if let Some(controller) = inner.controllers.get_mut(&web_client.controller_id) {
        controller.sockets.remove(socket_id);
        controller.last_disconnected_at = Some(Utc::now());
        if controller.sockets.is_empty() {
            release_owner_id = Some(controller.id.clone());
        }
    }

    if let Some(owner_id) = release_owner_id {
        release_idle_locks_for_controller_locked(&mut inner, &owner_id, reason);
    }
    LogEvent::new("controller_disconnected")
        .controller_id(Some(web_client.controller_id))
        .field("socketId", socket_id)
        .field("reason", reason)
        .emit();
}

pub(crate) fn create_transient_controller(controller_type: &str) -> ControllerIdentity {
    ControllerIdentity {
        id: format!("{controller_type}:{}", uuid()),
        controller_type: controller_type.to_string(),
    }
}

pub(crate) fn to_public_controller(controller: &ControllerIdentity) -> Value {
    json!({
        "ownerId": controller.id,
        "ownerType": controller.controller_type,
    })
}

pub(crate) fn get_unity_client_snapshot_locked(inner: &ServerState) -> Vec<Value> {
    inner
        .unity_clients
        .values()
        .map(|client| to_public_client_locked(inner, client))
        .collect()
}

pub(crate) fn to_public_client_locked(inner: &ServerState, client: &UnityClient) -> Value {
    json!({
        "clientId": client.client_id,
        "name": client.name,
        "ipAddress": client.ip_address,
        "ipAddresses": client.ip_addresses,
        "remoteAddress": client.remote_address,
        "platform": client.platform,
        "unityVersion": client.unity_version,
        "deviceName": client.device_name,
        "operatingSystem": client.operating_system,
        "methods": client.methods,
        "connectedAt": client.connected_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "lastSeenAt": client.last_seen_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "availabilityChangedAt": client.availability_changed_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "online": !client.sender.is_closed(),
        "available": client.available,
        "unavailableReason": client.unavailable_reason,
        "running": is_client_running_locked(inner, &client.client_id, None),
        "clientBusy": client.client_busy,
        "currentRequestId": client.current_request_id,
        "currentMethodName": client.current_method_name,
        "lock": to_public_lock_locked(inner, client.lock.as_ref()),
    })
}

pub(crate) fn to_public_lock_locked(inner: &ServerState, lock: Option<&ClientLock>) -> Value {
    let Some(lock) = lock else {
        return Value::Null;
    };
    let owner_connected = inner
        .controllers
        .get(&lock.owner_id)
        .is_some_and(|controller| !controller.sockets.is_empty());

    json!({
        "ownerId": lock.owner_id,
        "ownerType": lock.owner_type,
        "ownerConnected": owner_connected,
        "acquiredAt": lock.acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "lastSeenAt": lock.last_seen_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ExecutionRecord;
    use tokio::sync::mpsc;

    #[test]
    fn stale_busy_report_requires_terminal_history() {
        let mut inner = ServerState::default();

        assert!(!is_stale_busy_report_locked(&inner, "request-1"));

        inner
            .execution_history
            .push(execution_record("request-1", "running"));
        assert!(!is_stale_busy_report_locked(&inner, "request-1"));

        inner.execution_history.clear();
        inner
            .execution_history
            .push(execution_record("request-1", "failed"));
        assert!(is_stale_busy_report_locked(&inner, "request-1"));
    }

    #[test]
    fn stale_busy_report_requires_all_reported_requests_terminal() {
        let mut inner = ServerState::default();
        inner
            .execution_history
            .push(execution_record("request-1", "failed"));

        assert!(!is_stale_busy_report_locked(&inner, "request-1,request-2"));

        inner
            .execution_history
            .push(execution_record("request-2", "success"));
        assert!(is_stale_busy_report_locked(&inner, "request-1,request-2"));
    }

    #[test]
    fn duplicate_client_name_matches_different_connection() {
        let mut inner = ServerState::default();
        inner.unity_clients.insert(
            "client-1".to_string(),
            unity_client("client-1", "shared-name", "connection-1"),
        );

        let duplicate =
            find_duplicate_client_name_locked(&inner, "client-2", "connection-2", "shared-name");

        assert!(duplicate.is_some());
        assert_eq!(duplicate.unwrap().client_id, "client-1");
    }

    #[test]
    fn duplicate_client_name_rejects_same_client_id_on_new_connection() {
        let mut inner = ServerState::default();
        inner.unity_clients.insert(
            "client-1".to_string(),
            unity_client("client-1", "shared-name", "connection-1"),
        );

        let duplicate =
            find_duplicate_client_name_locked(&inner, "client-1", "connection-2", "shared-name");

        assert!(duplicate.is_some());
    }

    #[test]
    fn duplicate_client_name_allows_same_connection_refresh() {
        let mut inner = ServerState::default();
        inner.unity_clients.insert(
            "client-1".to_string(),
            unity_client("client-1", "shared-name", "connection-1"),
        );

        let duplicate =
            find_duplicate_client_name_locked(&inner, "client-1", "connection-1", "shared-name");

        assert!(duplicate.is_none());
    }

    #[test]
    fn duplicate_client_name_rejects_changed_client_id_on_same_connection() {
        let mut inner = ServerState::default();
        inner.unity_clients.insert(
            "client-1".to_string(),
            unity_client("client-1", "shared-name", "connection-1"),
        );

        let duplicate =
            find_duplicate_client_name_locked(&inner, "client-2", "connection-1", "shared-name");

        assert!(duplicate.is_some());
    }

    fn execution_record(request_id: &str, status: &str) -> ExecutionRecord {
        ExecutionRecord {
            request_id: request_id.to_string(),
            client_id: "client-1".to_string(),
            method_id: "Method()".to_string(),
            method_name: "Method".to_string(),
            arguments: Vec::new(),
            status: status.to_string(),
            allow_parallel_execution: false,
            success: None,
            result: None,
            error: None,
            duration_ms: None,
            started_at: None,
            finished_at: None,
            sequence_id: None,
            step_id: None,
            step_index: None,
            step_number: None,
            total_steps: None,
        }
    }

    fn unity_client(client_id: &str, name: &str, connection_id: &str) -> UnityClient {
        let now = Utc::now();
        let (sender, _receiver) = mpsc::channel(1);
        UnityClient {
            client_id: client_id.to_string(),
            name: name.to_string(),
            ip_address: String::new(),
            ip_addresses: Vec::new(),
            remote_address: String::new(),
            platform: String::new(),
            unity_version: String::new(),
            device_name: String::new(),
            operating_system: String::new(),
            methods: Vec::new(),
            connected_at: now,
            last_seen_at: now,
            availability_changed_at: now,
            available: true,
            unavailable_reason: String::new(),
            client_busy: false,
            current_request_id: String::new(),
            current_method_name: String::new(),
            sender,
            connection_id: connection_id.to_string(),
            is_alive: true,
            lock: None,
        }
    }
}
