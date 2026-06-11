use std::time::{Duration, Instant};

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::{
    clients::{
        create_transient_controller, get_unity_client_snapshot_locked, to_public_client_locked,
    },
    execution::dispatch_unity_execution,
    logging::LogEvent,
    messaging::send_json,
    state::{DispatchRequest, ExecutionMeta, SharedState},
    util::{parse_body_or_empty, value_to_opt_string, value_to_u64},
};

pub(crate) async fn api_health(State(state): State<SharedState>) -> impl IntoResponse {
    let inner = state.inner.read().await;
    (
        StatusCode::OK,
        axum::Json(json!({
            "ok": true,
            "uptime": state.started_at.elapsed().as_secs_f64(),
            "unityClientCount": inner.unity_clients.len(),
            "webClientCount": inner.web_clients.len(),
            "controllerCount": inner.controllers.len(),
            "executionTimeoutMs": state.config.execution_timeout_ms,
            "websocketOutboundQueueSize": state.config.websocket_outbound_queue_size,
            "unityHeartbeatStaleMs": state.config.unity_heartbeat_stale_ms,
            "webConsoleAuthRequired": state.config.web_console_token.is_some(),
            "listenHost": state.config.listen_host.to_string(),
            "accessScope": state.config.access_scope.as_str(),
            "clientDistDir": state.config.client_dist_dir.to_string_lossy(),
        })),
    )
}

pub(crate) async fn api_unity_clients(State(state): State<SharedState>) -> impl IntoResponse {
    let inner = state.inner.read().await;
    (
        StatusCode::OK,
        axum::Json(json!({ "clients": get_unity_client_snapshot_locked(&inner) })),
    )
}

pub(crate) async fn api_refresh_methods_if_needed(
    State(state): State<SharedState>,
    AxumPath(client_id): AxumPath<String>,
) -> Response {
    let min_count = state.config.method_refresh_min_count;
    let timeout_ms = state.config.method_refresh_timeout_ms;
    let request_id = format!("refresh-methods:{}", crate::util::uuid());
    let initial_count;
    {
        let inner = state.inner.read().await;
        let Some(client) = inner.unity_clients.get(&client_id) else {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": format!("Unity client {client_id} is not online.") })),
            )
                .into_response();
        };

        initial_count = client.methods.len();
        if initial_count >= min_count {
            return (
                StatusCode::OK,
                axum::Json(json!({
                    "refreshed": false,
                    "reason": "method_count_sufficient",
                    "minMethodCount": min_count,
                    "methodCount": initial_count,
                    "client": to_public_client_locked(&inner, client),
                })),
            )
                .into_response();
        }

        if client.sender.is_closed() {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": format!("Unity client {client_id} is not online.") })),
            )
                .into_response();
        }

        if !send_json(
            &client.sender,
            &json!({
                "type": "refresh_methods",
                "requestId": request_id,
            }),
        ) {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(json!({ "error": format!("Unity client {client_id} outbound queue is unavailable.") })),
            )
                .into_response();
        }
    }

    LogEvent::new("unity_methods_refresh_requested")
        .client_id_str(&client_id)
        .field("requestId", request_id.clone())
        .field("initialMethodCount", initial_count)
        .field("minMethodCount", min_count)
        .emit();

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        {
            let inner = state.inner.read().await;
            if let Some(client) = inner.unity_clients.get(&client_id) {
                let method_count = client.methods.len();
                if method_count >= min_count || Instant::now() >= deadline {
                    return (
                        StatusCode::OK,
                        axum::Json(json!({
                            "refreshed": method_count > initial_count,
                            "reason": if method_count >= min_count { "method_count_sufficient" } else { "timeout" },
                            "requestId": request_id,
                            "minMethodCount": min_count,
                            "initialMethodCount": initial_count,
                            "methodCount": method_count,
                            "client": to_public_client_locked(&inner, client),
                        })),
                    )
                        .into_response();
                }
            } else {
                return (
                    StatusCode::NOT_FOUND,
                    axum::Json(json!({ "error": format!("Unity client {client_id} disconnected during method refresh.") })),
                )
                    .into_response();
            }
        }

        sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) async fn api_results(State(state): State<SharedState>) -> impl IntoResponse {
    let inner = state.inner.read().await;
    (
        StatusCode::OK,
        axum::Json(json!({ "results": inner.execution_history })),
    )
}

pub(crate) async fn api_execute(
    State(state): State<SharedState>,
    AxumPath(client_id): AxumPath<String>,
    body: Bytes,
) -> Response {
    let body = match parse_body_or_empty(&body) {
        Ok(value) => value,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "error": message })),
            )
                .into_response();
        }
    };
    let controller = create_transient_controller("http");
    let request = DispatchRequest {
        client_id,
        method_id: value_to_opt_string(body.get("methodId")),
        method_name: value_to_opt_string(body.get("methodName")),
        method_arguments: body.get("arguments").cloned().unwrap_or(Value::Null),
        timeout_ms: value_to_u64(body.get("timeoutMs")),
        controller,
        wait_for_result: false,
        allow_busy: false,
        allow_parallel_execution: body
            .get("allowParallelExecution")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        meta: ExecutionMeta::empty(),
    };
    let log_client_id = request.client_id.clone();
    let log_controller = request.controller.clone();
    let log_method_id = request.method_id.clone();

    match dispatch_unity_execution(state, request).await {
        Ok(result) => (StatusCode::ACCEPTED, axum::Json(result.started)).into_response(),
        Err(error) => {
            LogEvent::warn("execution_rejected")
                .client_id_str(&log_client_id)
                .controller(&log_controller)
                .field_opt("methodId", log_method_id)
                .field("error", error.message.clone())
                .field("statusCode", error.status_code.as_u16())
                .emit();
            (
                error.status_code,
                axum::Json(json!({ "error": error.message })),
            )
                .into_response()
        }
    }
}

pub(crate) async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        axum::Json(json!({ "error": "Not found" })),
    )
}
