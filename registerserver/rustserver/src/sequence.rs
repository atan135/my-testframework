use std::{sync::Arc, time::Duration};

use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio::{sync::Notify, time};

use crate::{
    execution::{
        add_history_locked, build_server_failure_result, dispatch_unity_execution,
        prepare_client_for_execution_locked, release_client_lock_if_idle_locked,
    },
    logging::LogEvent,
    messaging::broadcast_web_locked,
    state::{
        ControllerIdentity, DispatchRequest, ExecutionMeta, MAX_SEQUENCE_STEP_DELAY_MS,
        NormalizedStep, PublicSequence, RequestError, SequenceState, SequenceStep, SharedState,
    },
    util::{
        iso_now, normalize_arguments, normalize_delay_ms, uuid, value_to_opt_string,
        value_to_string, value_to_u64,
    },
};

pub(crate) async fn run_execution_sequence(
    state: SharedState,
    controller: ControllerIdentity,
    payload: Value,
) -> Result<(), RequestError> {
    let sequence_id = value_to_opt_string(payload.get("sequenceId")).unwrap_or_else(uuid);
    let client_id = value_to_string(payload.get("clientId"));
    let steps = normalize_sequence_steps(payload.get("steps"));
    let stop_on_failure = payload
        .get("stopOnFailure")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let step_delay_ms = normalize_delay_ms(payload.get("stepDelayMs"), MAX_SEQUENCE_STEP_DELAY_MS);
    let sequence_timeout_ms = value_to_u64(payload.get("timeoutMs"));
    let started_at = iso_now();

    if client_id.is_empty() {
        LogEvent::warn("sequence_rejected")
            .sequence_id(Some(sequence_id))
            .controller(&controller)
            .field("error", "clientId is required.")
            .emit();
        return Err(RequestError::new(
            "clientId is required.",
            StatusCode::BAD_REQUEST,
        ));
    }

    if steps.is_empty() {
        LogEvent::warn("sequence_rejected")
            .sequence_id(Some(sequence_id))
            .client_id_str(&client_id)
            .controller(&controller)
            .field("error", "At least one request step is required.")
            .emit();
        return Err(RequestError::new(
            "At least one request step is required.",
            StatusCode::BAD_REQUEST,
        ));
    }

    {
        let mut inner = state.inner.write().await;
        if let Err(error) =
            prepare_client_for_execution_locked(&mut inner, &client_id, &controller, false, false)
        {
            LogEvent::warn("sequence_rejected")
                .sequence_id(Some(sequence_id))
                .client_id_str(&client_id)
                .controller(&controller)
                .field("error", error.message.clone())
                .field("statusCode", error.status_code.as_u16())
                .emit();
            return Err(error);
        }
    }

    let notify = Arc::new(Notify::new());
    let sequence_state = SequenceState {
        client_id: client_id.clone(),
        owner_id: controller.id.clone(),
        status: "running".to_string(),
        cancelled: false,
        cancel_reason: String::new(),
        current_request_id: String::new(),
        notify: notify.clone(),
    };

    let mut sequence = PublicSequence {
        sequence_id: sequence_id.clone(),
        client_id: client_id.clone(),
        status: "running".to_string(),
        stop_on_failure,
        step_delay_ms,
        total_steps: steps.len(),
        completed_steps: 0,
        success_count: 0,
        failed_count: 0,
        cancelled_count: 0,
        started_at,
        steps: steps
            .iter()
            .enumerate()
            .map(|(index, step)| to_public_sequence_step(step, index, steps.len()))
            .collect(),
        results: Vec::new(),
        cancel_reason: None,
        finished_at: None,
    };

    {
        let mut inner = state.inner.write().await;
        inner
            .active_sequences
            .insert(sequence_id.clone(), sequence_state);
        broadcast_web_locked(
            &inner,
            &json!({ "type": "sequence_started", "sequence": sequence }),
        );
    }
    LogEvent::new("sequence_started")
        .sequence_id(Some(sequence_id.clone()))
        .client_id_str(&client_id)
        .controller(&controller)
        .field("totalSteps", steps.len())
        .field("stopOnFailure", stop_on_failure)
        .field("stepDelayMs", step_delay_ms)
        .field("timeoutMs", sequence_timeout_ms)
        .emit();

    for (index, step) in steps.iter().enumerate() {
        if is_sequence_cancelled(state.clone(), &sequence_id).await {
            break;
        }

        if index > 0 && step_delay_ms > 0 {
            sleep_or_cancel(step_delay_ms, notify.clone()).await;
            if is_sequence_cancelled(state.clone(), &sequence_id).await {
                break;
            }
        }

        let step_meta = ExecutionMeta {
            sequence_id: Some(sequence_id.clone()),
            step_id: Some(step.step_id.clone()),
            step_index: Some(index),
            step_number: Some(index + 1),
            total_steps: Some(steps.len()),
        };

        let result = match dispatch_unity_execution(
            state.clone(),
            DispatchRequest {
                client_id: client_id.clone(),
                method_id: Some(step.method_id.clone()),
                method_name: Some(step.method_name.clone()),
                method_arguments: Value::Array(
                    step.arguments
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect::<Vec<_>>(),
                ),
                timeout_ms: step.timeout_ms.or(sequence_timeout_ms),
                controller: controller.clone(),
                wait_for_result: true,
                allow_busy: true,
                allow_parallel_execution: step.allow_parallel_execution,
                meta: step_meta.clone(),
            },
        )
        .await
        {
            Ok(dispatch) => {
                {
                    let mut inner = state.inner.write().await;
                    if let Some(active) = inner.active_sequences.get_mut(&sequence_id) {
                        active.current_request_id = dispatch.request_id.clone();
                    }
                    broadcast_web_locked(
                        &inner,
                        &json!({
                            "type": "sequence_step_started",
                            "sequenceId": sequence_id,
                            "step": to_public_sequence_step(step, index, steps.len()),
                            "execution": dispatch.started,
                        }),
                    );
                }
                LogEvent::new("sequence_step_started")
                    .request_id_str(&dispatch.request_id)
                    .client_id_str(&client_id)
                    .controller(&controller)
                    .sequence_id(Some(sequence_id.clone()))
                    .field("stepId", step.step_id.clone())
                    .field("stepIndex", index)
                    .field("stepNumber", index + 1)
                    .field("totalSteps", steps.len())
                    .field("methodId", step.method_id.clone())
                    .emit();

                let result = match dispatch.result_rx {
                    Some(rx) => match rx.await {
                        Ok(result) => result,
                        Err(_) => build_server_failure_result(
                            &client_id,
                            Some(&step.method_id),
                            Some(&step.method_name),
                            &Value::Array(
                                step.arguments.iter().cloned().map(Value::String).collect(),
                            ),
                            "Execution result channel closed.",
                            &step_meta,
                        ),
                    },
                    None => build_server_failure_result(
                        &client_id,
                        Some(&step.method_id),
                        Some(&step.method_name),
                        &Value::Array(step.arguments.iter().cloned().map(Value::String).collect()),
                        "Execution did not return a result waiter.",
                        &step_meta,
                    ),
                };

                {
                    let mut inner = state.inner.write().await;
                    if let Some(active) = inner.active_sequences.get_mut(&sequence_id) {
                        active.current_request_id.clear();
                    }
                }
                result
            }
            Err(error) => {
                let result = build_server_failure_result(
                    &client_id,
                    Some(&step.method_id),
                    Some(&step.method_name),
                    &Value::Array(step.arguments.iter().cloned().map(Value::String).collect()),
                    &error.message,
                    &step_meta,
                );
                let mut inner = state.inner.write().await;
                add_history_locked(&mut inner, result.clone());
                state
                    .archive
                    .enqueue_locked(&inner, &result, Some(&controller));
                broadcast_web_locked(&inner, &json!({ "type": "qa_result", "result": result }));
                LogEvent::warn("sequence_step_rejected")
                    .request_id_str(&result.request_id)
                    .client_id_str(&client_id)
                    .controller(&controller)
                    .sequence_id(Some(sequence_id.clone()))
                    .field("stepId", step.step_id.clone())
                    .field("stepIndex", index)
                    .field("methodId", step.method_id.clone())
                    .field("error", error.message)
                    .emit();
                result
            }
        };

        sequence.completed_steps += 1;
        if result.status == "cancelled" {
            sequence.cancelled_count += 1;
        } else if result.success.unwrap_or(false) {
            sequence.success_count += 1;
        } else {
            sequence.failed_count += 1;
        }
        sequence.results.push(result.clone());

        {
            let inner = state.inner.read().await;
            broadcast_web_locked(
                &inner,
                &json!({
                    "type": "sequence_step_result",
                    "sequenceId": sequence_id,
                    "step": to_public_sequence_step(step, index, steps.len()),
                    "result": result,
                }),
            );
        }
        LogEvent::new("sequence_step_finished")
            .request_id_str(&result.request_id)
            .client_id_str(&client_id)
            .controller(&controller)
            .sequence_id(Some(sequence_id.clone()))
            .field("stepId", step.step_id.clone())
            .field("stepIndex", index)
            .field("stepNumber", index + 1)
            .field("totalSteps", steps.len())
            .field("methodId", result.method_id.clone())
            .field("status", result.status.clone())
            .field("success", result.success.unwrap_or(false))
            .field("durationMs", result.duration_ms.unwrap_or(0))
            .emit();

        if result.status == "cancelled" {
            mark_sequence_cancelled(state.clone(), &sequence_id, "Stopped by controller.").await;
            break;
        }

        if stop_on_failure && !result.success.unwrap_or(false) {
            break;
        }
    }

    let (cancelled, cancel_reason) = get_sequence_cancel_state(state.clone(), &sequence_id).await;
    if cancelled {
        sequence.status = "cancelled".to_string();
        sequence.cancel_reason = Some(if cancel_reason.is_empty() {
            "Stopped by controller.".to_string()
        } else {
            cancel_reason
        });
    } else {
        sequence.status = if sequence.failed_count > 0 {
            "failed".to_string()
        } else {
            "success".to_string()
        };
    }
    sequence.finished_at = Some(iso_now());

    {
        let mut inner = state.inner.write().await;
        inner.active_sequences.remove(&sequence_id);
        broadcast_web_locked(
            &inner,
            &json!({ "type": "sequence_finished", "sequence": sequence }),
        );
        release_client_lock_if_idle_locked(&mut inner, &client_id, "sequence_finished");
    }
    LogEvent::new("sequence_finished")
        .sequence_id(Some(sequence.sequence_id.clone()))
        .client_id_str(&client_id)
        .controller(&controller)
        .field("status", sequence.status.clone())
        .field("totalSteps", sequence.total_steps)
        .field("completedSteps", sequence.completed_steps)
        .field("successCount", sequence.success_count)
        .field("failedCount", sequence.failed_count)
        .field("cancelledCount", sequence.cancelled_count)
        .field_opt("cancelReason", sequence.cancel_reason.clone())
        .emit();

    Ok(())
}

async fn is_sequence_cancelled(state: SharedState, sequence_id: &str) -> bool {
    let inner = state.inner.read().await;
    inner
        .active_sequences
        .get(sequence_id)
        .is_some_and(|sequence| sequence.cancelled)
}

async fn mark_sequence_cancelled(state: SharedState, sequence_id: &str, reason: &str) {
    let mut inner = state.inner.write().await;
    if let Some(sequence) = inner.active_sequences.get_mut(sequence_id) {
        sequence.cancelled = true;
        if sequence.cancel_reason.is_empty() {
            sequence.cancel_reason = reason.to_string();
        }
        sequence.notify.notify_waiters();
    }
}

async fn get_sequence_cancel_state(state: SharedState, sequence_id: &str) -> (bool, String) {
    let inner = state.inner.read().await;
    inner
        .active_sequences
        .get(sequence_id)
        .map(|sequence| (sequence.cancelled, sequence.cancel_reason.clone()))
        .unwrap_or((false, String::new()))
}

async fn sleep_or_cancel(ms: u64, notify: Arc<Notify>) {
    tokio::select! {
        _ = time::sleep(Duration::from_millis(ms)) => {}
        _ = notify.notified() => {}
    }
}

fn normalize_sequence_steps(raw_steps: Option<&Value>) -> Vec<NormalizedStep> {
    let Some(steps) = raw_steps.and_then(Value::as_array) else {
        return Vec::new();
    };

    steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            let method_id = value_to_opt_string(step.get("methodId"))
                .or_else(|| value_to_opt_string(step.get("methodName")))?;
            if method_id.is_empty() {
                return None;
            }
            Some(NormalizedStep {
                step_id: value_to_opt_string(step.get("stepId")).unwrap_or_else(uuid),
                method_name: value_to_opt_string(step.get("methodName"))
                    .or_else(|| value_to_opt_string(step.get("methodId")))
                    .unwrap_or_else(|| format!("Step {}", index + 1)),
                method_id,
                arguments: normalize_arguments(step.get("arguments").unwrap_or(&Value::Null)),
                timeout_ms: value_to_u64(step.get("timeoutMs")),
                allow_parallel_execution: step
                    .get("allowParallelExecution")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn to_public_sequence_step(
    step: &NormalizedStep,
    index: usize,
    total_steps: usize,
) -> SequenceStep {
    SequenceStep {
        step_id: step.step_id.clone(),
        step_index: index,
        step_number: index + 1,
        total_steps,
        method_id: step.method_id.clone(),
        method_name: step.method_name.clone(),
        arguments: step.arguments.clone(),
    }
}
