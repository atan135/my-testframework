pub mod output;
pub mod parser;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::{
    cli::{
        output::{print_cli_output, print_help},
        parser::{CliOptions, collect_cli_arguments, load_sequence_steps, parse_cli_args},
    },
    config::{get_qamcp_config_path, read_qamcp_config, save_qa_server_url},
    constants::{
        EXECUTE_TIMEOUT_MS, QAMCP_VERSION, SEQUENCE_TIMEOUT_MS, WAIT_RESULT_TIMEOUT_MS,
        WATCH_EVENTS_DURATION_MS, WS_CONNECT_TIMEOUT_MS,
    },
    qa::{
        api::{
            find_methods_data, get_health_data, get_method_data, get_results_data,
            list_unity_clients_data,
        },
        artifacts::capture_screenshot_data,
        ws::{
            execute_method_via_websocket, execute_sequence_via_websocket, stop_via_websocket,
            wait_for_result_via_websocket, watch_events_via_websocket,
        },
    },
};

pub fn should_run_cli(args: &[String]) -> bool {
    if args.is_empty() {
        return true;
    }
    if ["mcp", "serve", "stdio"].contains(&args[0].as_str())
        && args
            .get(1)
            .is_some_and(|arg| ["--help", "-h"].contains(&arg.as_str()))
    {
        return true;
    }
    !["mcp", "serve", "stdio"].contains(&args[0].as_str())
}

pub async fn run_cli(argv: &[String]) -> Result<()> {
    if argv.is_empty() || ["--help", "-h", "help"].contains(&argv[0].as_str()) {
        let command = if argv.first().is_some_and(|arg| arg == "help") {
            argv.get(1).map(|arg| parser::normalize_cli_command(arg))
        } else {
            None
        };
        print_help(command.as_deref());
        return Ok(());
    }

    if ["--version", "-v", "version"].contains(&argv[0].as_str()) {
        println!("{QAMCP_VERSION}");
        return Ok(());
    }

    let parsed = parse_cli_args(argv)?;
    if parsed.options.server_url.is_some() && parsed.command != "config" {
        bail!(
            "--server-url 只支持 \"qamcp config\"。请先运行 \"qamcp config set serverUrl <url>\"。"
        );
    }
    if parsed.options.help {
        print_help(Some(&parsed.command));
        return Ok(());
    }

    let mut options = parsed.options;
    let data = match parsed.command.as_str() {
        "config" => configure_qamcp(&options)?,
        "health" => get_health_data().await?,
        "clients" => list_unity_clients_data(options.methods == Some(true)).await?,
        "search" => {
            let query = options
                .query
                .clone()
                .unwrap_or_else(|| options.positionals.join(" "));
            let mut data = find_methods_data(
                options.client.as_deref(),
                &query,
                options.limit.unwrap_or(50).max(0) as usize,
            )
            .await?;
            if options.simple != Some(false) {
                options.simple = Some(true);
                data = simplify_tools_data(data);
            }
            data
        }
        "describe" => {
            let query = options
                .query
                .clone()
                .unwrap_or_else(|| options.positionals.join(" "));
            get_method_data(options.client.as_deref(), &query).await?
        }
        "results" => {
            get_results_data(
                options.client.as_deref(),
                options.status.as_deref(),
                options.sequence.as_deref(),
                options.limit.unwrap_or(20).max(0) as usize,
            )
            .await?
        }
        "run" => {
            let method_from_position = options.method.is_none() && !options.positionals.is_empty();
            if method_from_position {
                options.method = options.positionals.first().cloned();
            }
            execute_method_via_websocket(
                require_option(&options.client, "run requires --client <clientId>.")?,
                require_option(&options.method, "run requires --tool <toolId>.")?,
                options.name.clone(),
                collect_cli_arguments(&options, if method_from_position { 1 } else { 0 })?,
                options.timeout.unwrap_or(EXECUTE_TIMEOUT_MS),
            )
            .await?
        }
        "capture-screenshot" => {
            let save_path_from_position =
                options.save_path.is_none() && !options.positionals.is_empty();
            if save_path_from_position {
                options.save_path = options.positionals.first().cloned();
            }
            capture_screenshot_data(
                require_option(
                    &options.client,
                    "capture-screenshot requires --client <clientId>.",
                )?,
                require_option(
                    &options.save_path,
                    "capture-screenshot requires --output <path>.",
                )?,
            )
            .await?
        }
        "sequence" => {
            execute_sequence_via_websocket(
                require_option(&options.client, "sequence requires --client <clientId>.")?,
                load_sequence_steps(&options)?,
                options.stop_on_failure != Some(false),
                options.step_delay.unwrap_or(0),
                options.timeout.unwrap_or(SEQUENCE_TIMEOUT_MS),
            )
            .await?
        }
        "stop" => {
            if options.request.is_none() && options.sequence.is_none() {
                bail!("stop requires --request <requestId> or --sequence <sequenceId>.");
            }
            stop_via_websocket(
                options.request.clone(),
                options.sequence.clone(),
                options
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Stopped by qamcp CLI.".to_string()),
                options.timeout.unwrap_or(WS_CONNECT_TIMEOUT_MS),
            )
            .await?
        }
        "wait" => {
            wait_for_result_via_websocket(json!({
                "requestId": options.request,
                "sequenceId": options.sequence,
                "clientId": options.client,
                "methodId": options.method,
                "methodName": options.name,
                "status": options.status,
                "includeHistory": options.history != Some(false),
                "timeoutMs": options.timeout.unwrap_or(WAIT_RESULT_TIMEOUT_MS),
            }))
            .await?
        }
        "watch" => {
            watch_events_via_websocket(
                options.duration.unwrap_or(WATCH_EVENTS_DURATION_MS),
                options.event.clone(),
                options.snapshot != Some(false),
                options.limit.unwrap_or(100).max(0) as usize,
            )
            .await?
        }
        _ => bail!(
            "Unknown command: {}. Run \"qamcp --help\" for usage.",
            parsed.command
        ),
    };

    print_cli_output(&parsed.command, &data, &options)?;
    Ok(())
}

fn configure_qamcp(options: &CliOptions) -> Result<Value> {
    if let Some(server_url) = &options.server_url {
        return save_qa_server_url(server_url);
    }

    let action = options.positionals.first().map(String::as_str);
    let key = options.positionals.get(1).map(String::as_str);
    let value = options.positionals.get(2).map(String::as_str);
    let extra = options.positionals.len().saturating_sub(3);

    if action.is_none() || action == Some("show") {
        let mut config = read_qamcp_config()?;
        if let Value::Object(map) = &mut config {
            map.insert("action".to_string(), Value::String("show".to_string()));
        }
        return Ok(config);
    }

    if action == Some("path") {
        return Ok(json!({
            "action": "path",
            "configPath": get_qamcp_config_path()?.to_string_lossy(),
        }));
    }

    if action == Some("get") {
        let config_key = normalize_config_key(key.unwrap_or("serverUrl"));
        if config_key != "serverUrl" {
            bail!("未知配置项: {}。", key.unwrap_or(""));
        }
        return Ok(json!({
            "action": "get",
            "key": config_key,
            "value": read_qamcp_config()?.get("serverUrl").cloned().unwrap_or(Value::Null),
            "configPath": get_qamcp_config_path()?.to_string_lossy(),
        }));
    }

    if action == Some("set") {
        let config_key = normalize_config_key(key.unwrap_or(""));
        if config_key != "serverUrl" {
            bail!(
                "未知配置项: {}。运行 \"qamcp config --help\" 查看用法。",
                key.unwrap_or("")
            );
        }
        if value.is_none() || extra > 0 {
            bail!("用法: qamcp config set serverUrl <url>");
        }
        return save_qa_server_url(value.unwrap());
    }

    if normalize_config_key(action.unwrap_or("")) == "serverUrl" {
        if key.is_none() || value.is_some() || extra > 0 {
            bail!("用法: qamcp config serverUrl <url>");
        }
        return save_qa_server_url(key.unwrap());
    }

    bail!(
        "未知 config 操作: {}。运行 \"qamcp config --help\" 查看用法。",
        action.unwrap_or("")
    );
}

fn normalize_config_key(key: &str) -> String {
    let normalized = key.trim().to_lowercase().replace(['-', '_'], "");
    if normalized == "serverurl" || normalized == "url" {
        "serverUrl".to_string()
    } else {
        normalized
    }
}

fn require_option(value: &Option<String>, message: &str) -> Result<String> {
    match value {
        Some(value) if !value.is_empty() => Ok(value.clone()),
        _ => bail!("{message}"),
    }
}

fn simplify_tools_data(mut data: Value) -> Value {
    if let Some(methods) = data.get_mut("methods").and_then(Value::as_array_mut) {
        for method in methods {
            *method = json!({
                "methodId": simplify_method_id(method.get("methodId").and_then(Value::as_str).unwrap_or("")),
                "methodName": method.get("methodName").and_then(Value::as_str).unwrap_or(""),
            });
        }
    }
    data
}

fn simplify_method_id(method_id: &str) -> String {
    method_id
        .split_once('(')
        .map(|(name, _)| name)
        .unwrap_or(method_id)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_method_id_drops_parameter_signature() {
        assert_eq!(
            simplify_method_id("OperateListItemAndGetData(System.String,System.Int32)"),
            "OperateListItemAndGetData"
        );
        assert_eq!(simplify_method_id("Ping()"), "Ping");
        assert_eq!(simplify_method_id("AlreadySimple"), "AlreadySimple");
    }
}
