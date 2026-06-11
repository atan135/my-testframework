use std::fs;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default)]
pub struct CliOptions {
    pub positionals: Vec<String>,
    pub help: bool,
    pub text: Option<bool>,
    pub methods: Option<bool>,
    pub simple: Option<bool>,
    pub snapshot: Option<bool>,
    pub stop_on_failure: Option<bool>,
    pub history: Option<bool>,
    pub client: Option<String>,
    pub server_url: Option<String>,
    pub method: Option<String>,
    pub name: Option<String>,
    pub query: Option<String>,
    pub save_path: Option<String>,
    pub status: Option<String>,
    pub sequence: Option<String>,
    pub request: Option<String>,
    pub reason: Option<String>,
    pub arg: Vec<String>,
    pub arg_sources: Vec<CliArgumentSource>,
    pub args: Option<Value>,
    pub step: Vec<Value>,
    pub steps: Option<Value>,
    pub steps_file: Option<String>,
    pub event: Vec<String>,
    pub limit: Option<i64>,
    pub timeout: Option<u64>,
    pub duration: Option<u64>,
    pub step_delay: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliArgumentSource {
    Value(String),
    File(String),
}

pub struct ParsedCliArgs {
    pub command: String,
    pub options: CliOptions,
}

pub fn parse_cli_args(argv: &[String]) -> Result<ParsedCliArgs> {
    let raw_command = argv.first().map(String::as_str).unwrap_or("");
    if raw_command.is_empty() || raw_command.starts_with('-') {
        bail!("Missing command. Run \"qamcp --help\" for usage.");
    }
    let command = normalize_cli_command(raw_command);
    let mut options = CliOptions::default();
    let mut index = 1;

    while index < argv.len() {
        let arg = &argv[index];
        if arg == "--" {
            options
                .positionals
                .extend(argv[index + 1..].iter().cloned());
            break;
        }
        if !arg.starts_with('-') {
            options.positionals.push(arg.clone());
            index += 1;
            continue;
        }

        let (raw_name, inline_value) = split_inline_option(arg);
        let name = normalize_cli_option_name(raw_name);
        let Some(spec) = get_cli_option_spec(&name) else {
            bail!("Unknown option: {raw_name}.");
        };

        let value = if spec.option_type == OptionType::Boolean {
            let value = inline_value
                .map(|value| parse_boolean_option(value, raw_name))
                .transpose()?
                .unwrap_or(true);
            CliValue::Bool(value)
        } else {
            let value = if let Some(value) = inline_value {
                value.to_string()
            } else {
                index += 1;
                argv.get(index)
                    .cloned()
                    .ok_or_else(|| anyhow!("Option {raw_name} requires a value."))?
            };
            parse_cli_option_value(&value, raw_name, spec)?
        };

        set_cli_option(&mut options, spec, value)?;
        index += 1;
    }

    validate_command_options(&command, &options)?;

    Ok(ParsedCliArgs { command, options })
}

fn validate_command_options(command: &str, options: &CliOptions) -> Result<()> {
    if command == "capture-screenshot" {
        if options.method.is_some() {
            bail!(
                "capture-screenshot does not accept --tool/--method; it uses the default screenshot QaTest method."
            );
        }
        if options.timeout.is_some() {
            bail!("capture-screenshot does not accept --timeout; it uses the default timeout.");
        }
    }

    Ok(())
}

pub fn normalize_cli_command(command: &str) -> String {
    match command {
        "status" => "health",
        "serve" | "stdio" => "mcp",
        "qa_health" => "health",
        "list" | "client" | "qa_list_unity_clients" => "clients",
        "cfg" => "config",
        "search" => "search",
        "describe" => "describe",
        "history" | "result" | "qa_get_results" => "results",
        "exec" | "execute" | "qa_execute_method" => "run",
        "capture-screenshot" | "screenshot" | "qa_capture_screenshot" => "capture-screenshot",
        "seq" | "qa_execute_sequence" => "sequence",
        "stop-execution" | "qa_stop_execution" | "stop-sequence" | "stopseq"
        | "qa_stop_sequence" => "stop",
        "events" | "qa_watch_events" => "watch",
        "qa_wait_for_result" => "wait",
        other => other,
    }
    .to_string()
}

pub fn collect_cli_arguments(options: &CliOptions, positional_offset: usize) -> Result<Vec<Value>> {
    if options.args.is_some() && (!options.arg_sources.is_empty() || !options.arg.is_empty()) {
        bail!(
            "Use either repeated -a/--arg/--arg-file values or legacy --args JSON array, not both."
        );
    }
    if let Some(args) = &options.args {
        if let Some(args) = args.as_array() {
            return Ok(args.clone());
        }
        bail!(
            "--args must be a JSON array. Prefer repeated -a/--arg values for shell-safe qamcp run calls."
        );
    }
    if !options.arg_sources.is_empty() {
        return options
            .arg_sources
            .iter()
            .map(|source| match source {
                CliArgumentSource::Value(value) => Ok(json!(value)),
                CliArgumentSource::File(path) => {
                    let text = fs::read_to_string(path)
                        .with_context(|| format!("Failed to read argument file {path}"))?;
                    Ok(json!(text))
                }
            })
            .collect();
    }
    if !options.arg.is_empty() {
        return Ok(options.arg.iter().map(|arg| json!(arg)).collect());
    }
    Ok(options
        .positionals
        .iter()
        .skip(positional_offset)
        .map(|arg| json!(arg))
        .collect())
}

pub fn load_sequence_steps(options: &CliOptions) -> Result<Vec<Value>> {
    if let Some(steps_file) = &options.steps_file {
        let text = fs::read_to_string(steps_file)
            .with_context(|| format!("Failed to read JSON file {steps_file}"))?;
        let steps: Value = serde_json::from_str(&text)
            .with_context(|| format!("Failed to read JSON file {steps_file}"))?;
        return validate_sequence_steps(steps);
    }

    if let Some(steps) = &options.steps {
        return validate_sequence_steps(steps.clone());
    }
    if !options.step.is_empty() {
        return validate_sequence_steps(Value::Array(options.step.clone()));
    }

    bail!("sequence requires --steps <json>, --step <json>, or --steps-file <path>.");
}

fn normalize_cli_option_name(name: &str) -> String {
    match name {
        "-c" => "--client",
        "-m" => "--method",
        "-t" => "--tool",
        "-q" => "--query",
        "-n" => "--name",
        "-a" => "--arg",
        "-l" => "--limit",
        "-s" => "--status",
        "-r" => "--request",
        "-h" => "--help",
        other => other,
    }
    .to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptionType {
    Boolean,
    String,
    Number,
    Json,
    Csv,
}

#[derive(Clone, Copy)]
struct OptionSpec {
    key: &'static str,
    option_type: OptionType,
    repeat: bool,
    fixed_bool: Option<bool>,
}

#[derive(Debug)]
enum CliValue {
    Bool(bool),
    String(String),
    Number(i64),
    Json(Value),
    Csv(Vec<String>),
}

fn get_cli_option_spec(name: &str) -> Option<OptionSpec> {
    let boolean = |key, fixed_bool| OptionSpec {
        key,
        option_type: OptionType::Boolean,
        repeat: false,
        fixed_bool,
    };
    let string = |key, repeat| OptionSpec {
        key,
        option_type: OptionType::String,
        repeat,
        fixed_bool: None,
    };
    let number = |key| OptionSpec {
        key,
        option_type: OptionType::Number,
        repeat: false,
        fixed_bool: None,
    };
    let json = |key, repeat| OptionSpec {
        key,
        option_type: OptionType::Json,
        repeat,
        fixed_bool: None,
    };

    Some(match name {
        "--help" => boolean("help", None),
        "--json" | "--raw" => boolean("text", Some(false)),
        "--text" | "--human" | "--no-json" => boolean("text", Some(true)),
        "--methods" | "--tools" => boolean("methods", None),
        "--no-methods" | "--no-tools" => boolean("methods", Some(false)),
        "--simple" => boolean("simple", None),
        "--full" | "--no-simple" => boolean("simple", Some(false)),
        "--snapshot" => boolean("snapshot", None),
        "--no-snapshot" => boolean("snapshot", Some(false)),
        "--stop-on-failure" => boolean("stopOnFailure", None),
        "--no-stop-on-failure" => boolean("stopOnFailure", Some(false)),
        "--history" => boolean("history", None),
        "--no-history" => boolean("history", Some(false)),
        "--client" => string("client", false),
        "--server-url" => string("serverUrl", false),
        "--method" | "--tool" => string("method", false),
        "--name" => string("name", false),
        "--query" => string("query", false),
        "--save-path" | "--output" | "-o" => string("savePath", false),
        "--status" => string("status", false),
        "--sequence" => string("sequence", false),
        "--request" => string("request", false),
        "--reason" => string("reason", false),
        "--arg" => string("arg", true),
        "--arg-file" => string("argFile", true),
        "--args" => json("args", false),
        "--step" => json("step", true),
        "--steps" => json("steps", false),
        "--steps-file" => string("stepsFile", false),
        "--event" => OptionSpec {
            key: "event",
            option_type: OptionType::Csv,
            repeat: true,
            fixed_bool: None,
        },
        "--limit" => number("limit"),
        "--timeout" => number("timeout"),
        "--duration" => number("duration"),
        "--step-delay" => number("stepDelay"),
        _ => return None,
    })
}

fn parse_cli_option_value(value: &str, option_name: &str, spec: OptionSpec) -> Result<CliValue> {
    Ok(match spec.option_type {
        OptionType::Boolean => CliValue::Bool(parse_boolean_option(value, option_name)?),
        OptionType::String => CliValue::String(value.to_string()),
        OptionType::Number => {
            let number_value = value
                .parse::<f64>()
                .with_context(|| format!("Option {option_name} requires a number."))?;
            if !number_value.is_finite() {
                bail!("Option {option_name} requires a number.");
            }
            CliValue::Number(number_value.floor() as i64)
        }
        OptionType::Json => CliValue::Json(
            serde_json::from_str(value)
                .with_context(|| format!("Option {option_name} requires valid JSON"))?,
        ),
        OptionType::Csv => CliValue::Csv(
            value
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect(),
        ),
    })
}

fn parse_boolean_option(value: &str, option_name: &str) -> Result<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("Option {option_name} requires a boolean value."),
    }
}

fn set_cli_option(options: &mut CliOptions, spec: OptionSpec, value: CliValue) -> Result<()> {
    let value = match (spec.fixed_bool, value) {
        (Some(value), CliValue::Bool(_)) => CliValue::Bool(value),
        (_, value) => value,
    };

    match (spec.key, value, spec.repeat) {
        ("help", CliValue::Bool(value), _) => options.help = value,
        ("text", CliValue::Bool(value), _) => options.text = Some(value),
        ("methods", CliValue::Bool(value), _) => options.methods = Some(value),
        ("simple", CliValue::Bool(value), _) => options.simple = Some(value),
        ("snapshot", CliValue::Bool(value), _) => options.snapshot = Some(value),
        ("stopOnFailure", CliValue::Bool(value), _) => options.stop_on_failure = Some(value),
        ("history", CliValue::Bool(value), _) => options.history = Some(value),
        ("client", CliValue::String(value), _) => options.client = Some(value),
        ("serverUrl", CliValue::String(value), _) => options.server_url = Some(value),
        ("method", CliValue::String(value), _) => options.method = Some(value),
        ("name", CliValue::String(value), _) => options.name = Some(value),
        ("query", CliValue::String(value), _) => options.query = Some(value),
        ("savePath", CliValue::String(value), _) => options.save_path = Some(value),
        ("status", CliValue::String(value), _) => options.status = Some(value),
        ("sequence", CliValue::String(value), _) => options.sequence = Some(value),
        ("request", CliValue::String(value), _) => options.request = Some(value),
        ("reason", CliValue::String(value), _) => options.reason = Some(value),
        ("arg", CliValue::String(value), true) => {
            options.arg.push(value.clone());
            options.arg_sources.push(CliArgumentSource::Value(value));
        }
        ("argFile", CliValue::String(value), true) => {
            options.arg_sources.push(CliArgumentSource::File(value));
        }
        ("args", CliValue::Json(value), _) => options.args = Some(value),
        ("step", CliValue::Json(value), true) => options.step.push(value),
        ("steps", CliValue::Json(value), _) => options.steps = Some(value),
        ("stepsFile", CliValue::String(value), _) => options.steps_file = Some(value),
        ("event", CliValue::Csv(values), true) => options.event.extend(values),
        ("limit", CliValue::Number(value), _) => options.limit = Some(value),
        ("timeout", CliValue::Number(value), _) => options.timeout = Some(value.max(0) as u64),
        ("duration", CliValue::Number(value), _) => options.duration = Some(value.max(0) as u64),
        ("stepDelay", CliValue::Number(value), _) => options.step_delay = Some(value.max(0) as u64),
        _ => bail!("Invalid value for option {}", spec.key),
    }

    Ok(())
}

fn validate_sequence_steps(steps: Value) -> Result<Vec<Value>> {
    let Value::Array(steps) = steps else {
        bail!("--steps must be a JSON array.");
    };
    if steps.is_empty() {
        bail!("sequence requires at least one step.");
    }

    steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            let Value::Object(map) = step else {
                bail!("sequence step {} must be an object.", index + 1);
            };
            let method_id = map
                .get("methodId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("sequence step {} requires methodId.", index + 1))?;
            Ok(json!({
                "stepId": map.get("stepId").and_then(Value::as_str),
                "methodId": method_id,
                "methodName": map.get("methodName").and_then(Value::as_str),
                "arguments": map.get("arguments").and_then(Value::as_array).cloned().unwrap_or_default(),
            }))
        })
        .collect()
}

fn split_inline_option(arg: &str) -> (&str, Option<&str>) {
    if let Some(index) = arg.find('=') {
        (&arg[..index], Some(&arg[index + 1..]))
    } else {
        (arg, None)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::{collect_cli_arguments, normalize_cli_command, parse_cli_args};

    #[test]
    fn maps_legacy_command_aliases() {
        assert_eq!(normalize_cli_command("exec"), "run");
        assert_eq!(normalize_cli_command("stdio"), "mcp");
    }

    #[test]
    fn does_not_alias_method_discovery_commands() {
        assert_eq!(normalize_cli_command("tools"), "tools");
        assert_eq!(normalize_cli_command("methods"), "methods");
        assert_eq!(normalize_cli_command("find"), "find");
        assert_eq!(normalize_cli_command("tool"), "tool");
        assert_eq!(normalize_cli_command("method"), "method");
        assert_eq!(normalize_cli_command("qa_find_methods"), "qa_find_methods");
        assert_eq!(normalize_cli_command("qa_get_method"), "qa_get_method");
    }

    #[test]
    fn parses_repeated_args_and_full_tools_flag() {
        let argv = [
            "run".to_string(),
            "-c".to_string(),
            "unity-editor-1".to_string(),
            "--tool".to_string(),
            "QaTestSample.Ping()".to_string(),
            "--arg".to_string(),
            "alpha".to_string(),
            "--arg=2".to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();
        assert_eq!(parsed.command, "run");
        assert_eq!(parsed.options.client.as_deref(), Some("unity-editor-1"));
        assert_eq!(
            parsed.options.method.as_deref(),
            Some("QaTestSample.Ping()")
        );
        assert_eq!(parsed.options.arg, ["alpha", "2"]);
    }

    #[test]
    fn collects_repeated_args_as_ordered_json_strings() {
        let argv = [
            "run".to_string(),
            "--client".to_string(),
            "unity-editor-1".to_string(),
            "--tool".to_string(),
            "QaTestSample.Login(System.String,System.String)".to_string(),
            "-a".to_string(),
            "42513".to_string(),
            "-a".to_string(),
            "cn".to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();

        assert_eq!(
            collect_cli_arguments(&parsed.options, 0).unwrap(),
            vec![json!("42513"), json!("cn")]
        );
    }

    #[test]
    fn screenshot_accepts_only_client_and_output() {
        let argv = [
            "screenshot".to_string(),
            "-c".to_string(),
            "unity-editor-1".to_string(),
            "-o".to_string(),
            "target/screens/".to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();

        assert_eq!(parsed.command, "capture-screenshot");
        assert_eq!(parsed.options.client.as_deref(), Some("unity-editor-1"));
        assert_eq!(parsed.options.save_path.as_deref(), Some("target/screens/"));
    }

    #[test]
    fn screenshot_rejects_removed_options() {
        let timeout_error = match parse_cli_args(&[
            "screenshot".to_string(),
            "-c".to_string(),
            "unity-editor-1".to_string(),
            "-o".to_string(),
            "target/screens/".to_string(),
            "--timeout".to_string(),
            "120000".to_string(),
        ]) {
            Ok(_) => panic!("screenshot --timeout should fail"),
            Err(error) => error.to_string(),
        };
        assert!(timeout_error.contains("does not accept --timeout"));

        let tool_error = match parse_cli_args(&[
            "screenshot".to_string(),
            "-c".to_string(),
            "unity-editor-1".to_string(),
            "-o".to_string(),
            "target/screens/".to_string(),
            "--tool".to_string(),
            "CaptureScreenshotToRegister()".to_string(),
        ]) {
            Ok(_) => panic!("screenshot --tool should fail"),
            Err(error) => error.to_string(),
        };
        assert!(tool_error.contains("does not accept --tool"));

        let tag_error = match parse_cli_args(&[
            "screenshot".to_string(),
            "-c".to_string(),
            "unity-editor-1".to_string(),
            "-o".to_string(),
            "target/screens/".to_string(),
            "--tag".to_string(),
            "smoke".to_string(),
        ]) {
            Ok(_) => panic!("screenshot --tag should fail"),
            Err(error) => error.to_string(),
        };
        assert!(tag_error.contains("Unknown option: --tag"));
    }

    #[test]
    fn collects_arg_files_in_command_line_order() {
        let path = std::env::temp_dir().join(format!("qamcp_arg_file_{}.txt", std::process::id()));
        fs::write(&path, "line 1\nline 2").unwrap();
        let path_text = path.to_string_lossy().to_string();
        let argv = [
            "run".to_string(),
            "--client".to_string(),
            "unity-editor-1".to_string(),
            "--tool".to_string(),
            "QaTestSample.Login(System.String,System.String,System.String)".to_string(),
            "-a".to_string(),
            "before".to_string(),
            "--arg-file".to_string(),
            path_text,
            "--arg=after".to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();

        let result = collect_cli_arguments(&parsed.options, 0);
        fs::remove_file(path).ok();

        assert_eq!(
            result.unwrap(),
            vec![json!("before"), json!("line 1\nline 2"), json!("after")]
        );
    }

    #[test]
    fn reports_missing_arg_file_path() {
        let path =
            std::env::temp_dir().join(format!("qamcp_missing_arg_file_{}.txt", std::process::id()));
        let argv = [
            "run".to_string(),
            "--client".to_string(),
            "unity-editor-1".to_string(),
            "--tool".to_string(),
            "QaTestSample.Ping(System.String)".to_string(),
            "--arg-file".to_string(),
            path.to_string_lossy().to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();
        let error = collect_cli_arguments(&parsed.options, 0)
            .unwrap_err()
            .to_string();

        assert!(error.contains("Failed to read argument file"));
    }

    #[test]
    fn rejects_mixed_arg_sources() {
        let argv = [
            "run".to_string(),
            "--client".to_string(),
            "unity-editor-1".to_string(),
            "--tool".to_string(),
            "QaTestSample.Ping(System.String)".to_string(),
            "--args".to_string(),
            "[\"alpha\"]".to_string(),
            "-a".to_string(),
            "beta".to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();
        let error = collect_cli_arguments(&parsed.options, 0)
            .unwrap_err()
            .to_string();

        assert!(error.contains("Use either repeated -a/--arg/--arg-file values"));
    }

    #[test]
    fn rejects_args_mixed_with_arg_file() {
        let argv = [
            "run".to_string(),
            "--client".to_string(),
            "unity-editor-1".to_string(),
            "--tool".to_string(),
            "QaTestSample.Ping(System.String)".to_string(),
            "--args".to_string(),
            "[\"alpha\"]".to_string(),
            "--arg-file".to_string(),
            "record.txt".to_string(),
        ];
        let parsed = parse_cli_args(&argv).unwrap();
        let error = collect_cli_arguments(&parsed.options, 0)
            .unwrap_err()
            .to_string();

        assert!(error.contains("Use either repeated -a/--arg/--arg-file values"));
    }
}
