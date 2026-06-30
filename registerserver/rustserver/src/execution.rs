use std::time::Duration;

use axum::{extract::ws::Message, http::StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use tokio::{sync::oneshot, time};

use crate::{
    clients::to_public_lock_locked,
    logging::LogEvent,
    messaging::{broadcast_web_locked, send_message},
    state::{
        ClientLock, ControllerIdentity, DispatchRequest, DispatchResult, ExecutionMeta,
        ExecutionPatch, ExecutionRecord, HISTORY_LIMIT, PendingExecution, RequestError, Sender,
        ServerState, SharedState, UnityClient,
    },
    util::{
        iso_now, normalize_arguments, uuid, value_to_opt_string, value_to_string, value_to_u64,
    },
};

pub(crate) async fn dispatch_unity_execution(
    state: SharedState,
    request: DispatchRequest,
) -> Result<DispatchResult, RequestError> {
    let resolved_method_id = request
        .method_id
        .clone()
        .or(request.method_name.clone())
        .unwrap_or_default();
    if resolved_method_id.is_empty() {
        return Err(RequestError::new(
            "methodId is required.",
            StatusCode::BAD_REQUEST,
        ));
    }

    let command_method_name = request
        .method_name
        .clone()
        .unwrap_or_else(|| resolved_method_id.clone());
    let arguments = normalize_arguments(&request.method_arguments);
    let request_id = uuid();
    let sequence_id = request.meta.sequence_id.clone();
    let allow_parallel_execution = {
        let inner = state.inner.read().await;
        request.allow_parallel_execution
            || method_allows_parallel_execution_locked(
                &inner,
                &request.client_id,
                &resolved_method_id,
                &command_method_name,
            )
    };
    let command = json!({
        "type": "execute",
        "requestId": request_id,
        "methodId": resolved_method_id,
        "methodName": command_method_name,
        "allowParallelExecution": allow_parallel_execution,
        "arguments": arguments,
    });

    let mut started = ExecutionRecord {
        request_id: request_id.clone(),
        client_id: request.client_id.clone(),
        client_name: None,
        client_ip_address: None,
        client_remote_address: None,
        client_ip_addresses: None,
        method_id: resolved_method_id.clone(),
        method_name: command_method_name.clone(),
        arguments: arguments.clone(),
        status: "running".to_string(),
        allow_parallel_execution,
        success: None,
        result: None,
        error: None,
        duration_ms: None,
        started_at: Some(iso_now()),
        finished_at: None,
        sequence_id: sequence_id.clone(),
        step_id: request.meta.step_id.clone(),
        step_index: request.meta.step_index,
        step_number: request.meta.step_number,
        total_steps: request.meta.total_steps,
    };

    let (result_tx, result_rx) = oneshot::channel::<ExecutionRecord>();
    let send_failed = {
        let mut inner = state.inner.write().await;
        if let Some(sequence_id) = sequence_id.as_deref() {
            ensure_sequence_not_cancelled_locked(&inner, sequence_id)?;
        }

        let sender = match prepare_client_for_execution_locked(
            &mut inner,
            &request.client_id,
            &request.controller,
            request.allow_busy,
            allow_parallel_execution,
        ) {
            Ok(sender) => sender,
            Err(error) => {
                LogEvent::warn("execution_rejected")
                    .request_id_str(&request_id)
                    .client_id_str(&request.client_id)
                    .controller(&request.controller)
                    .sequence_id(sequence_id.clone())
                    .field_opt("methodId", Some(resolved_method_id.clone()))
                    .field("error", error.message.clone())
                    .field("statusCode", error.status_code.as_u16())
                    .emit();
                return Err(error);
            }
        };

        apply_client_snapshot_locked(&inner, &mut started);

        inner.pending_executions.insert(
            request_id.clone(),
            PendingExecution {
                started: started.clone(),
                client_id: request.client_id.clone(),
                owner_id: request.controller.id.clone(),
                owner_type: request.controller.controller_type.clone(),
                result_tx: Some(result_tx),
            },
        );
        if let Some(sequence_id) = sequence_id.as_deref()
            && let Some(sequence_state) = inner.active_sequences.get_mut(sequence_id)
        {
            sequence_state.current_request_id = request_id.clone();
        }
        add_history_locked(&mut inner, started.clone());
        state
            .archive
            .enqueue_locked(&inner, &started, Some(&request.controller));
        broadcast_web_locked(
            &inner,
            &json!({ "type": "execution_started", "execution": started }),
        );
        !send_message(&sender, Message::Text(command.to_string().into()))
    };

    let timeout_ms = request
        .timeout_ms
        .filter(|value| *value > 0)
        .unwrap_or(state.config.execution_timeout_ms);
    LogEvent::new("execution_started")
        .request_id_str(&request_id)
        .client_id_str(&request.client_id)
        .controller(&request.controller)
        .sequence_id(sequence_id.clone())
        .field("methodId", resolved_method_id.clone())
        .field("methodName", command_method_name.clone())
        .field("timeoutMs", timeout_ms)
        .field("waitForResult", request.wait_for_result)
        .field("allowParallelExecution", allow_parallel_execution)
        .emit();
    spawn_execution_timeout(state.clone(), request_id.clone(), timeout_ms);

    if send_failed {
        LogEvent::error("execution_send_failed")
            .request_id_str(&request_id)
            .client_id_str(&request.client_id)
            .controller(&request.controller)
            .sequence_id(sequence_id)
            .field("methodId", resolved_method_id)
            .emit();
        complete_pending_execution(
            state.clone(),
            &request_id,
            ExecutionPatch {
                status: "failed".to_string(),
                success: false,
                result: Value::String(String::new()),
                error: "Failed to send execution command.".to_string(),
                duration_ms: 0,
                finished_at: iso_now(),
                client_id: None,
                method_id: None,
                method_name: None,
            },
        )
        .await;
    }

    Ok(DispatchResult {
        request_id,
        started,
        result_rx: request.wait_for_result.then_some(result_rx),
    })
}

fn spawn_execution_timeout(state: SharedState, request_id: String, timeout_ms: u64) {
    tokio::spawn(async move {
        time::sleep(Duration::from_millis(timeout_ms)).await;
        complete_pending_execution(
            state,
            &request_id,
            ExecutionPatch {
                status: "failed".to_string(),
                success: false,
                result: Value::String(String::new()),
                error: format!("Timed out after {timeout_ms} ms."),
                duration_ms: timeout_ms,
                finished_at: iso_now(),
                client_id: None,
                method_id: None,
                method_name: None,
            },
        )
        .await;
    });
}

async fn complete_pending_execution(
    state: SharedState,
    request_id: &str,
    patch: ExecutionPatch,
) -> Option<ExecutionRecord> {
    let mut inner = state.inner.write().await;
    let mut pending = inner.pending_executions.remove(request_id)?;
    let mut result = pending.started.clone();
    let started_client_id = result.client_id.clone();

    result.request_id = request_id.to_string();
    if let Some(client_id) = patch.client_id {
        result.client_id = client_id;
    }
    if result.client_id != started_client_id {
        clear_client_snapshot(&mut result);
    }
    if let Some(method_id) = patch.method_id {
        result.method_id = method_id;
    }
    if let Some(method_name) = patch.method_name {
        result.method_name = method_name;
    }
    result.status = patch.status;
    result.success = Some(patch.success);
    result.result = Some(patch.result);
    result.error = Some(patch.error);
    result.duration_ms = Some(patch.duration_ms);
    result.finished_at = Some(patch.finished_at);
    apply_client_snapshot_locked(&inner, &mut result);
    let owner = ControllerIdentity {
        id: pending.owner_id.clone(),
        controller_type: pending.owner_type.clone(),
    };
    let sequence_id = result.sequence_id.clone();

    add_history_locked(&mut inner, result.clone());
    state.archive.enqueue_locked(&inner, &result, Some(&owner));
    broadcast_web_locked(&inner, &json!({ "type": "qa_result", "result": result }));
    if let Some(result_tx) = pending.result_tx.take() {
        let _ = result_tx.send(result.clone());
    }
    clear_client_busy_for_finished_request_locked(
        &mut inner,
        &result.client_id,
        &result.request_id,
        "execution_finished",
    );
    release_client_lock_if_idle_locked(&mut inner, &result.client_id, "execution_finished");
    LogEvent::new("execution_finished")
        .request_id_str(&result.request_id)
        .client_id_str(&result.client_id)
        .controller_id(Some(owner.id))
        .sequence_id(sequence_id)
        .field("methodId", result.method_id.clone())
        .field("status", result.status.clone())
        .field("success", result.success.unwrap_or(false))
        .field("durationMs", result.duration_ms.unwrap_or(0))
        .field_opt("error", result.error.clone())
        .emit();
    Some(result)
}

fn clear_client_busy_for_finished_request_locked(
    inner: &mut ServerState,
    client_id: &str,
    request_id: &str,
    reason: &str,
) {
    let mut changed_client_id = None;
    if let Some(client) = inner.unity_clients.get_mut(client_id)
        && client.client_busy
        && client.current_request_id == request_id
    {
        client.client_busy = false;
        client.current_request_id.clear();
        client.current_method_name.clear();
        changed_client_id = Some(client.client_id.clone());
    }

    if let Some(client_id) = changed_client_id
        && let Some(client) = inner.unity_clients.get(&client_id)
    {
        broadcast_web_locked(
            inner,
            &json!({
                "type": "unity_state_changed",
                "client": crate::clients::to_public_client_locked(inner, client),
                "source": reason,
            }),
        );
        LogEvent::new("unity_state_changed")
            .request_id_str(request_id)
            .client_id_str(&client_id)
            .field("busy", false)
            .field_opt("currentMethodName", Some(String::new()))
            .field("source", reason)
            .emit();
    }
}

pub(crate) async fn handle_unity_qa_result(
    state: SharedState,
    payload: &Value,
    bound_client_id: &str,
) {
    let request_id = value_to_string(payload.get("requestId"));
    let client_id =
        value_to_opt_string(payload.get("clientId")).unwrap_or_else(|| bound_client_id.to_string());
    let method_id = value_to_opt_string(payload.get("methodId"))
        .or_else(|| value_to_opt_string(payload.get("methodName")))
        .unwrap_or_default();
    let method_name = value_to_opt_string(payload.get("methodName"))
        .or_else(|| value_to_opt_string(payload.get("methodId")))
        .unwrap_or_default();
    let mut success = payload
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result_value = payload
        .get("result")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| Value::String(String::new()));
    let mut error = value_to_opt_string(payload.get("error")).unwrap_or_default();
    let duration_ms = value_to_u64(payload.get("durationMs")).unwrap_or(0);
    if success && let Some(reason) = business_failure_reason(&result_value) {
        success = false;
        if error.is_empty() {
            error = reason;
        }
    }

    let patch = ExecutionPatch {
        status: if success { "success" } else { "failed" }.to_string(),
        success,
        result: result_value.clone(),
        error: error.clone(),
        duration_ms,
        finished_at: iso_now(),
        client_id: Some(client_id.clone()),
        method_id: Some(method_id.clone()),
        method_name: Some(method_name.clone()),
    };

    {
        let inner = state.inner.read().await;
        if inner.pending_executions.contains_key(&request_id) {
            drop(inner);
            complete_pending_execution(state, &request_id, patch).await;
            return;
        }
    }

    let result = ExecutionRecord {
        request_id: request_id.clone(),
        client_id: client_id.clone(),
        client_name: None,
        client_ip_address: None,
        client_remote_address: None,
        client_ip_addresses: None,
        method_id,
        method_name,
        arguments: Vec::new(),
        status: if success { "success" } else { "failed" }.to_string(),
        allow_parallel_execution: false,
        success: Some(success),
        result: Some(result_value),
        error: Some(error),
        duration_ms: Some(duration_ms),
        started_at: None,
        finished_at: Some(iso_now()),
        sequence_id: None,
        step_id: None,
        step_index: None,
        step_number: None,
        total_steps: None,
    };

    let mut inner = state.inner.write().await;
    let mut result = result;
    apply_client_snapshot_locked(&inner, &mut result);
    if let Some(existing) = find_history_by_request_id_locked(&inner, &request_id)
        && existing.status != "running"
    {
        let mut late_result = serde_json::to_value(&result).unwrap_or_else(|_| json!({}));
        if let Some(object) = late_result.as_object_mut() {
            object.insert("late".to_string(), Value::Bool(true));
        }
        broadcast_web_locked(
            &inner,
            &json!({ "type": "qa_result_late", "result": late_result }),
        );
        LogEvent::warn("execution_late_result")
            .request_id_str(&request_id)
            .client_id_str(&result.client_id)
            .field("methodId", result.method_id.clone())
            .field("status", existing.status.clone())
            .emit();
        return;
    }

    add_history_locked(&mut inner, result.clone());
    state.archive.enqueue_locked(&inner, &result, None);
    broadcast_web_locked(&inner, &json!({ "type": "qa_result", "result": result }));
    release_client_lock_if_idle_locked(&mut inner, &result.client_id, "unexpected_result");
    LogEvent::warn("execution_unexpected_result")
        .request_id_str(&result.request_id)
        .client_id_str(&result.client_id)
        .field("methodId", result.method_id.clone())
        .field("status", result.status.clone())
        .emit();
}

pub(crate) async fn stop_execution_or_sequence(
    state: SharedState,
    request_id: Option<String>,
    sequence_id: Option<String>,
    controller: &ControllerIdentity,
    reason: String,
) -> Result<Value, RequestError> {
    if let Some(sequence_id) = sequence_id.filter(|value| !value.is_empty()) {
        let current_request_id = {
            let mut inner = state.inner.write().await;
            let (state_request_id, owner_id) = {
                let sequence_state =
                    inner
                        .active_sequences
                        .get_mut(&sequence_id)
                        .ok_or_else(|| {
                            RequestError::new(
                                format!("Sequence {sequence_id} is not running."),
                                StatusCode::NOT_FOUND,
                            )
                        })?;
                assert_owner(&sequence_state.owner_id, controller)?;
                sequence_state.cancelled = true;
                sequence_state.cancel_reason = reason.clone();
                sequence_state.notify.notify_waiters();
                (
                    sequence_state.current_request_id.clone(),
                    sequence_state.owner_id.clone(),
                )
            };
            if state_request_id.is_empty() {
                find_pending_sequence_request_locked(&inner, &sequence_id, &owner_id)
                    .unwrap_or_default()
            } else {
                state_request_id
            }
        };
        LogEvent::warn("stop_sequence_requested")
            .request_id_str(&current_request_id)
            .sequence_id(Some(sequence_id.clone()))
            .controller(controller)
            .field("reason", reason.clone())
            .emit();

        if !current_request_id.is_empty() {
            complete_pending_execution(
                state,
                &current_request_id,
                ExecutionPatch {
                    status: "cancelled".to_string(),
                    success: false,
                    result: Value::String(String::new()),
                    error: reason,
                    duration_ms: 0,
                    finished_at: iso_now(),
                    client_id: None,
                    method_id: None,
                    method_name: None,
                },
            )
            .await;
        }

        return Ok(json!({ "sequenceId": sequence_id, "status": "cancelling" }));
    }

    if let Some(request_id) = request_id.filter(|value| !value.is_empty()) {
        let sequence_id = {
            let mut inner = state.inner.write().await;
            let pending = inner.pending_executions.get(&request_id).ok_or_else(|| {
                RequestError::new(
                    format!("Execution {request_id} is not running."),
                    StatusCode::NOT_FOUND,
                )
            })?;
            assert_owner(&pending.owner_id, controller)?;

            let sequence_id = pending.started.sequence_id.clone();
            if let Some(sequence_id) = &sequence_id
                && let Some(sequence_state) = inner.active_sequences.get_mut(sequence_id)
            {
                sequence_state.cancelled = true;
                sequence_state.cancel_reason = reason.clone();
                sequence_state.notify.notify_waiters();
            }
            sequence_id
        };
        LogEvent::warn("stop_execution_requested")
            .request_id_str(&request_id)
            .sequence_id(sequence_id.clone())
            .controller(controller)
            .field("reason", reason.clone())
            .emit();

        complete_pending_execution(
            state,
            &request_id,
            ExecutionPatch {
                status: "cancelled".to_string(),
                success: false,
                result: Value::String(String::new()),
                error: reason,
                duration_ms: 0,
                finished_at: iso_now(),
                client_id: None,
                method_id: None,
                method_name: None,
            },
        )
        .await;

        let _ = sequence_id;
        return Ok(json!({ "requestId": request_id, "status": "cancelled" }));
    }

    Err(RequestError::new(
        "requestId or sequenceId is required.",
        StatusCode::BAD_REQUEST,
    ))
}

fn method_allows_parallel_execution_locked(
    inner: &ServerState,
    client_id: &str,
    method_id: &str,
    method_name: &str,
) -> bool {
    let Some(client) = inner.unity_clients.get(client_id) else {
        return false;
    };

    client
        .methods
        .iter()
        .find(|method| registered_method_matches(method, method_id, method_name))
        .is_some_and(registered_method_allows_parallel_execution)
}

fn registered_method_matches(method: &Value, method_id: &str, method_name: &str) -> bool {
    let candidates = [
        method.get("id").and_then(Value::as_str),
        method.get("name").and_then(Value::as_str),
        method.get("methodId").and_then(Value::as_str),
        method.get("methodName").and_then(Value::as_str),
    ];

    candidates.into_iter().flatten().any(|candidate| {
        (!method_id.is_empty() && candidate == method_id)
            || (!method_name.is_empty() && candidate == method_name)
    })
}

fn registered_method_allows_parallel_execution(method: &Value) -> bool {
    method
        .get("allowParallelExecution")
        .or_else(|| method.get("allowParallel"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn ensure_sequence_not_cancelled_locked(
    inner: &ServerState,
    sequence_id: &str,
) -> Result<(), RequestError> {
    if inner
        .active_sequences
        .get(sequence_id)
        .is_some_and(|sequence| sequence.cancelled)
    {
        return Err(RequestError::new(
            format!("Sequence {sequence_id} is already cancelled."),
            StatusCode::CONFLICT,
        ));
    }

    Ok(())
}

fn find_pending_sequence_request_locked(
    inner: &ServerState,
    sequence_id: &str,
    owner_id: &str,
) -> Option<String> {
    inner
        .pending_executions
        .iter()
        .find(|(_, pending)| {
            pending.owner_id == owner_id
                && pending.started.sequence_id.as_deref() == Some(sequence_id)
        })
        .map(|(request_id, _)| request_id.clone())
}

pub(crate) fn prepare_client_for_execution_locked(
    inner: &mut ServerState,
    client_id: &str,
    controller: &ControllerIdentity,
    allow_busy: bool,
    allow_parallel_execution: bool,
) -> Result<Sender, RequestError> {
    let (sender, should_acquire_lock) = {
        let client = inner.unity_clients.get(client_id).ok_or_else(|| {
            RequestError::new(
                format!("Unity client {client_id} is not online."),
                StatusCode::NOT_FOUND,
            )
        })?;

        if client.sender.is_closed() {
            return Err(RequestError::new(
                format!("Unity client {client_id} is not online."),
                StatusCode::NOT_FOUND,
            ));
        }

        if !client.available {
            return Err(RequestError::new(
                format!("Unity client {client_id} is unavailable."),
                StatusCode::CONFLICT,
            ));
        }

        if client
            .lock
            .as_ref()
            .is_some_and(|lock| lock.owner_id != controller.id)
            && !allow_parallel_execution
        {
            return Err(RequestError::new(
                format!("Unity client {client_id} is locked by another controller."),
                StatusCode::LOCKED,
            ));
        }

        if client.client_busy && !allow_parallel_execution {
            let detail = if client.current_request_id.is_empty() {
                "Unity client reports local busy.".to_string()
            } else {
                format!(
                    "Unity client reports local busy with request {}.",
                    client.current_request_id
                )
            };
            return Err(RequestError::new(
                format!("Unity client {client_id} is already busy. {detail}"),
                StatusCode::CONFLICT,
            ));
        }

        if !allow_busy
            && !allow_parallel_execution
            && is_client_running_locked(inner, client_id, None)
        {
            return Err(RequestError::new(
                format!("Unity client {client_id} is already running a request."),
                StatusCode::CONFLICT,
            ));
        }

        let should_acquire_lock = !allow_parallel_execution
            || client
                .lock
                .as_ref()
                .is_some_and(|lock| lock.owner_id == controller.id);
        (client.sender.clone(), should_acquire_lock)
    };

    if should_acquire_lock {
        acquire_client_lock_locked(inner, client_id, controller);
    }
    Ok(sender)
}

fn acquire_client_lock_locked(
    inner: &mut ServerState,
    client_id: &str,
    controller: &ControllerIdentity,
) {
    let mut lock_event = None;
    let now = Utc::now();
    if let Some(client) = inner.unity_clients.get_mut(client_id) {
        if let Some(lock) = client.lock.as_mut()
            && lock.owner_id == controller.id
        {
            lock.last_seen_at = now;
            return;
        }

        let lock = ClientLock {
            owner_id: controller.id.clone(),
            owner_type: controller.controller_type.clone(),
            acquired_at: now,
            last_seen_at: now,
        };
        client.lock = Some(lock.clone());
        lock_event = Some((client.client_id.clone(), lock));
    }

    if let Some((client_id, lock)) = lock_event {
        broadcast_web_locked(
            inner,
            &json!({
                "type": "client_locked",
                "clientId": client_id,
                "lock": to_public_lock_locked(inner, Some(&lock)),
            }),
        );
        LogEvent::new("client_locked")
            .client_id_str(&client_id)
            .controller_id(Some(lock.owner_id))
            .field("controllerType", lock.owner_type)
            .emit();
    }
}

pub(crate) fn release_client_lock_if_idle_locked(
    inner: &mut ServerState,
    client_id: &str,
    reason: &str,
) {
    let should_release = inner
        .unity_clients
        .get(client_id)
        .and_then(|client| client.lock.as_ref())
        .is_some_and(|lock| !is_client_running_locked(inner, client_id, Some(&lock.owner_id)));

    if !should_release {
        return;
    }

    let mut event = None;
    if let Some(client) = inner.unity_clients.get_mut(client_id)
        && let Some(lock) = client.lock.take()
    {
        event = Some((client.client_id.clone(), lock));
    }

    if let Some((client_id, lock)) = event {
        broadcast_web_locked(
            inner,
            &json!({
                "type": "client_unlocked",
                "clientId": client_id,
                "reason": reason,
                "lock": to_public_lock_locked(inner, Some(&lock)),
            }),
        );
        LogEvent::new("client_unlocked")
            .client_id_str(&client_id)
            .controller_id(Some(lock.owner_id))
            .field("controllerType", lock.owner_type)
            .field("reason", reason)
            .emit();
    }
}

pub(crate) fn release_idle_locks_for_controller_locked(
    inner: &mut ServerState,
    owner_id: &str,
    reason: &str,
) {
    let client_ids = inner
        .unity_clients
        .values()
        .filter(|client| {
            client
                .lock
                .as_ref()
                .is_some_and(|lock| lock.owner_id == owner_id)
        })
        .map(|client| client.client_id.clone())
        .collect::<Vec<_>>();

    for client_id in client_ids {
        let should_release = !is_client_running_locked(inner, &client_id, Some(owner_id));
        if should_release {
            let mut event = None;
            if let Some(client) = inner.unity_clients.get_mut(&client_id)
                && let Some(lock) = client.lock.take()
            {
                event = Some((client.client_id.clone(), lock));
            }

            if let Some((client_id, lock)) = event {
                broadcast_web_locked(
                    inner,
                    &json!({
                    "type": "client_unlocked",
                    "clientId": client_id,
                    "reason": reason,
                    "lock": to_public_lock_locked(inner, Some(&lock)),
                    }),
                );
                LogEvent::new("client_unlocked")
                    .client_id_str(&client_id)
                    .controller_id(Some(lock.owner_id))
                    .field("controllerType", lock.owner_type)
                    .field("reason", reason)
                    .emit();
            }
        }
    }
}

pub(crate) fn is_client_running_locked(
    inner: &ServerState,
    client_id: &str,
    owner_id: Option<&str>,
) -> bool {
    let locally_busy = inner
        .unity_clients
        .get(client_id)
        .is_some_and(|client| client.client_busy);
    if locally_busy {
        return true;
    }

    inner.pending_executions.values().any(|pending| {
        pending.client_id == client_id
            && owner_id.is_none_or(|owner_id| pending.owner_id == owner_id)
    }) || inner.active_sequences.values().any(|sequence| {
        sequence.client_id == client_id
            && sequence.status == "running"
            && owner_id.is_none_or(|owner_id| sequence.owner_id == owner_id)
    })
}

fn business_failure_reason(result: &Value) -> Option<String> {
    match result {
        Value::String(value) => business_failure_reason_from_text(value),
        Value::Object(_) => business_failure_reason_from_structured_value(result),
        _ => None,
    }
}

fn business_failure_reason_from_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("failed:"))
    {
        let reason = trimmed[7..].trim();
        return Some(if reason.is_empty() {
            "QaTest method returned failed.".to_string()
        } else {
            reason.to_string()
        });
    }

    if trimmed.eq_ignore_ascii_case("false") {
        return Some("QaTest method returned False.".to_string());
    }

    if !trimmed.starts_with('{') {
        return None;
    }

    serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|value| business_failure_reason_from_structured_value(&value))
}

fn business_failure_reason_from_structured_value(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let has_ok = object.contains_key("ok");
    let ok = object.get("ok").and_then(Value::as_bool).unwrap_or(true);
    let status = object.get("status").and_then(Value::as_str).unwrap_or("");
    let failed_status = matches!(
        status.to_ascii_lowercase().as_str(),
        "failed" | "failure" | "error" | "unsupported" | "cancelled" | "canceled"
    );

    if !failed_status && (!has_ok || ok) {
        return None;
    }

    for key in ["message", "error", "code", "status"] {
        if let Some(value) = object.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    Some("QaTest structured result failed.".to_string())
}

fn assert_owner(owner_id: &str, controller: &ControllerIdentity) -> Result<(), RequestError> {
    if owner_id != controller.id {
        return Err(RequestError::new(
            "Only the controller that owns this execution can stop it.",
            StatusCode::FORBIDDEN,
        ));
    }
    Ok(())
}

pub(crate) fn add_history_locked(inner: &mut ServerState, item: ExecutionRecord) {
    if !item.request_id.is_empty()
        && let Some(existing_index) = inner
            .execution_history
            .iter()
            .position(|history_item| history_item.request_id == item.request_id)
    {
        inner.execution_history[existing_index] = item;
        return;
    }

    inner.execution_history.insert(0, item);
    if inner.execution_history.len() > HISTORY_LIMIT {
        inner.execution_history.truncate(HISTORY_LIMIT);
    }
}

fn find_history_by_request_id_locked<'a>(
    inner: &'a ServerState,
    request_id: &str,
) -> Option<&'a ExecutionRecord> {
    if request_id.is_empty() {
        return None;
    }
    inner
        .execution_history
        .iter()
        .find(|item| item.request_id == request_id)
}

pub(crate) fn build_server_failure_result(
    client_id: &str,
    method_id: Option<&str>,
    method_name: Option<&str>,
    method_arguments: &Value,
    error: &str,
    meta: &ExecutionMeta,
) -> ExecutionRecord {
    ExecutionRecord {
        request_id: uuid(),
        client_id: client_id.to_string(),
        client_name: None,
        client_ip_address: None,
        client_remote_address: None,
        client_ip_addresses: None,
        method_id: method_id.or(method_name).unwrap_or_default().to_string(),
        method_name: method_name.or(method_id).unwrap_or_default().to_string(),
        arguments: normalize_arguments(method_arguments),
        status: "failed".to_string(),
        allow_parallel_execution: false,
        success: Some(false),
        result: Some(Value::String(String::new())),
        error: Some(error.to_string()),
        duration_ms: Some(0),
        started_at: None,
        finished_at: Some(iso_now()),
        sequence_id: meta.sequence_id.clone(),
        step_id: meta.step_id.clone(),
        step_index: meta.step_index,
        step_number: meta.step_number,
        total_steps: meta.total_steps,
    }
}

pub(crate) fn apply_client_snapshot_locked(inner: &ServerState, record: &mut ExecutionRecord) {
    let Some(client) = inner.unity_clients.get(&record.client_id) else {
        return;
    };
    apply_client_snapshot(record, client);
}

fn apply_client_snapshot(record: &mut ExecutionRecord, client: &UnityClient) {
    if record.client_name.is_none() {
        record.client_name = non_empty_string(&client.name);
    }
    if record.client_ip_address.is_none() {
        record.client_ip_address = non_empty_string(&client.ip_address);
    }
    if record.client_remote_address.is_none() {
        record.client_remote_address = non_empty_string(&client.remote_address);
    }
    if record.client_ip_addresses.is_none() && !client.ip_addresses.is_empty() {
        record.client_ip_addresses = Some(client.ip_addresses.clone());
    }
}

fn clear_client_snapshot(record: &mut ExecutionRecord) {
    record.client_name = None;
    record.client_ip_address = None;
    record.client_remote_address = None;
    record.client_ip_addresses = None;
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
