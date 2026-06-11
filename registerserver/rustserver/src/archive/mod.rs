use std::time::Duration;

use chrono::{DateTime, NaiveDateTime};
use serde_json::{Value, json};
use sqlx::{MySqlPool, mysql::MySqlPoolOptions};
use tokio::{sync::mpsc, time};

use crate::{
    config::Config,
    logging::LogEvent,
    state::{ControllerIdentity, ExecutionRecord, ServerState, UnityClient},
};

#[derive(Clone)]
pub(crate) struct ExecutionArchive {
    sender: Option<mpsc::Sender<ArchiveRecord>>,
}

impl ExecutionArchive {
    pub(crate) async fn initialize(config: &Config) -> Self {
        if !config.archive_mysql_enabled {
            return Self { sender: None };
        }

        let Some(url) = config.archive_mysql_url.as_deref() else {
            LogEvent::error("execution_archive_disabled")
                .field("reason", "QA_EXECUTION_ARCHIVE_MYSQL_URL is empty")
                .emit();
            return Self { sender: None };
        };

        let connect = MySqlPoolOptions::new()
            .max_connections(config.archive_mysql_max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url);
        let pool = match time::timeout(Duration::from_secs(5), connect).await {
            Ok(Ok(pool)) => pool,
            Ok(Err(error)) => {
                LogEvent::error("execution_archive_disabled")
                    .field("reason", "mysql_connect_failed")
                    .field("error", error.to_string())
                    .emit();
                return Self { sender: None };
            }
            Err(_) => {
                LogEvent::error("execution_archive_disabled")
                    .field("reason", "mysql_connect_timeout")
                    .field("timeoutMs", 5000)
                    .emit();
                return Self { sender: None };
            }
        };

        let (sender, receiver) = mpsc::channel(config.archive_queue_size);
        tokio::spawn(archive_writer_loop(pool, receiver));
        LogEvent::new("execution_archive_enabled")
            .field("backend", "mysql")
            .field("maxConnections", config.archive_mysql_max_connections)
            .field("queueSize", config.archive_queue_size)
            .emit();

        Self {
            sender: Some(sender),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub(crate) fn enqueue_locked(
        &self,
        inner: &ServerState,
        execution: &ExecutionRecord,
        controller: Option<&ControllerIdentity>,
    ) {
        let Some(sender) = &self.sender else {
            return;
        };

        let record = ArchiveRecord::from_execution(inner, execution, controller);
        if let Err(error) = sender.try_send(record) {
            LogEvent::warn("execution_archive_enqueue_failed")
                .request_id_str(&execution.request_id)
                .client_id_str(&execution.client_id)
                .field("error", error.to_string())
                .emit();
        }
    }
}

async fn archive_writer_loop(pool: MySqlPool, mut receiver: mpsc::Receiver<ArchiveRecord>) {
    while let Some(record) = receiver.recv().await {
        if let Err(error) = upsert_archive_record(&pool, &record).await {
            LogEvent::error("execution_archive_write_failed")
                .request_id_str(&record.request_id)
                .client_id_str(&record.client_id)
                .field("status", record.status.clone())
                .field("error", error.to_string())
                .emit();
        }
    }
}

#[derive(Clone)]
struct ArchiveRecord {
    request_id: String,
    sequence_id: Option<String>,
    step_id: Option<String>,
    step_index: Option<i64>,
    step_number: Option<i64>,
    total_steps: Option<i64>,
    client_id: String,
    client_name: Option<String>,
    client_ip: Option<String>,
    client_platform: Option<String>,
    unity_version: Option<String>,
    controller_id: Option<String>,
    controller_type: Option<String>,
    method_id: String,
    method_name: Option<String>,
    method_real_name: Option<String>,
    method_display_name: Option<String>,
    declaring_type: Option<String>,
    return_type: Option<String>,
    allow_parallel_execution: bool,
    status: String,
    success: Option<bool>,
    duration_ms: Option<i64>,
    error_text: Option<String>,
    arguments_json: String,
    result_json: Option<String>,
    method_metadata_json: Option<String>,
    client_snapshot_json: Option<String>,
    execution_record_json: String,
    started_at: Option<NaiveDateTime>,
    finished_at: Option<NaiveDateTime>,
}

impl ArchiveRecord {
    fn from_execution(
        inner: &ServerState,
        execution: &ExecutionRecord,
        controller: Option<&ControllerIdentity>,
    ) -> Self {
        let client = inner.unity_clients.get(&execution.client_id);
        let method_metadata = client.and_then(|client| find_method_metadata(client, execution));
        let client_snapshot = client.map(client_snapshot_json);
        let method_id = non_empty_string(&execution.method_id)
            .or_else(|| non_empty_string(&execution.method_name))
            .unwrap_or_else(|| execution.request_id.clone());
        let method_name = non_empty_string(&execution.method_name);
        let method_real_name = method_metadata
            .and_then(method_real_name)
            .or_else(|| method_real_name_from_text(&execution.method_id))
            .or_else(|| method_real_name_from_text(&execution.method_name));
        let method_display_name = method_metadata
            .and_then(|method| json_string_field(method, &["name", "displayName"]))
            .or_else(|| method_name.clone());
        let declaring_type =
            method_metadata.and_then(|method| json_string_field(method, &["declaringType"]));
        let return_type =
            method_metadata.and_then(|method| json_string_field(method, &["returnType"]));

        Self {
            request_id: execution.request_id.clone(),
            sequence_id: execution.sequence_id.clone(),
            step_id: execution.step_id.clone(),
            step_index: usize_to_i64(execution.step_index),
            step_number: usize_to_i64(execution.step_number),
            total_steps: usize_to_i64(execution.total_steps),
            client_id: execution.client_id.clone(),
            client_name: client.and_then(|client| non_empty_string(&client.name)),
            client_ip: client.and_then(|client| non_empty_string(&client.ip_address)),
            client_platform: client.and_then(|client| non_empty_string(&client.platform)),
            unity_version: client.and_then(|client| non_empty_string(&client.unity_version)),
            controller_id: controller.and_then(|controller| non_empty_string(&controller.id)),
            controller_type: controller
                .and_then(|controller| non_empty_string(&controller.controller_type)),
            method_id,
            method_name,
            method_real_name,
            method_display_name,
            declaring_type,
            return_type,
            allow_parallel_execution: execution.allow_parallel_execution,
            status: execution.status.clone(),
            success: execution.success,
            duration_ms: execution.duration_ms.and_then(u64_to_i64),
            error_text: execution.error.clone().filter(|value| !value.is_empty()),
            arguments_json: Value::Array(
                execution
                    .arguments
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            )
            .to_string(),
            result_json: execution.result.as_ref().map(Value::to_string),
            method_metadata_json: method_metadata.map(Value::to_string),
            client_snapshot_json: client_snapshot.map(|value| value.to_string()),
            execution_record_json: serde_json::to_string(execution)
                .unwrap_or_else(|_| json!({}).to_string()),
            started_at: parse_datetime(execution.started_at.as_deref()),
            finished_at: parse_datetime(execution.finished_at.as_deref()),
        }
    }
}

async fn upsert_archive_record(
    pool: &MySqlPool,
    record: &ArchiveRecord,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO execution_records (
          request_id,
          sequence_id,
          step_id,
          step_index,
          step_number,
          total_steps,
          client_id,
          client_name,
          client_ip,
          client_platform,
          unity_version,
          controller_id,
          controller_type,
          method_id,
          method_name,
          method_real_name,
          method_display_name,
          declaring_type,
          return_type,
          allow_parallel_execution,
          status,
          success,
          duration_ms,
          error_text,
          arguments_json,
          result_json,
          method_metadata_json,
          client_snapshot_json,
          execution_record_json,
          started_at,
          finished_at
        ) VALUES (
          ?, ?, ?, ?, ?, ?,
          ?, ?, ?, ?, ?,
          ?, ?,
          ?, ?, ?, ?, ?, ?, ?,
          ?, ?, ?, ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?, ?
        )
        ON DUPLICATE KEY UPDATE
          sequence_id = VALUES(sequence_id),
          step_id = VALUES(step_id),
          step_index = VALUES(step_index),
          step_number = VALUES(step_number),
          total_steps = VALUES(total_steps),
          client_id = VALUES(client_id),
          client_name = VALUES(client_name),
          client_ip = VALUES(client_ip),
          client_platform = VALUES(client_platform),
          unity_version = VALUES(unity_version),
          controller_id = VALUES(controller_id),
          controller_type = VALUES(controller_type),
          method_id = VALUES(method_id),
          method_name = VALUES(method_name),
          method_real_name = VALUES(method_real_name),
          method_display_name = VALUES(method_display_name),
          declaring_type = VALUES(declaring_type),
          return_type = VALUES(return_type),
          allow_parallel_execution = VALUES(allow_parallel_execution),
          status = VALUES(status),
          success = VALUES(success),
          duration_ms = VALUES(duration_ms),
          error_text = VALUES(error_text),
          arguments_json = VALUES(arguments_json),
          result_json = VALUES(result_json),
          method_metadata_json = VALUES(method_metadata_json),
          client_snapshot_json = VALUES(client_snapshot_json),
          execution_record_json = VALUES(execution_record_json),
          started_at = COALESCE(execution_records.started_at, VALUES(started_at)),
          finished_at = VALUES(finished_at),
          updated_at = CURRENT_TIMESTAMP(3)
        "#,
    )
    .bind(&record.request_id)
    .bind(&record.sequence_id)
    .bind(&record.step_id)
    .bind(record.step_index)
    .bind(record.step_number)
    .bind(record.total_steps)
    .bind(&record.client_id)
    .bind(&record.client_name)
    .bind(&record.client_ip)
    .bind(&record.client_platform)
    .bind(&record.unity_version)
    .bind(&record.controller_id)
    .bind(&record.controller_type)
    .bind(&record.method_id)
    .bind(&record.method_name)
    .bind(&record.method_real_name)
    .bind(&record.method_display_name)
    .bind(&record.declaring_type)
    .bind(&record.return_type)
    .bind(record.allow_parallel_execution)
    .bind(&record.status)
    .bind(record.success)
    .bind(record.duration_ms)
    .bind(&record.error_text)
    .bind(&record.arguments_json)
    .bind(&record.result_json)
    .bind(&record.method_metadata_json)
    .bind(&record.client_snapshot_json)
    .bind(&record.execution_record_json)
    .bind(record.started_at)
    .bind(record.finished_at)
    .execute(pool)
    .await?;

    Ok(())
}

fn find_method_metadata<'a>(
    client: &'a UnityClient,
    execution: &ExecutionRecord,
) -> Option<&'a Value> {
    client.methods.iter().find(|method| {
        let candidates = [
            method.get("id").and_then(Value::as_str),
            method.get("name").and_then(Value::as_str),
            method.get("methodId").and_then(Value::as_str),
            method.get("methodName").and_then(Value::as_str),
        ];

        candidates.into_iter().flatten().any(|candidate| {
            (!execution.method_id.is_empty() && candidate == execution.method_id)
                || (!execution.method_name.is_empty() && candidate == execution.method_name)
        })
    })
}

fn client_snapshot_json(client: &UnityClient) -> Value {
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
        "connectedAt": client.connected_at.to_rfc3339(),
        "lastSeenAt": client.last_seen_at.to_rfc3339(),
        "available": client.available,
        "unavailableReason": client.unavailable_reason,
    })
}

fn method_real_name(method: &Value) -> Option<String> {
    json_string_field(method, &["id", "methodId"])
        .and_then(|value| method_real_name_from_text(&value))
        .or_else(|| json_string_field(method, &["methodName"]))
}

fn method_real_name_from_text(value: &str) -> Option<String> {
    let before_parameters = value.split_once('(').map_or(value, |(name, _)| name);
    let after_type = before_parameters
        .rsplit_once('.')
        .map_or(before_parameters, |(_, name)| name)
        .trim();
    non_empty_string(after_type)
}

fn json_string_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .and_then(non_empty_string)
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn usize_to_i64(value: Option<usize>) -> Option<i64> {
    value.and_then(|value| i64::try_from(value).ok())
}

fn u64_to_i64(value: u64) -> Option<i64> {
    i64::try_from(value).ok()
}

fn parse_datetime(value: Option<&str>) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|value| value.naive_utc())
}
