use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use axum::{extract::ws::Message, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Notify, RwLock, mpsc, oneshot};

use crate::archive::ExecutionArchive;
use crate::config::Config;

pub(crate) const HISTORY_LIMIT: usize = 200;
pub(crate) const MAX_SEQUENCE_STEP_DELAY_MS: u64 = 300_000;

pub(crate) type SharedState = Arc<AppState>;
pub(crate) type Sender = mpsc::Sender<Message>;

pub(crate) struct AppState {
    pub(crate) started_at: Instant,
    pub(crate) config: Config,
    pub(crate) archive: ExecutionArchive,
    pub(crate) inner: RwLock<ServerState>,
}

#[derive(Default)]
pub(crate) struct ServerState {
    pub(crate) unity_clients: HashMap<String, UnityClient>,
    pub(crate) web_clients: HashMap<String, WebClient>,
    pub(crate) controllers: HashMap<String, Controller>,
    pub(crate) execution_history: Vec<ExecutionRecord>,
    pub(crate) pending_executions: HashMap<String, PendingExecution>,
    pub(crate) active_sequences: HashMap<String, SequenceState>,
}

pub(crate) struct UnityClient {
    pub(crate) client_id: String,
    pub(crate) name: String,
    pub(crate) ip_address: String,
    pub(crate) ip_addresses: Vec<String>,
    pub(crate) remote_address: String,
    pub(crate) platform: String,
    pub(crate) unity_version: String,
    pub(crate) device_name: String,
    pub(crate) operating_system: String,
    pub(crate) methods: Vec<Value>,
    pub(crate) connected_at: DateTime<Utc>,
    pub(crate) last_seen_at: DateTime<Utc>,
    pub(crate) availability_changed_at: DateTime<Utc>,
    pub(crate) available: bool,
    pub(crate) unavailable_reason: String,
    pub(crate) client_busy: bool,
    pub(crate) current_request_id: String,
    pub(crate) current_method_name: String,
    pub(crate) sender: Sender,
    pub(crate) connection_id: String,
    pub(crate) is_alive: bool,
    pub(crate) lock: Option<ClientLock>,
}

pub(crate) struct WebClient {
    pub(crate) sender: Sender,
    pub(crate) controller_id: String,
    pub(crate) is_alive: bool,
}

pub(crate) struct Controller {
    pub(crate) id: String,
    pub(crate) controller_type: String,
    pub(crate) sockets: HashSet<String>,
    pub(crate) last_seen_at: DateTime<Utc>,
    pub(crate) last_disconnected_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub(crate) struct ClientLock {
    pub(crate) owner_id: String,
    pub(crate) owner_type: String,
    pub(crate) acquired_at: DateTime<Utc>,
    pub(crate) last_seen_at: DateTime<Utc>,
}

pub(crate) struct PendingExecution {
    pub(crate) started: ExecutionRecord,
    pub(crate) client_id: String,
    pub(crate) owner_id: String,
    pub(crate) owner_type: String,
    pub(crate) result_tx: Option<oneshot::Sender<ExecutionRecord>>,
}

pub(crate) struct SequenceState {
    pub(crate) client_id: String,
    pub(crate) owner_id: String,
    pub(crate) status: String,
    pub(crate) cancelled: bool,
    pub(crate) cancel_reason: String,
    pub(crate) current_request_id: String,
    pub(crate) notify: Arc<Notify>,
}

#[derive(Clone)]
pub(crate) struct ControllerIdentity {
    pub(crate) id: String,
    pub(crate) controller_type: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionRecord {
    pub(crate) request_id: String,
    pub(crate) client_id: String,
    pub(crate) method_id: String,
    pub(crate) method_name: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) status: String,
    pub(crate) allow_parallel_execution: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sequence_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) step_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) step_number: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_steps: Option<usize>,
}

#[derive(Clone)]
pub(crate) struct ExecutionMeta {
    pub(crate) sequence_id: Option<String>,
    pub(crate) step_id: Option<String>,
    pub(crate) step_index: Option<usize>,
    pub(crate) step_number: Option<usize>,
    pub(crate) total_steps: Option<usize>,
}

impl ExecutionMeta {
    pub(crate) fn empty() -> Self {
        Self {
            sequence_id: None,
            step_id: None,
            step_index: None,
            step_number: None,
            total_steps: None,
        }
    }
}

pub(crate) struct ExecutionPatch {
    pub(crate) status: String,
    pub(crate) success: bool,
    pub(crate) result: Value,
    pub(crate) error: String,
    pub(crate) duration_ms: u64,
    pub(crate) finished_at: String,
    pub(crate) client_id: Option<String>,
    pub(crate) method_id: Option<String>,
    pub(crate) method_name: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SequenceStep {
    pub(crate) step_id: String,
    pub(crate) step_index: usize,
    pub(crate) step_number: usize,
    pub(crate) total_steps: usize,
    pub(crate) method_id: String,
    pub(crate) method_name: String,
    pub(crate) arguments: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct NormalizedStep {
    pub(crate) step_id: String,
    pub(crate) method_id: String,
    pub(crate) method_name: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) allow_parallel_execution: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicSequence {
    pub(crate) sequence_id: String,
    pub(crate) client_id: String,
    pub(crate) status: String,
    pub(crate) stop_on_failure: bool,
    pub(crate) step_delay_ms: u64,
    pub(crate) total_steps: usize,
    pub(crate) completed_steps: usize,
    pub(crate) success_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) cancelled_count: usize,
    pub(crate) started_at: String,
    pub(crate) steps: Vec<SequenceStep>,
    pub(crate) results: Vec<ExecutionRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cancel_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<String>,
}

pub(crate) struct DispatchRequest {
    pub(crate) client_id: String,
    pub(crate) method_id: Option<String>,
    pub(crate) method_name: Option<String>,
    pub(crate) method_arguments: Value,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) controller: ControllerIdentity,
    pub(crate) wait_for_result: bool,
    pub(crate) allow_busy: bool,
    pub(crate) allow_parallel_execution: bool,
    pub(crate) meta: ExecutionMeta,
}

pub(crate) struct DispatchResult {
    pub(crate) request_id: String,
    pub(crate) started: ExecutionRecord,
    pub(crate) result_rx: Option<oneshot::Receiver<ExecutionRecord>>,
}

#[derive(Debug)]
pub(crate) struct RequestError {
    pub(crate) message: String,
    pub(crate) status_code: StatusCode,
}

impl RequestError {
    pub(crate) fn new(message: impl Into<String>, status_code: StatusCode) -> Self {
        Self {
            message: message.into(),
            status_code,
        }
    }
}
