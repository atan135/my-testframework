use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::{
    constants::{
        EVENT_SESSION_MAX_EVENTS, EVENT_SESSION_TTL_MS, EXECUTE_TIMEOUT_MS,
        MAX_SEQUENCE_STEP_DELAY_MS, QAMCP_VERSION, SEQUENCE_TIMEOUT_MS, WAIT_RESULT_TIMEOUT_MS,
        WATCH_EVENTS_DURATION_MS, WS_CONNECT_TIMEOUT_MS,
    },
    qa::{
        api::{
            find_methods_data, get_health_data, get_method_data, get_results_data,
            list_unity_clients_data,
        },
        artifacts::capture_screenshot_data,
        ws::{
            EventSessionStore, execute_method_via_websocket, execute_sequence_via_websocket,
            stop_via_websocket, wait_for_result_via_websocket, watch_events_via_websocket,
        },
    },
};

const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

#[derive(Clone, Default)]
struct McpState {
    event_sessions: EventSessionStore,
}

pub async fn run_mcp_stdio() -> Result<()> {
    let state = McpState::default();
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut tasks = JoinSet::new();

    while let Some(line) = lines.next_line().await? {
        while let Some(result) = tasks.try_join_next() {
            result??;
        }

        let state = state.clone();
        let stdout = stdout.clone();
        tasks.spawn(async move {
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(message) => handle_message(&state, message).await,
                Err(error) => Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("Parse error: {error}"),
                )),
            };

            write_response(stdout, response).await
        });
    }

    while let Some(result) = tasks.join_next().await {
        result??;
    }

    Ok(())
}

async fn write_response(
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    response: Option<Value>,
) -> Result<()> {
    let Some(response) = response else {
        return Ok(());
    };

    let payload = format!("{}\n", serde_json::to_string(&response)?);
    let mut stdout = stdout.lock().await;
    stdout.write_all(payload.as_bytes()).await?;
    stdout.flush().await?;
    Ok(())
}

async fn handle_message(state: &McpState, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");

    if id.is_none() {
        return None;
    }
    let id = id.unwrap_or(Value::Null);

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": requested_protocol_version(&message),
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "qamcp",
                "version": QAMCP_VERSION
            }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(state, &message).await,
        _ => Err(anyhow!("Method not found: {method}")),
    };

    Some(match result {
        Ok(result) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
        Err(error) => error_response(id, -32000, &error.to_string()),
    })
}

fn requested_protocol_version(message: &Value) -> String {
    let requested = message
        .get("params")
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or(MCP_PROTOCOL_VERSION);
    if [
        "2025-11-25",
        "2025-06-18",
        "2025-03-26",
        "2024-11-05",
        "2024-10-07",
    ]
    .contains(&requested)
    {
        requested.to_string()
    } else {
        MCP_PROTOCOL_VERSION.to_string()
    }
}

async fn call_tool(state: &McpState, message: &Value) -> Result<Value> {
    let params = message
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("tools/call requires params."))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tools/call requires params.name."))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let data = match name {
        "qa_health" => get_health_data().await?,
        "qa_list_unity_clients" => {
            list_unity_clients_data(bool_arg(&arguments, "includeMethods", true)).await?
        }
        "qa_find_methods" => {
            find_methods_data(
                string_arg(&arguments, "clientId").as_deref(),
                &string_arg(&arguments, "query").unwrap_or_default(),
                usize_arg(&arguments, "limit", 50),
            )
            .await?
        }
        "qa_get_method" => {
            get_method_data(
                string_arg(&arguments, "clientId").as_deref(),
                &required_string_arg(&arguments, "query")?,
            )
            .await?
        }
        "qa_get_results" => {
            get_results_data(
                string_arg(&arguments, "clientId").as_deref(),
                string_arg(&arguments, "status").as_deref(),
                string_arg(&arguments, "sequenceId").as_deref(),
                usize_arg(&arguments, "limit", 20),
            )
            .await?
        }
        "qa_execute_method" => {
            execute_method_via_websocket(
                required_string_arg(&arguments, "clientId")?,
                required_string_arg(&arguments, "methodId")?,
                string_arg(&arguments, "methodName"),
                array_arg(&arguments, "arguments"),
                u64_arg(&arguments, "timeoutMs", EXECUTE_TIMEOUT_MS),
            )
            .await?
        }
        "qa_capture_screenshot" => {
            capture_screenshot_data(
                required_string_arg(&arguments, "clientId")?,
                required_string_arg(&arguments, "savePath")?,
            )
            .await?
        }
        "qa_execute_sequence" => {
            execute_sequence_via_websocket(
                required_string_arg(&arguments, "clientId")?,
                required_array_arg(&arguments, "steps")?,
                bool_arg(&arguments, "stopOnFailure", true),
                u64_arg(&arguments, "stepDelayMs", 0),
                u64_arg(&arguments, "timeoutMs", SEQUENCE_TIMEOUT_MS),
            )
            .await?
        }
        "qa_stop_execution" => {
            stop_via_websocket(
                Some(required_string_arg(&arguments, "requestId")?),
                None,
                string_arg(&arguments, "reason").unwrap_or_else(|| "Stopped by qamcp.".to_string()),
                u64_arg(&arguments, "timeoutMs", WS_CONNECT_TIMEOUT_MS),
            )
            .await?
        }
        "qa_stop_sequence" => {
            stop_via_websocket(
                None,
                Some(required_string_arg(&arguments, "sequenceId")?),
                string_arg(&arguments, "reason").unwrap_or_else(|| "Stopped by qamcp.".to_string()),
                u64_arg(&arguments, "timeoutMs", WS_CONNECT_TIMEOUT_MS),
            )
            .await?
        }
        "qa_watch_events" => {
            watch_events_via_websocket(
                u64_arg(&arguments, "durationMs", WATCH_EVENTS_DURATION_MS),
                string_array_arg(&arguments, "eventTypes"),
                bool_arg(&arguments, "includeSnapshot", true),
                usize_arg(&arguments, "maxEvents", 100),
            )
            .await?
        }
        "qa_wait_for_result" => {
            let mut filters = object_arg(arguments);
            if !filters.contains_key("includeHistory") {
                filters.insert("includeHistory".to_string(), Value::Bool(true));
            }
            if !filters.contains_key("timeoutMs") {
                filters.insert("timeoutMs".to_string(), json!(WAIT_RESULT_TIMEOUT_MS));
            }
            wait_for_result_via_websocket(Value::Object(filters)).await?
        }
        "qa_open_event_session" => {
            state
                .event_sessions
                .open_event_session(
                    string_array_arg(&arguments, "eventTypes"),
                    bool_arg(&arguments, "includeSnapshot", true),
                    usize_arg(&arguments, "maxEvents", EVENT_SESSION_MAX_EVENTS),
                    u64_arg(&arguments, "ttlMs", EVENT_SESSION_TTL_MS),
                )
                .await?
        }
        "qa_poll_event_session" => {
            state
                .event_sessions
                .poll_event_session(
                    &required_string_arg(&arguments, "sessionId")?,
                    u64_arg(&arguments, "cursor", 0),
                    usize_arg(&arguments, "maxEvents", 100),
                )
                .await?
        }
        "qa_close_event_session" => {
            state
                .event_sessions
                .close_event_session(
                    &required_string_arg(&arguments, "sessionId")?,
                    "closed by qa_close_event_session",
                )
                .await?
        }
        _ => bail!("Unknown tool: {name}"),
    };

    to_tool_result(data)
}

fn to_tool_result(data: Value) -> Result<Value> {
    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&data)?,
            }
        ],
        "structuredContent": data,
    }))
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    })
}

fn required_string_arg(arguments: &Value, key: &str) -> Result<String> {
    string_arg(arguments, key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{key} is required."))
}

fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn bool_arg(arguments: &Value, key: &str, default_value: bool) -> bool {
    arguments
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(default_value)
}

fn u64_arg(arguments: &Value, key: &str, default_value: u64) -> u64 {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default_value)
}

fn usize_arg(arguments: &Value, key: &str, default_value: usize) -> usize {
    u64_arg(arguments, key, default_value as u64) as usize
}

fn array_arg(arguments: &Value, key: &str) -> Vec<Value> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn required_array_arg(arguments: &Value, key: &str) -> Result<Vec<Value>> {
    let values = array_arg(arguments, key);
    if values.is_empty() {
        bail!("{key} is required.");
    }
    Ok(values)
}

fn string_array_arg(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn object_arg(arguments: Value) -> Map<String, Value> {
    match arguments {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "qa_health",
            "检查 QA 服务状态",
            "检查 QA 注册服务是否可连接。",
            object_schema(vec![], vec![]),
            true,
            false,
            true,
        ),
        tool(
            "qa_list_unity_clients",
            "列出 Unity QA 客户端",
            "列出当前已连接到 QA 注册服务的 Unity 客户端。",
            object_schema(
                vec![(
                    "includeMethods",
                    bool_schema("是否返回每个客户端完整注册的 QaTest 方法列表，默认返回。"),
                )],
                vec![],
            ),
            true,
            false,
            true,
        ),
        tool(
            "qa_find_methods",
            "搜索 QaTest 方法",
            "按客户端、方法名、方法 ID、声明类型或描述搜索已注册的 QaTest 方法。",
            object_schema(
                vec![
                    ("clientId", string_schema("限定搜索某个 Unity 客户端。")),
                    ("query", string_schema("搜索文本。")),
                    ("limit", integer_schema("最多返回数量。", 1, 200)),
                ],
                vec![],
            ),
            true,
            false,
            true,
        ),
        tool(
            "qa_get_method",
            "获取 QaTest 方法详情",
            "按精确 methodId、短 methodId 或方法名获取已注册 QaTest 方法详情；query 用 | 分隔时可一次查询多个。",
            object_schema(
                vec![
                    ("clientId", string_schema("限定搜索某个 Unity 客户端。")),
                    (
                        "query",
                        string_schema("精确 methodId、短 methodId 或方法名；多个查询用 | 分隔。"),
                    ),
                ],
                vec!["query"],
            ),
            true,
            false,
            true,
        ),
        tool(
            "qa_get_results",
            "获取 QA 执行结果",
            "读取 QA 注册服务中的最近执行记录，并支持本地过滤。",
            object_schema(
                vec![
                    ("clientId", string_schema("按 Unity 客户端过滤。")),
                    (
                        "status",
                        enum_schema(vec!["running", "success", "failed", "cancelled"]),
                    ),
                    ("sequenceId", string_schema("按请求序列 ID 过滤。")),
                    ("limit", integer_schema("最多返回数量。", 1, 200)),
                ],
                vec![],
            ),
            true,
            false,
            true,
        ),
        tool(
            "qa_execute_method",
            "执行单个 QaTest 方法",
            "通过 QA 服务 WebSocket 执行一个已注册的 QaTest 方法，并等待执行结果。",
            object_schema(
                vec![
                    ("clientId", string_schema("Unity 客户端 ID。")),
                    ("methodId", string_schema("QaTest 方法 ID。")),
                    ("methodName", string_schema("显示用方法名。")),
                    ("arguments", qa_arguments_schema()),
                    (
                        "timeoutMs",
                        integer_schema("等待超时，单位毫秒。", 1000, 300000),
                    ),
                ],
                vec!["clientId", "methodId"],
            ),
            false,
            true,
            false,
        ),
        tool(
            "qa_capture_screenshot",
            "执行截图 QaTest 并保存",
            "搜索或指定 Unity 截图上传 QaTest 方法，执行后从返回 artifact 下载截图到本地路径。",
            object_schema(
                vec![
                    ("clientId", string_schema("Unity 客户端 ID。")),
                    (
                        "savePath",
                        string_schema("本地保存路径；如果是目录则使用 artifact fileName。"),
                    ),
                ],
                vec!["clientId", "savePath"],
            ),
            false,
            true,
            false,
        ),
        tool(
            "qa_execute_sequence",
            "执行 QaTest 请求序列",
            "通过 QA 服务 WebSocket 按顺序执行多个 QaTest 方法，并等待序列汇总结果。",
            object_schema(
                vec![
                    ("clientId", string_schema("Unity 客户端 ID。")),
                    ("steps", sequence_steps_schema()),
                    ("stopOnFailure", bool_schema("步骤失败后是否停止。")),
                    (
                        "stepDelayMs",
                        integer_schema("相邻步骤之间等待的毫秒数。", 0, MAX_SEQUENCE_STEP_DELAY_MS),
                    ),
                    (
                        "timeoutMs",
                        integer_schema("等待超时，单位毫秒。", 1000, 600000),
                    ),
                ],
                vec!["clientId", "steps"],
            ),
            false,
            true,
            false,
        ),
        tool(
            "qa_stop_execution",
            "停止 QaTest 执行",
            "请求 QA 服务停止一个正在运行的单次执行。",
            object_schema(
                vec![
                    ("requestId", string_schema("请求 ID。")),
                    ("reason", string_schema("停止原因。")),
                    (
                        "timeoutMs",
                        integer_schema("等待超时，单位毫秒。", 1000, 30000),
                    ),
                ],
                vec!["requestId"],
            ),
            false,
            true,
            true,
        ),
        tool(
            "qa_stop_sequence",
            "停止 QaTest 请求序列",
            "请求 QA 服务停止一个正在运行的请求序列。",
            object_schema(
                vec![
                    ("sequenceId", string_schema("请求序列 ID。")),
                    ("reason", string_schema("停止原因。")),
                    (
                        "timeoutMs",
                        integer_schema("等待超时，单位毫秒。", 1000, 30000),
                    ),
                ],
                vec!["sequenceId"],
            ),
            false,
            true,
            true,
        ),
        tool(
            "qa_watch_events",
            "监听 QA WebSocket 事件",
            "临时连接 QA 服务 WebSocket，监听一段时间内的事件并一次性返回。",
            object_schema(
                vec![
                    (
                        "durationMs",
                        integer_schema("监听持续时间，单位毫秒。", 100, 300000),
                    ),
                    ("eventTypes", string_array_schema("只返回指定事件类型。")),
                    (
                        "includeSnapshot",
                        bool_schema("是否包含连接后收到的 snapshot 事件。"),
                    ),
                    ("maxEvents", integer_schema("最多返回事件数量。", 1, 1000)),
                ],
                vec![],
            ),
            true,
            false,
            false,
        ),
        tool(
            "qa_wait_for_result",
            "等待 QA 执行结果",
            "按 requestId、sequenceId、clientId 或 methodId 等条件等待 QA 执行结果返回。",
            object_schema(
                vec![
                    ("requestId", string_schema("请求 ID。")),
                    ("sequenceId", string_schema("请求序列 ID。")),
                    ("clientId", string_schema("Unity 客户端 ID。")),
                    ("methodId", string_schema("QaTest 方法 ID。")),
                    ("methodName", string_schema("QaTest 方法名。")),
                    (
                        "status",
                        enum_schema(vec!["running", "success", "failed", "cancelled"]),
                    ),
                    (
                        "includeHistory",
                        bool_schema("如果提供 requestId，是否先查询已有执行历史。"),
                    ),
                    (
                        "timeoutMs",
                        integer_schema("等待超时，单位毫秒。", 1000, 600000),
                    ),
                ],
                vec![],
            ),
            true,
            false,
            false,
        ),
        tool(
            "qa_open_event_session",
            "打开 QA 事件会话",
            "创建一条可跨多次 MCP 调用轮询的 QA WebSocket 长连接事件会话。",
            object_schema(
                vec![
                    ("eventTypes", string_array_schema("只缓存指定事件类型。")),
                    (
                        "includeSnapshot",
                        bool_schema("是否缓存连接后收到的 snapshot 事件。"),
                    ),
                    (
                        "maxEvents",
                        integer_schema("会话内最多缓存事件数量。", 10, 5000),
                    ),
                    (
                        "ttlMs",
                        integer_schema("会话自动过期时间，单位毫秒。", 10000, 3600000),
                    ),
                ],
                vec![],
            ),
            true,
            false,
            false,
        ),
        tool(
            "qa_poll_event_session",
            "轮询 QA 事件会话",
            "从已打开的 QA 事件会话中读取缓存事件。",
            object_schema(
                vec![
                    ("sessionId", string_schema("事件会话 ID。")),
                    (
                        "cursor",
                        integer_schema("上一次返回的 nextCursor。", 0, i64::MAX as u64),
                    ),
                    (
                        "maxEvents",
                        integer_schema("本次最多返回事件数量。", 1, 1000),
                    ),
                ],
                vec!["sessionId"],
            ),
            true,
            false,
            true,
        ),
        tool(
            "qa_close_event_session",
            "关闭 QA 事件会话",
            "关闭指定 QA WebSocket 事件会话并释放连接资源。",
            object_schema(
                vec![("sessionId", string_schema("事件会话 ID。"))],
                vec!["sessionId"],
            ),
            false,
            false,
            true,
        ),
    ]
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "title": title,
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "idempotentHint": idempotent,
            "openWorldHint": false,
        }
    })
}

fn object_schema(properties: Vec<(&str, Value)>, required: Vec<&str>) -> Value {
    let properties = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<Map<_, _>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn string_schema(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn bool_schema(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

fn integer_schema(description: &str, minimum: u64, maximum: u64) -> Value {
    json!({
        "type": "integer",
        "description": description,
        "minimum": minimum,
        "maximum": maximum,
    })
}

fn enum_schema(values: Vec<&str>) -> Value {
    json!({ "type": "string", "enum": values })
}

fn string_array_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": { "type": "string" },
    })
}

fn qa_arguments_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "anyOf": [
                { "type": "string" },
                { "type": "number" },
                { "type": "boolean" }
            ]
        }
    })
}

fn sequence_steps_schema() -> Value {
    json!({
        "type": "array",
        "minItems": 1,
        "items": {
            "type": "object",
            "properties": {
                "stepId": { "type": "string" },
                "methodId": { "type": "string" },
                "methodName": { "type": "string" },
                "arguments": qa_arguments_schema(),
            },
            "required": ["methodId"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::tool_definitions;

    #[test]
    fn exposes_expected_tool_count() {
        let tools = tool_definitions();
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|name| name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 15);
        assert!(names.contains(&"qa_execute_method"));
        assert!(names.contains(&"qa_capture_screenshot"));
        assert!(names.contains(&"qa_open_event_session"));
        assert!(names.contains(&"qa_close_event_session"));
    }
}
