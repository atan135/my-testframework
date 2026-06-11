use std::{collections::HashMap, net::SocketAddr};

use axum::{
    extract::{
        ConnectInfo, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    auth::{is_browser_web_socket, is_web_console_authorized},
    clients::{
        DUPLICATE_CLIENT_NAME_ERROR_CODE, UnityRegisterResult, attach_controller,
        detach_web_socket, get_unity_client_snapshot_locked, mark_unity_socket_alive,
        mark_web_socket_alive, register_unity_client, remove_unity_client, to_public_controller,
        touch_controller, touch_unity_client, update_unity_client_execution_state,
    },
    execution::{dispatch_unity_execution, handle_unity_qa_result, stop_execution_or_sequence},
    logging::LogEvent,
    messaging::{send_json, send_message},
    sequence::run_execution_sequence,
    state::{ControllerIdentity, DispatchRequest, ExecutionMeta, Sender, SharedState},
    util::{
        iso_now, merge_json_object, parse_message, parse_message_bytes, uuid, value_to_opt_string,
        value_to_string, value_to_u64,
    },
};

pub(crate) async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let role = query.get("role").map(String::as_str).unwrap_or("");
    if is_browser_web_socket(&query) && !is_web_console_authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized web console token.").into_response();
    }

    if role == "unity" {
        ws.on_upgrade(move |socket| handle_unity_socket(state, socket, remote_addr))
            .into_response()
    } else {
        ws.on_upgrade(move |socket| handle_web_socket(state, socket, query))
            .into_response()
    }
}

async fn handle_unity_socket(state: SharedState, socket: WebSocket, remote_addr: SocketAddr) {
    let connection_id = uuid();
    let remote_address = remote_addr.ip().to_string();
    let (tx, mut rx) = mpsc::channel::<Message>(state.config.websocket_outbound_queue_size);
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_sender.send(message).await.is_err() {
                break;
            }
        }
    });

    let mut bound_client_id = String::new();

    while let Some(message) = ws_receiver.next().await {
        let message = match message {
            Ok(message) => message,
            Err(_) => break,
        };

        match message {
            Message::Text(text) => {
                let Some(payload) = parse_message(text.as_str()) else {
                    send_json(
                        &tx,
                        &json!({ "type": "error", "error": "Invalid JSON message." }),
                    );
                    continue;
                };
                handle_unity_payload(
                    state.clone(),
                    &tx,
                    &connection_id,
                    &remote_address,
                    &mut bound_client_id,
                    payload,
                )
                .await;
            }
            Message::Binary(bytes) => {
                let Some(payload) = parse_message_bytes(&bytes) else {
                    send_json(
                        &tx,
                        &json!({ "type": "error", "error": "Invalid JSON message." }),
                    );
                    continue;
                };
                handle_unity_payload(
                    state.clone(),
                    &tx,
                    &connection_id,
                    &remote_address,
                    &mut bound_client_id,
                    payload,
                )
                .await;
            }
            Message::Pong(_) => {
                mark_unity_socket_alive(state.clone(), &bound_client_id, &connection_id).await;
            }
            Message::Ping(payload) => {
                let _ = send_message(&tx, Message::Pong(payload));
            }
            Message::Close(_) => break,
        }
    }

    if !bound_client_id.is_empty() {
        remove_unity_client(
            state.clone(),
            &bound_client_id,
            "closed",
            Some(&connection_id),
        )
        .await;
    }
    writer.abort();
}

async fn handle_web_socket(state: SharedState, socket: WebSocket, query: HashMap<String, String>) {
    let socket_id = uuid();
    let (tx, mut rx) = mpsc::channel::<Message>(state.config.websocket_outbound_queue_size);
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_sender.send(message).await.is_err() {
                break;
            }
        }
    });

    let controller = attach_controller(state.clone(), &socket_id, &tx, &query).await;
    {
        let inner = state.inner.read().await;
        send_json(
            &tx,
            &json!({
                "type": "snapshot",
                "controller": to_public_controller(&controller),
                "clients": get_unity_client_snapshot_locked(&inner),
                "history": inner.execution_history,
            }),
        );
    }

    while let Some(message) = ws_receiver.next().await {
        let message = match message {
            Ok(message) => message,
            Err(_) => break,
        };

        match message {
            Message::Text(text) => {
                let Some(payload) = parse_message(text.as_str()) else {
                    send_json(
                        &tx,
                        &json!({ "type": "error", "error": "Invalid JSON message." }),
                    );
                    continue;
                };
                handle_web_payload(state.clone(), &tx, &controller, payload).await;
            }
            Message::Pong(_) => {
                mark_web_socket_alive(state.clone(), &socket_id).await;
            }
            Message::Ping(payload) => {
                let _ = send_message(&tx, Message::Pong(payload));
            }
            Message::Close(_) => break,
            Message::Binary(bytes) => {
                let Some(payload) = parse_message_bytes(&bytes) else {
                    send_json(
                        &tx,
                        &json!({ "type": "error", "error": "Invalid JSON message." }),
                    );
                    continue;
                };
                handle_web_payload(state.clone(), &tx, &controller, payload).await;
            }
        }
    }

    detach_web_socket(state.clone(), &socket_id, "closed").await;
    writer.abort();
}

async fn handle_unity_payload(
    state: SharedState,
    tx: &Sender,
    connection_id: &str,
    remote_address: &str,
    bound_client_id: &mut String,
    payload: Value,
) {
    if payload.get("type").and_then(Value::as_str) == Some("register") {
        match register_unity_client(state.clone(), tx, connection_id, remote_address, &payload)
            .await
        {
            UnityRegisterResult::Registered { client_id } => {
                *bound_client_id = client_id.clone();
                send_json(tx, &json!({ "type": "registered", "clientId": client_id }));
            }
            UnityRegisterResult::Rejected {
                client_id,
                client_name,
                existing_client_id,
                message,
            } => {
                let close_reason = "QaTest 客户端名称重复。";
                send_json(
                    tx,
                    &json!({
                        "type": "error",
                        "fatal": true,
                        "code": DUPLICATE_CLIENT_NAME_ERROR_CODE,
                        "error": message.clone(),
                        "clientId": client_id,
                        "clientName": client_name,
                        "existingClientId": existing_client_id,
                    }),
                );
                let _ = send_message(
                    tx,
                    Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::POLICY,
                        reason: close_reason.into(),
                    })),
                );
            }
        }
        return;
    }

    if bound_client_id.is_empty()
        && let Some(client_id) = value_to_opt_string(payload.get("clientId"))
    {
        *bound_client_id = client_id;
    }

    if !bound_client_id.is_empty() {
        touch_unity_client(
            state.clone(),
            bound_client_id.as_str(),
            connection_id,
            payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("message"),
        )
        .await;
        update_unity_client_execution_state(
            state.clone(),
            bound_client_id.as_str(),
            connection_id,
            &payload,
        )
        .await;
    }

    match payload.get("type").and_then(Value::as_str) {
        Some("heartbeat") => {
            send_json(
                tx,
                &json!({ "type": "heartbeat_ack", "serverTime": iso_now() }),
            );
        }
        Some("qa_result") => {
            handle_unity_qa_result(state, &payload, bound_client_id.as_str()).await;
        }
        _ => {}
    }
}

async fn handle_web_payload(
    state: SharedState,
    tx: &Sender,
    controller: &ControllerIdentity,
    payload: Value,
) {
    touch_controller(state.clone(), &controller.id).await;

    match payload.get("type").and_then(Value::as_str) {
        Some("refresh") => {
            let inner = state.inner.read().await;
            send_json(
                tx,
                &json!({
                    "type": "snapshot",
                    "controller": to_public_controller(controller),
                    "clients": get_unity_client_snapshot_locked(&inner),
                    "history": inner.execution_history,
                }),
            );
        }
        Some("execute") => {
            handle_web_execute(state.clone(), tx, controller, &payload).await;
        }
        Some("execute_sequence") => {
            let state_clone = state.clone();
            let tx_clone = tx.clone();
            let controller_clone = controller.clone();
            let payload_clone = payload.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    run_execution_sequence(state_clone, controller_clone, payload_clone).await
                {
                    send_json(
                        &tx_clone,
                        &json!({
                            "type": "error",
                            "error": error.message,
                        }),
                    );
                }
            });
        }
        Some("stop_sequence")
        | Some("cancel_sequence")
        | Some("stop_execution")
        | Some("cancel_execution")
        | Some("stop") => {
            handle_web_stop(state, tx, controller, &payload).await;
        }
        _ => {}
    }
}

async fn handle_web_execute(
    state: SharedState,
    tx: &Sender,
    controller: &ControllerIdentity,
    payload: &Value,
) {
    let request = DispatchRequest {
        client_id: value_to_string(payload.get("clientId")),
        method_id: value_to_opt_string(payload.get("methodId")),
        method_name: value_to_opt_string(payload.get("methodName")),
        method_arguments: payload.get("arguments").cloned().unwrap_or(Value::Null),
        timeout_ms: value_to_u64(payload.get("timeoutMs")),
        controller: controller.clone(),
        wait_for_result: false,
        allow_busy: false,
        allow_parallel_execution: payload
            .get("allowParallelExecution")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        meta: ExecutionMeta::empty(),
    };

    match dispatch_unity_execution(state, request).await {
        Ok(result) => {
            send_json(
                tx,
                &json!({ "type": "execute_accepted", "execution": result.started }),
            );
        }
        Err(error) => {
            LogEvent::warn("execution_rejected")
                .client_id_str(&value_to_string(payload.get("clientId")))
                .controller(controller)
                .field_opt("methodId", value_to_opt_string(payload.get("methodId")))
                .field("error", error.message.clone())
                .field("statusCode", error.status_code.as_u16())
                .emit();
            send_json(
                tx,
                &json!({
                    "type": "execute_rejected",
                    "error": error.message,
                }),
            );
        }
    }
}

async fn handle_web_stop(
    state: SharedState,
    tx: &Sender,
    controller: &ControllerIdentity,
    payload: &Value,
) {
    let request_id = value_to_opt_string(payload.get("requestId"));
    let sequence_id = value_to_opt_string(payload.get("sequenceId"));
    let reason = value_to_opt_string(payload.get("reason"))
        .unwrap_or_else(|| "Stopped by controller.".to_string());

    match stop_execution_or_sequence(
        state,
        request_id.clone(),
        sequence_id.clone(),
        controller,
        reason,
    )
    .await
    {
        Ok(value) => {
            let mut response = json!({ "type": "stop_accepted" });
            merge_json_object(&mut response, value);
            send_json(tx, &response);
        }
        Err(error) => {
            LogEvent::warn("stop_rejected")
                .request_id(request_id)
                .sequence_id(sequence_id)
                .controller(controller)
                .field("error", error.message.clone())
                .field("statusCode", error.status_code.as_u16())
                .emit();
            send_json(
                tx,
                &json!({
                    "type": "stop_rejected",
                    "error": error.message,
                }),
            );
        }
    }
}
