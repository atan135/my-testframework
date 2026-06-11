use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};
use url::Url;

use crate::constants::DEFAULT_REGISTER_SERVER_URL;

pub const CONFIG_FILE_NAME: &str = "qamcp.config.json";

pub fn get_qamcp_config_path() -> Result<PathBuf> {
    Ok(get_qamcp_runtime_directory()?.join(CONFIG_FILE_NAME))
}

pub fn read_qamcp_config() -> Result<Value> {
    let config_path = get_qamcp_config_path()?;
    if !config_path.exists() {
        return Ok(json!({
            "configPath": path_to_string(&config_path),
            "exists": false,
            "serverUrl": normalize_qa_server_url(DEFAULT_REGISTER_SERVER_URL)?.to_string(),
        }));
    }

    let mut config = read_raw_config(&config_path)?;
    config.insert(
        "configPath".to_string(),
        Value::String(path_to_string(&config_path)),
    );
    config.insert("exists".to_string(), Value::Bool(true));
    let server_url = config
        .get("serverUrl")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_REGISTER_SERVER_URL);
    config.insert(
        "serverUrl".to_string(),
        Value::String(normalize_qa_server_url(server_url)?.to_string()),
    );
    Ok(Value::Object(config))
}

pub fn save_qa_server_url(server_url: &str) -> Result<Value> {
    let config_path = get_qamcp_config_path()?;
    let mut config = if config_path.exists() {
        read_raw_config(&config_path)?
    } else {
        Map::new()
    };
    let normalized_server_url = normalize_qa_server_url(server_url)?.to_string();
    config.insert(
        "serverUrl".to_string(),
        Value::String(normalized_server_url.clone()),
    );

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
    }

    let file_payload = serde_json::to_string_pretty(&Value::Object(config.clone()))?;
    fs::write(&config_path, format!("{file_payload}\n"))
        .with_context(|| format!("Failed to write qamcp config at {}", config_path.display()))?;

    Ok(json!({
        "configPath": path_to_string(&config_path),
        "exists": true,
        "saved": true,
        "serverUrl": normalized_server_url,
    }))
}

pub fn get_qa_server_url() -> Result<Url> {
    let config = read_qamcp_config()?;
    let server_url = config
        .get("serverUrl")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_REGISTER_SERVER_URL);
    normalize_qa_server_url(server_url)
}

pub fn normalize_qa_server_url(value: &str) -> Result<Url> {
    let raw_value = value.trim();
    if raw_value.is_empty() {
        bail!("serverUrl cannot be empty.");
    }

    let with_protocol = if has_url_scheme(raw_value) {
        raw_value.to_string()
    } else {
        format!("http://{raw_value}")
    };

    let mut url =
        Url::parse(&with_protocol).with_context(|| format!("Invalid serverUrl: {raw_value}"))?;
    match url.scheme() {
        "ws" => url
            .set_scheme("http")
            .map_err(|_| anyhow!("serverUrl must use http, https, ws, or wss."))?,
        "wss" => url
            .set_scheme("https")
            .map_err(|_| anyhow!("serverUrl must use http, https, ws, or wss."))?,
        "http" | "https" => {}
        _ => bail!("serverUrl must use http, https, ws, or wss."),
    }

    url.set_fragment(None);
    url.set_query(None);
    url.set_path("/");
    Ok(url)
}

fn read_raw_config(config_path: &Path) -> Result<Map<String, Value>> {
    let text = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read qamcp config at {}", config_path.display()))?;
    let parsed: Value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to read qamcp config at {}", config_path.display()))?;
    match parsed {
        Value::Object(map) => Ok(map),
        _ => bail!(
            "qamcp config at {} must be a JSON object.",
            config_path.display()
        ),
    }
}

fn get_qamcp_runtime_directory() -> Result<PathBuf> {
    if let Ok(config_dir) = std::env::var("QAMCP_CONFIG_DIR") {
        let trimmed = config_dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let exe_path = std::env::current_exe().context("Failed to resolve current executable path")?;
    exe_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("Failed to resolve qamcp runtime directory"))
}

fn has_url_scheme(value: &str) -> bool {
    let Some(separator_index) = value.find("://") else {
        return false;
    };
    let scheme = &value[..separator_index];
    let mut chars = scheme.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }

    for ch in chars {
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')) {
            return false;
        }
    }
    true
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::normalize_qa_server_url;

    #[test]
    fn normalizes_ws_urls_to_http_roots() {
        let url = normalize_qa_server_url("ws://localhost:3456/ws?role=unity").unwrap();
        assert_eq!(url.to_string(), "http://localhost:3456/");
    }

    #[test]
    fn adds_default_http_scheme() {
        let url = normalize_qa_server_url("localhost:3000").unwrap();
        assert_eq!(url.to_string(), "http://localhost:3000/");
    }
}
