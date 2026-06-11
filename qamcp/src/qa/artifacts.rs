use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    config::get_qa_server_url,
    constants::{EXECUTE_TIMEOUT_MS, HTTP_TIMEOUT_MS},
    qa::{api::find_methods_data, ws::execute_method_via_websocket},
};

const SCREENSHOT_METHOD_QUERIES: [&str; 2] = ["CaptureScreenshotToRegister", "截图上传到register"];
const DEFAULT_ARTIFACT_FILE_NAME: &str = "artifact.bin";

pub async fn capture_screenshot_data(client_id: String, save_path: String) -> Result<Value> {
    let selected_method = resolve_screenshot_method(&client_id).await?;
    let execution = execute_method_via_websocket(
        client_id,
        selected_method.method_id.clone(),
        selected_method.method_name.clone(),
        Vec::new(),
        EXECUTE_TIMEOUT_MS,
    )
    .await?;
    let artifact = extract_artifact_from_execution(&execution)?;
    let download_url = artifact_download_url(&artifact)?;
    let download = download_artifact_to_path(
        None,
        Some(download_url.as_str()),
        &save_path,
        artifact.get("fileName").and_then(Value::as_str),
    )
    .await?;

    Ok(build_screenshot_summary(&artifact, &download))
}

async fn resolve_screenshot_method(client_id: &str) -> Result<SelectedMethod> {
    for query in SCREENSHOT_METHOD_QUERIES {
        let data = find_methods_data(Some(client_id), query, 20).await?;
        let methods = data
            .get("methods")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(method) = select_method_candidate(&methods, query) {
            return Ok(SelectedMethod {
                method_id: required_method_field(&method, "methodId")?,
                method_name: method
                    .get("methodName")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            });
        }
    }

    bail!(
        "No screenshot QaTest method found for client {client_id}. Search queries: {}.",
        SCREENSHOT_METHOD_QUERIES.join(", ")
    );
}

fn select_method_candidate(methods: &[Value], query: &str) -> Option<Value> {
    let normalized_query = query.trim().to_lowercase();
    let exact_name = methods.iter().find(|method| {
        method
            .get("methodName")
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim().to_lowercase() == normalized_query)
    });
    if let Some(method) = exact_name {
        let mut method = value_object(method.clone());
        method.insert("_qamcpMatchType".to_string(), json!("methodName"));
        return Some(Value::Object(method));
    }

    let exact_short_id = methods.iter().find(|method| {
        method
            .get("methodId")
            .and_then(Value::as_str)
            .is_some_and(|value| short_method_id(value).to_lowercase() == normalized_query)
    });
    if let Some(method) = exact_short_id {
        let mut method = value_object(method.clone());
        method.insert("_qamcpMatchType".to_string(), json!("shortMethodId"));
        return Some(Value::Object(method));
    }

    let exact_id = methods.iter().find(|method| {
        method
            .get("methodId")
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim().to_lowercase() == normalized_query)
    });
    if let Some(method) = exact_id {
        let mut method = value_object(method.clone());
        method.insert("_qamcpMatchType".to_string(), json!("methodId"));
        return Some(Value::Object(method));
    }

    methods.first().cloned().map(|method| {
        let mut method = value_object(method);
        method.insert("_qamcpMatchType".to_string(), json!("searchFirst"));
        Value::Object(method)
    })
}

fn required_method_field(method: &Value, key: &str) -> Result<String> {
    method
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .ok_or_else(|| anyhow!("Selected screenshot method is missing {key}."))
}

fn extract_artifact_from_execution(execution: &Value) -> Result<Value> {
    let result_payload = execution
        .get("result")
        .and_then(|result| result.get("result"))
        .or_else(|| execution.get("result"))
        .unwrap_or(execution);
    let parsed = parse_json_value(result_payload)?;
    find_artifact_metadata(&parsed).ok_or_else(|| {
        anyhow!(
            "Screenshot QaTest result did not contain downloadUrl or artifactId. Result: {}",
            stringify_compact(result_payload)
        )
    })
}

fn parse_json_value(value: &Value) -> Result<Value> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(Value::Null)
            } else {
                serde_json::from_str(trimmed)
                    .with_context(|| "Screenshot QaTest result string is not valid JSON")
            }
        }
        other => Ok(other.clone()),
    }
}

fn find_artifact_metadata(value: &Value) -> Option<Value> {
    match value {
        Value::Object(map)
            if map.get("downloadUrl").and_then(Value::as_str).is_some()
                || map.get("artifactId").and_then(Value::as_str).is_some() =>
        {
            Some(Value::Object(map.clone()))
        }
        Value::Object(map) => {
            for key in ["artifact", "metadata", "data", "result"] {
                if let Some(found) = map.get(key).and_then(find_artifact_metadata) {
                    return Some(found);
                }
            }
            map.values().find_map(find_artifact_metadata)
        }
        Value::Array(values) => values.iter().find_map(find_artifact_metadata),
        Value::String(_) => parse_json_value(value)
            .ok()
            .and_then(|parsed| find_artifact_metadata(&parsed)),
        _ => None,
    }
}

async fn download_artifact_to_path(
    artifact_id: Option<&str>,
    download_url: Option<&str>,
    save_path: &str,
    fallback_file_name: Option<&str>,
) -> Result<Value> {
    let url = match (
        artifact_id.filter(|value| !value.trim().is_empty()),
        download_url.filter(|value| !value.trim().is_empty()),
    ) {
        (Some(_), Some(_)) => bail!("Provide either artifactId or downloadUrl, not both."),
        (Some(artifact_id), None) => artifact_url_from_id(artifact_id.trim())?,
        (None, Some(download_url)) => resolve_register_url(download_url.trim())?,
        (None, None) => bail!("Either artifactId or downloadUrl is required."),
    };
    let client = Client::builder()
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .build()
        .context("Failed to create HTTP client")?;
    let response = client.get(url.clone()).send().await.with_context(|| {
        format!("QA server artifact download timed out or failed after {HTTP_TIMEOUT_MS} ms.")
    })?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .bytes()
        .await
        .context("Failed to read artifact response bytes")?;
    if !status.is_success() {
        let message = parse_error_message(&bytes)
            .unwrap_or_else(|| format!("Artifact download failed with HTTP {}.", status.as_u16()));
        bail!(message);
    }

    let file_name = fallback_file_name
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| content_disposition_file_name(&headers))
        .or_else(|| file_name_from_url_path(&url))
        .unwrap_or_else(|| DEFAULT_ARTIFACT_FILE_NAME.to_string());
    let saved_path = resolve_save_path(save_path, &file_name)?;
    if let Some(parent) = saved_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    fs::write(&saved_path, bytes.as_ref())
        .with_context(|| format!("Failed to write artifact to {}", saved_path.display()))?;

    Ok(json!({
        "savedPath": path_to_string(&saved_path),
        "sizeBytes": bytes.len() as u64,
        "sha256": sha256_hex(bytes.as_ref()),
        "downloadUrl": url.to_string(),
        "fileName": file_name,
        "contentType": headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or(""),
    }))
}

fn build_screenshot_summary(artifact: &Value, download: &Value) -> Value {
    let mut summary = Map::new();
    insert_value(&mut summary, "success", json!(true));
    insert_value(&mut summary, "status", json!("success"));
    insert_optional(
        &mut summary,
        "savedPath",
        first_value([download.get("savedPath")]),
    );
    insert_optional(
        &mut summary,
        "artifactId",
        first_value([
            artifact.get("artifactId"),
            artifact
                .get("artifact")
                .and_then(|value| value.get("artifactId")),
        ]),
    );
    insert_optional(
        &mut summary,
        "fileName",
        first_value([
            download.get("fileName"),
            artifact.get("fileName"),
            artifact
                .get("artifact")
                .and_then(|value| value.get("fileName")),
        ]),
    );
    insert_optional(
        &mut summary,
        "clientId",
        first_value([artifact.get("clientId")]),
    );
    insert_optional(
        &mut summary,
        "clientName",
        first_value([artifact.get("clientName")]),
    );
    insert_optional(&mut summary, "width", first_value([artifact.get("width")]));
    insert_optional(
        &mut summary,
        "height",
        first_value([artifact.get("height")]),
    );
    insert_optional(
        &mut summary,
        "sizeBytes",
        first_value([
            download.get("sizeBytes"),
            artifact.get("sizeBytes"),
            artifact.get("pngSizeBytes"),
            artifact
                .get("artifact")
                .and_then(|value| value.get("sizeBytes")),
        ]),
    );
    insert_optional(
        &mut summary,
        "sha256",
        first_value([
            download.get("sha256"),
            artifact.get("sha256"),
            artifact
                .get("artifact")
                .and_then(|value| value.get("sha256")),
        ]),
    );

    Value::Object(summary)
}

fn insert_value(summary: &mut Map<String, Value>, key: &str, value: Value) {
    summary.insert(key.to_string(), value);
}

fn insert_optional(summary: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.filter(|value| !is_empty_json_value(value)) {
        summary.insert(key.to_string(), value.clone());
    }
}

fn first_value<const N: usize>(values: [Option<&Value>; N]) -> Option<&Value> {
    values
        .into_iter()
        .flatten()
        .find(|value| !is_empty_json_value(value))
}

fn is_empty_json_value(value: &Value) -> bool {
    matches!(value, Value::Null) || value.as_str().is_some_and(|text| text.trim().is_empty())
}

fn artifact_download_url(artifact: &Value) -> Result<Url> {
    if let Some(download_url) = artifact.get("downloadUrl").and_then(Value::as_str) {
        return resolve_register_url(download_url);
    }
    let artifact_id = artifact
        .get("artifactId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Artifact metadata did not contain downloadUrl or artifactId."))?;
    artifact_url_from_id(artifact_id)
}

fn artifact_url_from_id(artifact_id: &str) -> Result<Url> {
    resolve_register_url(&format!(
        "/api/artifacts/{}/download",
        encode_path_segment(artifact_id)
    ))
}

fn resolve_register_url(value: &str) -> Result<Url> {
    if let Ok(url) = Url::parse(value) {
        return Ok(url);
    }
    get_qa_server_url()?
        .join(value.trim_start_matches('/'))
        .context("Failed to build artifact download URL")
}

fn resolve_save_path(save_path: &str, file_name: &str) -> Result<PathBuf> {
    let trimmed = save_path.trim();
    if trimmed.is_empty() {
        bail!("savePath is required.");
    }
    let path = PathBuf::from(trimmed);
    if path.exists() && path.is_dir() {
        return Ok(path.join(sanitize_local_file_name(file_name)));
    }
    if looks_like_directory_path(trimmed) {
        return Ok(path.join(sanitize_local_file_name(file_name)));
    }
    Ok(path)
}

fn looks_like_directory_path(value: &str) -> bool {
    value.ends_with(std::path::MAIN_SEPARATOR) || value.ends_with('/') || value.ends_with('\\')
}

fn sanitize_local_file_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return DEFAULT_ARTIFACT_FILE_NAME.to_string();
    }
    let path = Path::new(trimmed);
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_ARTIFACT_FILE_NAME)
        .to_string()
}

fn content_disposition_file_name(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers
        .get(reqwest::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    for part in value.split(';').map(str::trim) {
        if let Some(file_name) = part.strip_prefix("filename=") {
            return Some(file_name.trim_matches('"').to_string());
        }
    }
    None
}

fn file_name_from_url_path(url: &Url) -> Option<String> {
    let name = url.path_segments()?.next_back()?.trim();
    if name.is_empty() || name == "download" {
        None
    } else {
        Some(name.to_string())
    }
}

fn parse_error_message(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
                .map(str::to_string)
        })
        .or_else(|| Some(text.to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn short_method_id(method_id: &str) -> &str {
    method_id
        .split_once('(')
        .map(|(name, _)| name)
        .unwrap_or(method_id)
        .trim()
}

fn stringify_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string())
}

fn value_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

struct SelectedMethod {
    method_id: String,
    method_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        artifact_download_url, build_screenshot_summary, extract_artifact_from_execution,
        resolve_save_path, select_method_candidate,
    };

    #[test]
    fn parses_artifact_from_string_result() {
        let execution = json!({
            "result": {
                "result": "{\"artifactId\":\"a\",\"fileName\":\"screen.png\"}"
            }
        });

        let artifact = extract_artifact_from_execution(&execution).unwrap();

        assert_eq!(
            artifact.get("artifactId").and_then(|value| value.as_str()),
            Some("a")
        );
        assert_eq!(
            artifact.get("fileName").and_then(|value| value.as_str()),
            Some("screen.png")
        );
    }

    #[test]
    fn parses_nested_artifact_from_object_result() {
        let execution = json!({
            "result": {
                "result": {
                    "ok": true,
                    "artifact": {
                        "downloadUrl": "/api/artifacts/id/download",
                        "fileName": "screen.png"
                    }
                }
            }
        });

        let artifact = extract_artifact_from_execution(&execution).unwrap();

        assert_eq!(
            artifact.get("downloadUrl").and_then(|value| value.as_str()),
            Some("/api/artifacts/id/download")
        );
    }

    #[test]
    fn resolves_save_path_directory_with_trailing_separator() {
        let path = resolve_save_path("target/qamcp-artifacts/", "screen.png").unwrap();

        assert!(path.ends_with("screen.png"));
    }

    #[test]
    fn resolves_save_path_file_when_extension_is_present() {
        let path = resolve_save_path("target/qamcp-artifacts/screen.png", "fallback.png").unwrap();

        assert!(path.ends_with("screen.png"));
    }

    #[test]
    fn selects_exact_short_method_id_before_first_search_result() {
        let methods = vec![
            json!({"methodId": "OtherCaptureScreenshotToRegister()", "methodName": "Other"}),
            json!({"methodId": "CaptureScreenshotToRegister(System.String)", "methodName": "截图"}),
        ];

        let selected = select_method_candidate(&methods, "CaptureScreenshotToRegister").unwrap();

        assert_eq!(
            selected.get("methodId").and_then(|value| value.as_str()),
            Some("CaptureScreenshotToRegister(System.String)")
        );
    }

    #[test]
    fn builds_download_url_from_artifact_id() {
        let artifact = json!({"artifactId": "123e4567-e89b-12d3-a456-426614174000"});

        let url = artifact_download_url(&artifact).unwrap();

        assert_eq!(
            url.path(),
            "/api/artifacts/123e4567-e89b-12d3-a456-426614174000/download"
        );
    }

    #[test]
    fn builds_minimal_screenshot_summary() {
        let artifact = json!({
            "artifact": {
                "artifactId": "artifact-1",
                "fileName": "server.png",
                "sha256": "server-sha",
                "sizeBytes": 100
            },
            "clientId": "client-1",
            "clientName": "client-name",
            "fileName": "screen.png",
            "height": 1080,
            "pngSizeBytes": 2371842,
            "width": 1920
        });
        let download = json!({
            "fileName": "local.png",
            "savedPath": "./img.png",
            "sha256": "download-sha",
            "sizeBytes": 2371842
        });

        let summary = build_screenshot_summary(&artifact, &download);

        assert_eq!(summary.get("success").and_then(Value::as_bool), Some(true));
        assert_eq!(
            summary.get("savedPath").and_then(Value::as_str),
            Some("./img.png")
        );
        assert_eq!(
            summary.get("artifactId").and_then(Value::as_str),
            Some("artifact-1")
        );
        assert_eq!(
            summary.get("fileName").and_then(Value::as_str),
            Some("local.png")
        );
        assert_eq!(
            summary.get("clientId").and_then(Value::as_str),
            Some("client-1")
        );
        assert_eq!(
            summary.get("clientName").and_then(Value::as_str),
            Some("client-name")
        );
        assert_eq!(summary.get("width").and_then(Value::as_u64), Some(1920));
        assert_eq!(summary.get("height").and_then(Value::as_u64), Some(1080));
        assert_eq!(
            summary.get("sizeBytes").and_then(Value::as_u64),
            Some(2371842)
        );
        assert_eq!(
            summary.get("sha256").and_then(Value::as_str),
            Some("download-sha")
        );
        assert!(summary.get("artifact").is_none());
        assert!(summary.get("download").is_none());
        assert!(summary.get("execution").is_none());
        assert!(summary.get("result").is_none());
        assert!(summary.get("selectedMethod").is_none());
    }
}
