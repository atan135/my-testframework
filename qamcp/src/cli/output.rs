use anyhow::Result;
use serde_json::Value;

use crate::cli::parser::CliOptions;

pub fn print_cli_output(command: &str, data: &Value, options: &CliOptions) -> Result<()> {
    if options.text != Some(true) {
        println!("{}", serde_json::to_string_pretty(data)?);
        return Ok(());
    }

    match command {
        "health" => print_health(data),
        "clients" => print_clients(data),
        "config" => print_config(data),
        "search" => print_tools(data, options),
        "describe" => print_tool(data),
        "results" => print_results(data),
        "run" => print_execution(data),
        "capture-screenshot" => print_artifact_save(data),
        "sequence" => print_sequence(data),
        "stop" => print_stop(data),
        "wait" => print_wait(data),
        "watch" => print_watch(data),
        _ => println!("{}", serde_json::to_string_pretty(data)?),
    }
    Ok(())
}

pub fn print_help(command: Option<&str>) {
    let lines = match command {
        Some("health") => vec![
            "health 选项:",
            "  --text, --human        输出人类可读健康检查摘要。",
        ],
        Some("clients") => vec![
            "clients 选项:",
            "  --text, --human        输出人类可读客户端摘要。",
            "",
            "方法发现请使用 qamcp search --client \"<clientId>\" --limit 10000。",
        ],
        Some("config") => vec![
            "config 用法:",
            "  qamcp config                         查看当前配置。",
            "  qamcp config path                    输出配置文件路径。",
            "  qamcp config get serverUrl           输出当前 QA register server 地址。",
            "  qamcp config set serverUrl <url>     保存 QA register server 地址。",
            "  qamcp config --server-url <url>      同上。",
            "",
            "默认 serverUrl: http://localhost:3000",
            "配置会保存到 qamcp 可执行文件同目录的 qamcp.config.json。",
        ],
        Some("mcp") => vec![
            "mcp 用法:",
            "  qamcp mcp",
            "  qamcp mcp --help",
            "",
            "说明:",
            "  启动 stdio MCP server，供支持 MCP 的 AI 客户端调用 MCP tools。",
            "  MCP 模式通过标准输入输出通信，不会在终端输出普通 CLI 结果。",
            "",
            "准备工作:",
            "  1. 启动 QA register server。",
            "  2. 确认 Unity 工程进入 Play Mode 并连接到 register server。",
            "  3. 如非默认地址，先运行 qamcp config set serverUrl <url>。",
            "",
            "MCP 客户端配置示例:",
            "  command: qamcp",
            "  args: [\"mcp\"]",
            "",
            "帮助:",
            "  qamcp help mcp",
            "  qamcp mcp --help",
            "  qamcp mcp -h",
        ],
        Some("search") => vec![
            "search 选项:",
            "  --client, -c <id>       限定搜索某个 Unity 客户端。",
            "  --limit, -l <number>    最多输出的方法数量。默认值: 50。",
            "  --text, --human        输出人类可读表格。",
            "",
            "用法:",
            "  qamcp search --client \"<clientId>\" --limit 10000",
            "",
            "说明:",
            "  QaTest 方法由当前 Unity 项目动态注册；使用 search 获取当前项目实际 methodId。",
        ],
        Some("describe") => vec![
            "describe 选项:",
            "  --query, -q <text>      必填，精确 methodId、短 methodId 或方法名；用 | 分隔可一次查询多个。",
            "  --client, -c <id>       限定查询某个 Unity 客户端。",
            "  --text, --human        输出人类可读方法详情。",
            "",
            "用法:",
            "  qamcp describe --client \"<clientId>\" --query \"<methodName-or-methodId>\"",
        ],
        Some("results") => vec![
            "results 选项:",
            "  --client, -c <id>       按 Unity 客户端过滤。",
            "  --status, -s <status>   按 running、success、failed 或 cancelled 过滤。",
            "  --limit, -l <number>    最多输出的结果数量。默认值: 20。",
            "  --text, --human        输出人类可读结果记录。",
            "",
            "用法:",
            "  qamcp results --client \"<clientId>\" --limit 20",
            "  qamcp results --client \"<clientId>\" --status failed --limit 20",
        ],
        Some("run") => vec![
            "run 选项:",
            "  --client, -c <id>       必填，Unity 客户端 ID。",
            "  --tool, -t <methodId>   必填，QaTest 方法 ID。先用 search/describe 获取当前项目实际值。",
            "  --name, -n <name>       可选，显示用方法名。",
            "  --arg, -a <value>       按方法参数顺序重复传参，例如 -a val1 -a val2。",
            "  --arg-file <path>       读取本机文件内容作为一个参数，按出现顺序参与传参。",
            "  --args <json-array>     兼容旧脚本的 JSON 数组参数；Windows shell 下不推荐。",
            "  --timeout <ms>          等待超时。默认值: 70000。",
            "  --text, --human        输出人类可读执行结果。",
            "",
            "推荐流程:",
            "  1. qamcp clients",
            "  2. qamcp search --client \"<clientId>\" --limit 10000",
            "  3. qamcp describe --client \"<clientId>\" --query \"<methodName-or-methodId>\"",
            "  4. qamcp run --client \"<clientId>\" --tool \"<methodId>\" --name \"<methodName>\" -a \"arg1\" -a \"arg2\" --timeout 70000",
            "  5. 大文本参数可用 --arg-file，例如 -a \"\" --arg-file \".\\record.txt\" -a \"1\"。",
            "",
            "说明:",
            "  QaTest 方法由当前 Unity 项目动态注册；help 不固定展示具体 methodId。",
            "  run 参数优先使用重复 -a/--arg，避免 PowerShell/cmd 对 JSON 引号和转义符的改写。",
            "  --arg-file 由 qamcp 所在机器读取文件，Unity 客户端只接收文件内容字符串。",
            "  方法注册元数据 allowParallelExecution=true 时，server 会自动允许只读查询类方法并行执行；qamcp run 不需要额外并行参数。",
        ],
        Some("capture-screenshot") => vec![
            "capture-screenshot 用法:",
            "  qamcp capture-screenshot --client \"<clientId>\" --output \".\\screens\\\"",
            "  qamcp screenshot -c \"<clientId>\" -o \".\\screen.png\"",
            "",
            "选项:",
            "  --client, -c <id>       必填，Unity 客户端 ID。",
            "  --output, -o <path>     必填，本地保存路径；如果是目录则使用 artifact fileName。",
        ],
        Some("sequence") => vec![
            "sequence:",
            "  当前推荐 QA Agent 指令集不使用 qamcp sequence，help 不展示其参数。",
            "  请使用 qamcp run 执行单个 QaTest 方法，并用 results、wait 或 watch 获取结果。",
        ],
        Some("stop") => vec![
            "stop:",
            "  当前推荐 QA Agent 指令集不使用 qamcp stop，help 不展示其参数。",
            "  请通过 run、results、wait 和 watch 完成常规 QA 调用流程。",
        ],
        Some("wait") => vec![
            "wait 选项:",
            "  --request, -r <id>      匹配请求 ID。",
            "  --client, -c <id>       匹配客户端 ID。",
            "  --timeout <ms>          等待超时。默认值: 180000。",
            "  --text, --human        输出人类可读等待结果。",
            "",
            "用法:",
            "  qamcp wait --client \"<clientId>\" --request \"<requestId>\" --timeout 180000",
        ],
        Some("watch") => vec![
            "watch 选项:",
            "  --event <type>          事件类型，例如 qa_result。",
            "  --duration <ms>         监听时长。默认值: 10000。",
            "  --limit, -l <number>    最多保留的事件数量。默认值: 100。",
            "  --text, --human        输出人类可读事件数据。",
            "",
            "用法:",
            "  qamcp watch --event qa_result --duration 5000 --limit 50",
        ],
        _ => vec![
            "用法:",
            "  qamcp                       显示此帮助。",
            "  qamcp <command> [options]   执行一个 CLI command，默认输出 JSON。",
            "  qamcp mcp                   启动 stdio MCP server。",
            "",
            "功能:",
            "  qamcp 是 QA Test Framework 的本地 MCP/CLI 工具，用于连接 QA register server，",
            "  发现在线 Unity 客户端、搜索和描述可执行 QaTest 方法、运行单个测试指令、查看结果和监听事件。",
            "",
            "命令:",
            "  config                 配置 QA register server 地址。",
            "  health                 检查 QA register server 健康状态。",
            "  clients                列出在线 Unity 客户端。",
            "  search                 搜索已注册的 QaTest 方法。",
            "  describe               查询一个 QaTest 方法详情。",
            "  run                    运行单个 QaTest 方法并等待结果。",
            "  capture-screenshot     执行截图 QaTest，下载 artifact 到本地路径。",
            "  results                查看最近执行结果。",
            "  wait                   等待指定请求的执行结果。",
            "  watch                  短时间监听 QA WebSocket 事件。",
            "  mcp                    启动 stdio MCP server。",
            "  help [command]         显示全局帮助或指定 command 的帮助。",
            "",
            "全局选项:",
            "  --json                 输出格式化 JSON。默认行为。",
            "  --text, --human        输出人类可读摘要。",
            "  --help, -h             显示帮助，例如 qamcp run --help。",
            "",
            "AI 调用提示:",
            "  默认输出 JSON；QaTest 方法由当前 Unity 项目动态注册，先用 search/describe 获取实际 methodId。",
            "  search/describe 会返回 allowParallelExecution；值为 true 的只读查询类方法可由 server 自动并行下发。",
            "",
            "常用示例:",
            "  qamcp --help",
            "  qamcp config",
            "  qamcp config set serverUrl http://localhost:3000",
            "  qamcp health",
            "  qamcp clients",
            "  qamcp search --client \"<clientId>\" --limit 10000",
            "  qamcp describe --client \"<clientId>\" --query \"<methodName-or-methodId>\"",
            "  qamcp run --client \"<clientId>\" --tool \"<methodId>\" --name \"<methodName>\" -a \"arg1\" -a \"arg2\" --timeout 70000",
            "  qamcp run --client \"<clientId>\" --tool \"<methodId>\" --name \"<methodName>\" -a \"\" --arg-file \".\\record.txt\" -a \"1\" --timeout 70000",
            "  qamcp screenshot --client \"<clientId>\" --output \".\\screens\\\"",
            "  qamcp results --client \"<clientId>\" --limit 20",
            "  qamcp results --client \"<clientId>\" --status failed --limit 20",
            "  qamcp wait --client \"<clientId>\" --request \"<requestId>\" --timeout 180000",
            "  qamcp watch --event qa_result --duration 5000 --limit 50",
        ],
    };

    println!("{}", lines.join("\n"));
}

fn print_config(data: &Value) {
    if data.get("action").and_then(Value::as_str) == Some("path") {
        println!("{}", text(data, "configPath").unwrap_or_default());
        return;
    }
    if data.get("action").and_then(Value::as_str) == Some("get") {
        println!("{}", text(data, "value").unwrap_or_default());
        return;
    }
    if data.get("saved").and_then(Value::as_bool) == Some(true) {
        println!("已保存 qamcp 配置。");
    }
    print_key_values(&[
        ("Config", text(data, "configPath")),
        (
            "Exists",
            format_bool(data.get("exists").and_then(Value::as_bool)),
        ),
        ("Server", text(data, "serverUrl")),
    ]);
}

fn print_health(data: &Value) {
    let health = data.get("health").unwrap_or(&Value::Null);
    print_key_values(&[
        ("Server", text(data, "serverUrl")),
        (
            "Status",
            text(health, "status").or_else(|| {
                health
                    .get("ok")
                    .and_then(Value::as_bool)
                    .map(|ok| if ok { "ok" } else { "unknown" }.to_string())
            }),
        ),
        (
            "Unity clients",
            value_to_string(health.get("unityClientCount")),
        ),
        ("Web clients", value_to_string(health.get("webClientCount"))),
        (
            "Controllers",
            value_to_string(health.get("controllerCount")),
        ),
        (
            "Execution timeout",
            format_ms(health.get("executionTimeoutMs")),
        ),
        (
            "Unity stale timeout",
            format_ms(health.get("unityHeartbeatStaleMs")),
        ),
        (
            "Web auth required",
            format_bool(
                health
                    .get("webConsoleAuthRequired")
                    .and_then(Value::as_bool),
            ),
        ),
        ("Uptime", format_seconds(health.get("uptime"))),
    ]);
}

fn print_clients(data: &Value) {
    let clients = array(data, "clients");
    println!(
        "Unity clients: {}",
        data.get("count").and_then(Value::as_u64).unwrap_or(0)
    );
    if clients.is_empty() {
        println!("No Unity clients are connected.");
        return;
    }

    let rows = clients
        .iter()
        .map(|client| {
            vec![
                text(client, "clientId").unwrap_or_default(),
                text(client, "name").unwrap_or_default(),
                text(client, "ipAddress")
                    .or_else(|| text(client, "remoteAddress"))
                    .unwrap_or_default(),
                text(client, "platform").unwrap_or_default(),
                if client.get("available").and_then(Value::as_bool) == Some(false) {
                    "no".to_string()
                } else {
                    "yes".to_string()
                },
                if client.get("running").and_then(Value::as_bool) == Some(true)
                    || client.get("clientBusy").and_then(Value::as_bool) == Some(true)
                {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
                value_to_string(client.get("methodCount"))
                    .or_else(|| {
                        client
                            .get("methods")
                            .and_then(Value::as_array)
                            .map(|v| v.len().to_string())
                    })
                    .unwrap_or_default(),
                text(client, "currentMethodName")
                    .or_else(|| text(client, "currentRequestId"))
                    .unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>();
    print_table(
        &[
            "clientId",
            "name",
            "ip",
            "platform",
            "available",
            "busy",
            "tools",
            "current",
        ],
        rows,
    );
}

fn print_tools(data: &Value, options: &CliOptions) {
    let methods = array(data, "methods");
    println!(
        "Methods: {}{}",
        data.get("count").and_then(Value::as_u64).unwrap_or(0),
        if data.get("truncated").and_then(Value::as_bool) == Some(true) {
            format!(" (showing {})", methods.len())
        } else {
            String::new()
        }
    );
    if methods.is_empty() {
        println!("No methods matched.");
        return;
    }
    if options.simple == Some(true) {
        print_table(
            &["methodName", "methodId"],
            methods
                .iter()
                .map(|method| {
                    vec![
                        text(method, "methodName").unwrap_or_default(),
                        text(method, "methodId").unwrap_or_default(),
                    ]
                })
                .collect(),
        );
        return;
    }
    print_table(
        &[
            "clientId",
            "methodName",
            "methodId",
            "parallel",
            "params",
            "returnType",
            "description",
        ],
        methods
            .iter()
            .map(|method| {
                vec![
                    text(method, "clientId").unwrap_or_default(),
                    text(method, "methodName").unwrap_or_default(),
                    text(method, "methodId").unwrap_or_default(),
                    format_bool(
                        method
                            .get("allowParallelExecution")
                            .and_then(Value::as_bool),
                    )
                    .unwrap_or_default(),
                    format_parameters(method.get("parameters")),
                    text(method, "returnType").unwrap_or_default(),
                    text(method, "description").unwrap_or_default(),
                ]
            })
            .collect(),
    );
}

fn print_tool(data: &Value) {
    if let Some(methods) = data.get("methods").and_then(Value::as_array) {
        println!(
            "Methods: {}",
            data.get("count")
                .and_then(Value::as_u64)
                .unwrap_or(methods.len() as u64)
        );
        for (index, item) in methods.iter().enumerate() {
            if index > 0 {
                println!();
            }
            print_tool_detail(item.get("method").unwrap_or(item), item);
        }
        return;
    }

    let method = data.get("method").unwrap_or(&Value::Null);
    print_tool_detail(method, data);
}

fn print_tool_detail(method: &Value, match_data: &Value) {
    print_key_values(&[
        ("Query", text(match_data, "query")),
        ("Method", text(method, "methodName")),
        ("ID", text(method, "methodId")),
        ("Client", text(method, "clientId")),
        ("Client name", text(method, "clientName")),
        ("Client IP", text(method, "clientIpAddress")),
        ("Declaring type", text(method, "declaringType")),
        ("Return type", text(method, "returnType")),
        (
            "Static",
            format_bool(method.get("isStatic").and_then(Value::as_bool)),
        ),
        (
            "Parallel",
            format_bool(
                method
                    .get("allowParallelExecution")
                    .and_then(Value::as_bool),
            ),
        ),
        (
            "Parameters",
            Some(format_parameters(method.get("parameters"))),
        ),
        ("Description", text(method, "description")),
        ("Match", text(match_data, "matchType")),
    ]);
}

fn print_results(data: &Value) {
    let results = array(data, "results");
    println!(
        "Results: {}",
        data.get("count").and_then(Value::as_u64).unwrap_or(0)
    );
    if results.is_empty() {
        println!("No results matched.");
        return;
    }
    print_table(
        &[
            "status",
            "clientId",
            "methodName",
            "requestId",
            "sequenceId",
            "duration",
            "result",
            "error",
        ],
        results
            .iter()
            .map(|result| {
                vec![
                    text(result, "status").unwrap_or_default(),
                    text(result, "clientId").unwrap_or_default(),
                    text(result, "methodName")
                        .or_else(|| text(result, "methodId"))
                        .unwrap_or_default(),
                    text(result, "requestId").unwrap_or_default(),
                    text(result, "sequenceId").unwrap_or_default(),
                    format_ms(result.get("durationMs")).unwrap_or_default(),
                    stringify_brief(result.get("result")),
                    text(result, "error").unwrap_or_default(),
                ]
            })
            .collect(),
    );
}

fn print_execution(data: &Value) {
    let result = data.get("result").unwrap_or(&Value::Null);
    let execution = data.get("execution").unwrap_or(&Value::Null);
    print_key_values(&[
        (
            "Status",
            text(result, "status").or_else(|| {
                result
                    .get("success")
                    .and_then(Value::as_bool)
                    .map(|ok| if ok { "success" } else { "failed" }.to_string())
            }),
        ),
        (
            "Request",
            text(result, "requestId").or_else(|| text(execution, "requestId")),
        ),
        (
            "Client",
            text(result, "clientId").or_else(|| text(execution, "clientId")),
        ),
        (
            "Method",
            text(result, "methodName")
                .or_else(|| text(result, "methodId"))
                .or_else(|| text(execution, "methodId")),
        ),
        ("Duration", format_ms(result.get("durationMs"))),
        ("Result", Some(stringify_brief(result.get("result")))),
        ("Error", text(result, "error")),
    ]);
}

fn print_artifact_save(data: &Value) {
    print_key_values(&[
        ("Saved", text(data, "savedPath")),
        ("Artifact", text(data, "artifactId")),
        ("File", text(data, "fileName")),
        (
            "Client",
            text(data, "clientName").or_else(|| text(data, "clientId")),
        ),
        ("Size", value_to_string(data.get("sizeBytes"))),
        ("SHA-256", text(data, "sha256")),
    ]);
}

fn print_sequence(data: &Value) {
    let sequence = data.get("sequence").unwrap_or(&Value::Null);
    print_key_values(&[
        (
            "Status",
            text(sequence, "status").or_else(|| Some("unknown".to_string())),
        ),
        (
            "Sequence",
            text(data, "sequenceId").or_else(|| text(sequence, "sequenceId")),
        ),
        ("Client", text(sequence, "clientId")),
        (
            "Steps",
            Some(format!(
                "{}/{}",
                value_to_string(sequence.get("completedSteps"))
                    .unwrap_or_else(|| { array(data, "stepResults").len().to_string() }),
                value_to_string(sequence.get("totalSteps"))
                    .unwrap_or_else(|| { array(data, "startedSteps").len().to_string() })
            )),
        ),
        (
            "Failed",
            value_to_string(sequence.get("failedSteps")).or_else(|| Some("0".to_string())),
        ),
    ]);
}

fn print_stop(data: &Value) {
    let acknowledgement = data.get("acknowledgement").unwrap_or(&Value::Null);
    print_key_values(&[
        (
            "Status",
            text(acknowledgement, "type").or_else(|| Some("stop_accepted".to_string())),
        ),
        ("Controller", text(data, "controllerId")),
        ("Request", text(acknowledgement, "requestId")),
        ("Sequence", text(acknowledgement, "sequenceId")),
        ("Reason", text(acknowledgement, "reason")),
    ]);
}

fn print_wait(data: &Value) {
    if data.get("result").is_some() {
        println!(
            "Matched {} from {}.",
            text(data, "eventType").unwrap_or_default(),
            text(data, "source").unwrap_or_default()
        );
        print_execution(
            &serde_json::json!({ "result": data.get("result").cloned().unwrap_or(Value::Null) }),
        );
        return;
    }
    if data.get("sequence").is_some() {
        println!(
            "Matched {} from {}.",
            text(data, "eventType").unwrap_or_default(),
            text(data, "source").unwrap_or_default()
        );
        print_sequence(data);
        return;
    }
    println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
}

fn print_watch(data: &Value) {
    let events = array(data, "events");
    println!(
        "Events: {}{}",
        data.get("count").and_then(Value::as_u64).unwrap_or(0),
        if data.get("truncated").and_then(Value::as_bool) == Some(true) {
            format!(
                " ({} dropped)",
                data.get("droppedEvents")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            )
        } else {
            String::new()
        }
    );
    if events.is_empty() {
        println!("No events captured.");
        return;
    }
    print_table(
        &["index", "receivedAt", "type", "summary"],
        events
            .iter()
            .map(|event| {
                vec![
                    value_to_string(event.get("index")).unwrap_or_default(),
                    text(event, "receivedAt").unwrap_or_default(),
                    text(event, "type").unwrap_or_default(),
                    summarize_event(event.get("message").unwrap_or(&Value::Null)),
                ]
            })
            .collect(),
    );
}

fn print_key_values(rows: &[(&str, Option<String>)]) {
    let rows = rows
        .iter()
        .filter_map(|(key, value)| value.as_ref().filter(|v| !v.is_empty()).map(|v| (*key, v)))
        .collect::<Vec<_>>();
    let width = rows.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    for (key, value) in rows {
        println!("{key:width$}  {value}");
    }
}

fn print_table(columns: &[&str], rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        return;
    }
    let normalized_rows = rows
        .into_iter()
        .map(|row| row.into_iter().map(single_line).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let widths = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let row_width = normalized_rows
                .iter()
                .map(|row| row.get(index).map_or(0, |value| value.len()))
                .max()
                .unwrap_or(0);
            column.len().max(row_width).min(80)
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        columns
            .iter()
            .enumerate()
            .map(|(index, column)| format!("{column:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    );
    println!(
        "{}",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in normalized_rows {
        println!(
            "{}",
            row.iter()
                .enumerate()
                .map(|(index, value)| {
                    format!(
                        "{:<width$}",
                        truncate(value, widths[index]),
                        width = widths[index]
                    )
                })
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn array<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn single_line(value: String) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(value: &str, max_length: usize) -> String {
    if value.len() <= max_length {
        return value.to_string();
    }
    if max_length <= 3 {
        return value.chars().take(max_length).collect();
    }
    format!(
        "{}...",
        value.chars().take(max_length - 3).collect::<String>()
    )
}

fn format_parameters(parameters: Option<&Value>) -> String {
    parameters
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|parameter| {
            format!(
                "{}:{}",
                text(parameter, "name").unwrap_or_else(|| "?".to_string()),
                text(parameter, "type").unwrap_or_else(|| "?".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_ms(value: Option<&Value>) -> Option<String> {
    value_to_string(value).map(|value| format!("{value} ms"))
}

fn format_seconds(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_f64)
        .map(|value| format!("{value:.1} s"))
}

fn format_bool(value: Option<bool>) -> Option<String> {
    value.map(format_bool_text)
}

fn format_bool_text(value: bool) -> String {
    if value { "yes" } else { "no" }.to_string()
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        value => Some(value.to_string()),
    }
}

fn stringify_brief(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

fn summarize_event(message: &Value) -> String {
    if let Some(result) = message.get("result") {
        return format!(
            "{} {} {}",
            text(result, "status").unwrap_or_default(),
            text(result, "methodName")
                .or_else(|| text(result, "methodId"))
                .unwrap_or_default(),
            text(result, "requestId").unwrap_or_default()
        )
        .trim()
        .to_string();
    }
    if let Some(sequence) = message.get("sequence") {
        return format!(
            "{} {}",
            text(sequence, "status").unwrap_or_default(),
            text(sequence, "sequenceId").unwrap_or_default()
        )
        .trim()
        .to_string();
    }
    if let Some(client) = message.get("client") {
        return format!(
            "{} {}",
            text(client, "clientId").unwrap_or_default(),
            text(client, "name").unwrap_or_default()
        )
        .trim()
        .to_string();
    }
    text(message, "error").unwrap_or_else(|| message.to_string())
}
