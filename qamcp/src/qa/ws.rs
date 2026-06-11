use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    sync::Mutex,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::{
    config::get_qa_server_url,
    constants::{MAX_SEQUENCE_STEP_DELAY_MS, WS_CONNECT_TIMEOUT_MS, controller_id},
    qa::api::request_json,
};

type QaSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone, Default)]
pub struct EventSessionStore {
    sessions: Arc<Mutex<HashMap<String, EventSessionHandle>>>,
}

struct EventSessionHandle {
    data: Arc<Mutex<EventSessionData>>,
    task: JoinHandle<()>,
}

#[derive(Debug)]
struct EventSessionData {
    session_id: String,
    event_types: Vec<String>,
    include_snapshot: bool,
    max_events: usize,
    ttl_ms: u64,
    created_at: String,
    expires_at: String,
    last_polled_at: Option<String>,
    events: VecDeque<Value>,
    next_index: u64,
    dropped_events: u64,
    closed: bool,
    close_code: Option<u16>,
    close_reason: String,
    last_error: String,
}

pub async fn execute_method_via_websocket(
    client_id: String,
    method_id: String,
    method_name: Option<String>,
    method_arguments: Vec<Value>,
    timeout_ms: u64,
) -> Result<Value> {
    let mut socket = open_qa_websocket().await?;
    let mut request_id = String::new();
    let mut accepted_execution = Value::Null;
    let message = json!({
        "type": "execute",
        "clientId": client_id,
        "methodId": method_id,
        "methodName": method_name.unwrap_or_else(|| method_id.clone()),
        "arguments": normalize_arguments(&method_arguments),
    });

    let operation = async {
        send_ws_json(&mut socket, &message).await?;
        loop {
            let message = read_ws_json(&mut socket).await?;
            match message.get("type").and_then(Value::as_str) {
                Some("execute_rejected") => {
                    bail!(
                        "{}",
                        message
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("QA server rejected the execution request.")
                    );
                }
                Some("execute_accepted") => {
                    accepted_execution = message.get("execution").cloned().unwrap_or(Value::Null);
                    request_id = accepted_execution
                        .get("requestId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }
                Some("qa_result")
                    if !request_id.is_empty()
                        && message
                            .get("result")
                            .and_then(|result| result.get("requestId"))
                            .and_then(Value::as_str)
                            == Some(request_id.as_str()) =>
                {
                    return Ok(json!({
                        "execution": accepted_execution,
                        "result": message.get("result").cloned().unwrap_or(Value::Null),
                    }));
                }
                _ => {}
            }
        }
    };

    timeout(Duration::from_millis(timeout_ms), operation)
        .await
        .map_err(|_| anyhow!("Timed out waiting for QaTest result after {timeout_ms} ms."))?
}

pub async fn execute_sequence_via_websocket(
    client_id: String,
    steps: Vec<Value>,
    stop_on_failure: bool,
    step_delay_ms: u64,
    timeout_ms: u64,
) -> Result<Value> {
    let mut socket = open_qa_websocket().await?;
    let sequence_id = Uuid::new_v4().to_string();
    let normalized_steps = steps
        .into_iter()
        .map(normalize_sequence_step)
        .collect::<Result<Vec<_>>>()?;
    let message = json!({
        "type": "execute_sequence",
        "sequenceId": sequence_id,
        "clientId": client_id,
        "stopOnFailure": stop_on_failure,
        "stepDelayMs": normalize_step_delay_ms(step_delay_ms),
        "steps": normalized_steps,
    });
    let mut started_steps = Vec::new();
    let mut step_results = Vec::new();
    let mut qa_results = Vec::new();

    let operation = async {
        send_ws_json(&mut socket, &message).await?;
        loop {
            let message = read_ws_json(&mut socket).await?;
            match message.get("type").and_then(Value::as_str) {
                Some("error") => {
                    bail!(
                        "{}",
                        message
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("QA server reported a sequence execution error.")
                    );
                }
                Some("sequence_step_started")
                    if message.get("sequenceId").and_then(Value::as_str)
                        == Some(sequence_id.as_str()) =>
                {
                    started_steps.push(json!({
                        "step": message.get("step").cloned().unwrap_or(Value::Null),
                        "execution": message.get("execution").cloned().unwrap_or(Value::Null),
                    }));
                }
                Some("sequence_step_result")
                    if message.get("sequenceId").and_then(Value::as_str)
                        == Some(sequence_id.as_str()) =>
                {
                    step_results.push(json!({
                        "step": message.get("step").cloned().unwrap_or(Value::Null),
                        "result": message.get("result").cloned().unwrap_or(Value::Null),
                    }));
                }
                Some("qa_result")
                    if message
                        .get("result")
                        .and_then(|result| result.get("sequenceId"))
                        .and_then(Value::as_str)
                        == Some(sequence_id.as_str()) =>
                {
                    qa_results.push(message.get("result").cloned().unwrap_or(Value::Null));
                }
                Some("sequence_finished")
                    if message
                        .get("sequence")
                        .and_then(|sequence| sequence.get("sequenceId"))
                        .and_then(Value::as_str)
                        == Some(sequence_id.as_str()) =>
                {
                    return Ok(json!({
                        "sequenceId": sequence_id,
                        "sequence": message.get("sequence").cloned().unwrap_or(Value::Null),
                        "startedSteps": started_steps,
                        "stepResults": step_results,
                        "qaResults": qa_results,
                    }));
                }
                _ => {}
            }
        }
    };

    timeout(Duration::from_millis(timeout_ms), operation)
        .await
        .map_err(|_| anyhow!("Timed out waiting for QaTest sequence after {timeout_ms} ms."))?
}

pub async fn stop_via_websocket(
    request_id: Option<String>,
    sequence_id: Option<String>,
    reason: String,
    timeout_ms: u64,
) -> Result<Value> {
    let mut socket = open_qa_websocket().await?;
    let message = json!({
        "type": if sequence_id.is_some() { "stop_sequence" } else { "stop_execution" },
        "requestId": request_id,
        "sequenceId": sequence_id,
        "reason": reason,
    });

    let operation = async {
        send_ws_json(&mut socket, &message).await?;
        loop {
            let message = read_ws_json(&mut socket).await?;
            match message.get("type").and_then(Value::as_str) {
                Some("stop_rejected") => {
                    bail!(
                        "{}",
                        message
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("QA server rejected the stop request.")
                    );
                }
                Some("stop_accepted") => {
                    return Ok(json!({
                        "controllerId": controller_id(),
                        "acknowledgement": message,
                    }));
                }
                _ => {}
            }
        }
    };

    timeout(Duration::from_millis(timeout_ms), operation)
        .await
        .map_err(|_| {
            anyhow!("Timed out waiting for QA stop acknowledgement after {timeout_ms} ms.")
        })?
}

pub async fn watch_events_via_websocket(
    duration_ms: u64,
    event_types: Vec<String>,
    include_snapshot: bool,
    max_events: usize,
) -> Result<Value> {
    let mut socket = open_qa_websocket().await?;
    let normalized_event_types = normalize_event_types(event_types);
    let timer = sleep(Duration::from_millis(duration_ms));
    tokio::pin!(timer);

    let mut events = Vec::new();
    let mut dropped_events = 0_u64;
    let mut close_code = Value::Null;
    let mut close_reason = String::new();
    let mut status = "completed";

    loop {
        tokio::select! {
            _ = &mut timer => break,
            message = socket.next() => {
                match message {
                    Some(Ok(message)) => {
                        if let Message::Close(frame) = &message {
                            status = "closed";
                            if let Some(frame) = frame {
                                close_code = json!(u16::from(frame.code));
                                close_reason = frame.reason.to_string();
                            }
                            break;
                        }
                        let Some(message) = parse_ws_message(message) else {
                            continue;
                        };
                        if !should_keep_event(&message, &normalized_event_types, include_snapshot) {
                            continue;
                        }
                        if events.len() >= max_events {
                            dropped_events += 1;
                            continue;
                        }
                        events.push(to_event_record(&message, (events.len() + 1) as u64));
                    }
                    Some(Err(error)) => return Err(error).context("QA server WebSocket read failed"),
                    None => {
                        status = "closed";
                        break;
                    }
                }
            }
        }
    }

    Ok(json!({
        "status": status,
        "durationMs": duration_ms,
        "eventTypes": normalized_event_types,
        "includeSnapshot": include_snapshot,
        "count": events.len(),
        "droppedEvents": dropped_events,
        "truncated": dropped_events > 0,
        "closeCode": close_code,
        "closeReason": close_reason,
        "events": events,
    }))
}

pub async fn wait_for_result_via_websocket(filters: Value) -> Result<Value> {
    ensure_result_wait_filter(&filters)?;

    if filters
        .get("includeHistory")
        .and_then(Value::as_bool)
        .unwrap_or(true)
        && filters.get("requestId").and_then(Value::as_str).is_some()
    {
        if let Some(result) = find_historical_result(&filters).await? {
            return Ok(json!({
                "source": "history",
                "eventType": "qa_result",
                "result": result,
            }));
        }
    }

    let timeout_ms = filters
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(crate::constants::WAIT_RESULT_TIMEOUT_MS);
    let mut socket = open_qa_websocket().await?;
    let operation = async {
        loop {
            let message = read_ws_json(&mut socket).await?;
            match message.get("type").and_then(Value::as_str) {
                Some("qa_result")
                    if matches_result_filter(
                        message.get("result").unwrap_or(&Value::Null),
                        &filters,
                    ) =>
                {
                    return Ok(json!({
                        "source": "websocket",
                        "eventType": "qa_result",
                        "result": message.get("result").cloned().unwrap_or(Value::Null),
                    }));
                }
                Some("sequence_finished")
                    if matches_sequence_filter(
                        message.get("sequence").unwrap_or(&Value::Null),
                        &filters,
                    ) =>
                {
                    return Ok(json!({
                        "source": "websocket",
                        "eventType": "sequence_finished",
                        "sequence": message.get("sequence").cloned().unwrap_or(Value::Null),
                    }));
                }
                _ => {}
            }
        }
    };

    timeout(Duration::from_millis(timeout_ms), operation)
        .await
        .map_err(|_| anyhow!("Timed out waiting for QA result after {timeout_ms} ms."))?
}

impl EventSessionStore {
    pub async fn open_event_session(
        &self,
        event_types: Vec<String>,
        include_snapshot: bool,
        max_events: usize,
        ttl_ms: u64,
    ) -> Result<Value> {
        let mut socket = open_qa_websocket().await?;
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let data = Arc::new(Mutex::new(EventSessionData {
            session_id: session_id.clone(),
            event_types: normalize_event_types(event_types),
            include_snapshot,
            max_events,
            ttl_ms,
            created_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::milliseconds(ttl_ms as i64)).to_rfc3339(),
            last_polled_at: None,
            events: VecDeque::new(),
            next_index: 1,
            dropped_events: 0,
            closed: false,
            close_code: None,
            close_reason: String::new(),
            last_error: String::new(),
        }));
        let task_data = Arc::clone(&data);
        let task = tokio::spawn(async move {
            let ttl = sleep(Duration::from_millis(ttl_ms));
            tokio::pin!(ttl);
            loop {
                tokio::select! {
                    _ = &mut ttl => {
                        mark_session_closed(&task_data, None, "event session ttl expired").await;
                        break;
                    }
                    message = socket.next() => {
                        match message {
                            Some(Ok(message)) => {
                                if let Message::Close(frame) = &message {
                                    let (code, reason) = frame.as_ref()
                                        .map(|frame| (Some(u16::from(frame.code)), frame.reason.to_string()))
                                        .unwrap_or((None, String::new()));
                                    mark_session_closed(&task_data, code, &reason).await;
                                    break;
                                }
                                if let Some(message) = parse_ws_message(message) {
                                    record_session_event(&task_data, message).await;
                                }
                            }
                            Some(Err(error)) => {
                                let error = error.to_string();
                                {
                                    let mut session = task_data.lock().await;
                                    session.last_error = error.clone();
                                }
                                record_session_event(
                                    &task_data,
                                    json!({ "type": "qamcp_session_error", "error": error }),
                                )
                                .await;
                                mark_session_closed(&task_data, None, "websocket error").await;
                                break;
                            }
                            None => {
                                mark_session_closed(&task_data, None, "").await;
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.sessions.lock().await.insert(
            session_id,
            EventSessionHandle {
                data: Arc::clone(&data),
                task,
            },
        );
        summarize_event_session(&data).await
    }

    pub async fn poll_event_session(
        &self,
        session_id: &str,
        cursor: u64,
        max_events: usize,
    ) -> Result<Value> {
        let data = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(session_id)
                .map(|handle| Arc::clone(&handle.data))
                .ok_or_else(|| anyhow!("QA event session not found: {session_id}"))?
        };

        let mut session = data.lock().await;
        session.last_polled_at = Some(Utc::now().to_rfc3339());
        let first_available_index = session
            .events
            .front()
            .and_then(|event| event.get("index").and_then(Value::as_u64))
            .unwrap_or(session.next_index);
        let lost_event_count = if cursor > 0 && cursor < first_available_index.saturating_sub(1) {
            first_available_index - cursor - 1
        } else {
            0
        };
        let matching_events = session
            .events
            .iter()
            .filter(|event| event.get("index").and_then(Value::as_u64).unwrap_or(0) > cursor)
            .cloned()
            .collect::<Vec<_>>();
        let events = matching_events
            .iter()
            .take(max_events)
            .cloned()
            .collect::<Vec<_>>();
        let next_cursor = events
            .last()
            .and_then(|event| event.get("index").and_then(Value::as_u64))
            .unwrap_or(cursor);
        let mut summary = summarize_event_session_locked(&session);
        merge_json_object(
            &mut summary,
            json!({
                "cursor": cursor,
                "nextCursor": next_cursor,
                "count": events.len(),
                "hasMore": matching_events.len() > events.len(),
                "lostEventCount": lost_event_count,
                "events": events,
            }),
        );
        Ok(summary)
    }

    pub async fn close_event_session(&self, session_id: &str, reason: &str) -> Result<Value> {
        let handle = self.sessions.lock().await.remove(session_id);
        let Some(handle) = handle else {
            return Ok(json!({
                "sessionId": session_id,
                "existed": false,
                "closed": false,
            }));
        };

        handle.task.abort();
        {
            let mut session = handle.data.lock().await;
            session.closed = true;
            session.close_reason = reason.to_string();
        }
        let mut summary = summarize_event_session(&handle.data).await?;
        merge_json_object(
            &mut summary,
            json!({
                "existed": true,
                "closed": true,
            }),
        );
        Ok(summary)
    }
}

async fn open_qa_websocket() -> Result<QaSocket> {
    let url = get_qa_websocket_url()?;
    let operation = connect_async(url.as_str());
    let (socket, _) = timeout(Duration::from_millis(WS_CONNECT_TIMEOUT_MS), operation)
        .await
        .map_err(|_| {
            anyhow!("Timed out connecting to QA server WebSocket after {WS_CONNECT_TIMEOUT_MS} ms.")
        })?
        .context("Failed to connect QA server WebSocket")?;
    Ok(socket)
}

async fn send_ws_json(socket: &mut QaSocket, message: &Value) -> Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(message)?.into()))
        .await
        .context("Failed to send QA server WebSocket message")
}

async fn read_ws_json(socket: &mut QaSocket) -> Result<Value> {
    loop {
        match socket.next().await {
            Some(Ok(message)) => {
                if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                    continue;
                }
                if let Some(value) = parse_ws_message(message) {
                    return Ok(value);
                }
            }
            Some(Err(error)) => return Err(error).context("QA server WebSocket read failed"),
            None => bail!("QA server WebSocket closed before the expected message was received."),
        }
    }
}

fn parse_ws_message(message: Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str(text.as_ref()).ok(),
        Message::Binary(bytes) => serde_json::from_slice(bytes.as_ref()).ok(),
        _ => None,
    }
}

fn normalize_event_types(event_types: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for event_type in event_types {
        let event_type = event_type.trim();
        if !event_type.is_empty() && !normalized.iter().any(|existing| existing == event_type) {
            normalized.push(event_type.to_string());
        }
    }
    normalized
}

fn should_keep_event(message: &Value, event_types: &[String], include_snapshot: bool) -> bool {
    let Some(message_type) = message.get("type").and_then(Value::as_str) else {
        return false;
    };
    if !include_snapshot && message_type == "snapshot" {
        return false;
    }
    event_types.is_empty()
        || event_types
            .iter()
            .any(|event_type| event_type == message_type)
}

fn to_event_record(message: &Value, index: u64) -> Value {
    json!({
        "index": index,
        "receivedAt": Utc::now().to_rfc3339(),
        "type": message.get("type").and_then(Value::as_str).unwrap_or("unknown"),
        "message": message,
    })
}

async fn record_session_event(data: &Arc<Mutex<EventSessionData>>, message: Value) {
    let mut session = data.lock().await;
    if !should_keep_event(&message, &session.event_types, session.include_snapshot) {
        return;
    }
    let event = to_event_record(&message, session.next_index);
    session.next_index += 1;
    session.events.push_back(event);
    while session.events.len() > session.max_events {
        session.events.pop_front();
        session.dropped_events += 1;
    }
}

async fn mark_session_closed(data: &Arc<Mutex<EventSessionData>>, code: Option<u16>, reason: &str) {
    let mut session = data.lock().await;
    session.closed = true;
    session.close_code = code;
    session.close_reason = reason.to_string();
}

async fn summarize_event_session(data: &Arc<Mutex<EventSessionData>>) -> Result<Value> {
    let session = data.lock().await;
    Ok(summarize_event_session_locked(&session))
}

fn summarize_event_session_locked(session: &EventSessionData) -> Value {
    json!({
        "sessionId": session.session_id,
        "status": if session.closed { "closed" } else { "open" },
        "eventTypes": session.event_types,
        "includeSnapshot": session.include_snapshot,
        "maxEvents": session.max_events,
        "ttlMs": session.ttl_ms,
        "createdAt": session.created_at,
        "expiresAt": session.expires_at,
        "lastPolledAt": session.last_polled_at,
        "bufferedEventCount": session.events.len(),
        "nextIndex": session.next_index,
        "droppedEvents": session.dropped_events,
        "closeCode": session.close_code,
        "closeReason": session.close_reason,
        "lastError": session.last_error,
    })
}

async fn find_historical_result(filters: &Value) -> Result<Option<Value>> {
    let payload = request_json("/api/results").await?;
    Ok(payload
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|result| matches_result_filter(result, filters))
        .cloned())
}

fn ensure_result_wait_filter(filters: &Value) -> Result<()> {
    if [
        "requestId",
        "sequenceId",
        "clientId",
        "methodId",
        "methodName",
    ]
    .into_iter()
    .any(|key| filters.get(key).and_then(Value::as_str).is_some())
    {
        return Ok(());
    }
    bail!("At least one of requestId, sequenceId, clientId, methodId, or methodName is required.");
}

fn matches_result_filter(result: &Value, filters: &Value) -> bool {
    matches_optional_string(result, filters, "requestId")
        && matches_optional_string(result, filters, "sequenceId")
        && matches_optional_string(result, filters, "clientId")
        && matches_optional_string(result, filters, "methodId")
        && matches_optional_string(result, filters, "methodName")
        && matches_optional_string(result, filters, "status")
}

fn matches_sequence_filter(sequence: &Value, filters: &Value) -> bool {
    filters
        .get("sequenceId")
        .and_then(Value::as_str)
        .is_some_and(|sequence_id| {
            sequence.get("sequenceId").and_then(Value::as_str) == Some(sequence_id)
        })
        && matches_optional_string(sequence, filters, "clientId")
        && matches_optional_string(sequence, filters, "status")
}

fn matches_optional_string(value: &Value, filters: &Value, key: &str) -> bool {
    filters
        .get(key)
        .and_then(Value::as_str)
        .is_none_or(|expected| value.get(key).and_then(Value::as_str) == Some(expected))
}

fn normalize_arguments(method_arguments: &[Value]) -> Vec<String> {
    method_arguments
        .iter()
        .map(|argument| match argument {
            Value::String(value) => value.clone(),
            Value::Number(value) => value.to_string(),
            Value::Bool(value) => value.to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn normalize_sequence_step(step: Value) -> Result<Value> {
    let Value::Object(map) = step else {
        bail!("sequence step must be an object.");
    };
    let method_id = map
        .get("methodId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("sequence step requires methodId."))?
        .to_string();
    let method_name = map
        .get("methodName")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(&method_id)
        .to_string();
    let arguments = map
        .get("arguments")
        .and_then(Value::as_array)
        .map(|arguments| normalize_arguments(arguments))
        .unwrap_or_default();
    let step_id = map
        .get("stepId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    Ok(json!({
        "stepId": step_id,
        "methodId": method_id,
        "methodName": method_name,
        "arguments": arguments,
    }))
}

fn normalize_step_delay_ms(value: u64) -> u64 {
    value.min(MAX_SEQUENCE_STEP_DELAY_MS)
}

fn get_qa_websocket_url() -> Result<String> {
    let mut url = get_qa_server_url()?;
    match url.scheme() {
        "https" => url
            .set_scheme("wss")
            .map_err(|_| anyhow!("Failed to build QA WebSocket URL"))?,
        _ => url
            .set_scheme("ws")
            .map_err(|_| anyhow!("Failed to build QA WebSocket URL"))?,
    }
    url.set_path("/ws");
    url.query_pairs_mut()
        .clear()
        .append_pair("role", "web")
        .append_pair("controllerType", "mcp")
        .append_pair("controllerId", controller_id());
    Ok(url.to_string())
}

fn merge_json_object(target: &mut Value, source: Value) {
    let (Value::Object(target), Value::Object(source)) = (target, source) else {
        return;
    };
    target.extend(source);
}
