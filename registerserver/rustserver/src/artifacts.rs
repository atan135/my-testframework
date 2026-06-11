use std::path::{Path, PathBuf};

use axum::{
    Json,
    body::Bytes,
    extract::{Path as AxumPath, Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::fs;
use uuid::Uuid;

use crate::{
    logging::LogEvent,
    state::SharedState,
    util::{iso_now, uuid},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateArtifactQuery {
    client_id: String,
    kind: String,
    file_name: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactMetadata {
    artifact_id: String,
    kind: String,
    client_id: String,
    file_name: String,
    content_type: String,
    size_bytes: u64,
    sha256: String,
    created_at: String,
    download_url: String,
}

pub(crate) async fn api_create_artifact(
    State(state): State<SharedState>,
    Query(query): Query<CreateArtifactQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client_id = query.client_id.trim().to_string();
    if client_id.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "clientId is required.");
    }

    let kind = query.kind.trim().to_string();
    if kind.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "kind is required.");
    }

    let content_type = match normalized_content_type(&headers) {
        Ok(value) => value,
        Err(message) => return json_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, message),
    };

    if body.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "Artifact body is empty.");
    }

    if body.len() > state.config.artifact_max_bytes {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Artifact body exceeds QA_ARTIFACT_MAX_BYTES ({} bytes).",
                state.config.artifact_max_bytes
            ),
        );
    }

    let artifact_id = uuid();
    let file_name = sanitize_file_name(query.file_name.as_deref())
        .unwrap_or_else(|| fallback_file_name(&kind, content_type));
    let artifact_dir = state.config.artifact_dir.join(&artifact_id);
    let file_path = artifact_dir.join(&file_name);
    let metadata_path = artifact_dir.join("metadata.json");
    let metadata = ArtifactMetadata {
        artifact_id: artifact_id.clone(),
        kind,
        client_id,
        file_name,
        content_type: content_type.to_string(),
        size_bytes: body.len() as u64,
        sha256: sha256_hex(&body),
        created_at: iso_now(),
        download_url: format!("/api/artifacts/{artifact_id}/download"),
    };

    if let Err(error) = fs::create_dir_all(&artifact_dir).await {
        LogEvent::error("artifact_store_failed")
            .field("artifactId", artifact_id)
            .field("error", error.to_string())
            .emit();
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create artifact directory.",
        );
    }

    if let Err(error) = fs::write(&file_path, &body).await {
        LogEvent::error("artifact_store_failed")
            .field("artifactId", metadata.artifact_id.clone())
            .field("error", error.to_string())
            .emit();
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to write artifact.",
        );
    }

    let metadata_json = match serde_json::to_vec(&metadata) {
        Ok(value) => value,
        Err(error) => {
            LogEvent::error("artifact_store_failed")
                .field("artifactId", metadata.artifact_id.clone())
                .field("error", error.to_string())
                .emit();
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to serialize artifact metadata.",
            );
        }
    };

    if let Err(error) = fs::write(&metadata_path, metadata_json).await {
        LogEvent::error("artifact_store_failed")
            .field("artifactId", metadata.artifact_id.clone())
            .field("error", error.to_string())
            .emit();
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to write artifact metadata.",
        );
    }

    LogEvent::new("artifact_stored")
        .field("artifactId", metadata.artifact_id.clone())
        .client_id_str(&metadata.client_id)
        .field("kind", metadata.kind.clone())
        .field("contentType", metadata.content_type.clone())
        .field("sizeBytes", metadata.size_bytes)
        .emit();

    (StatusCode::CREATED, Json(metadata)).into_response()
}

pub(crate) async fn api_get_artifact(
    State(state): State<SharedState>,
    AxumPath(artifact_id): AxumPath<String>,
) -> Response {
    match load_metadata(&state.config.artifact_dir, &artifact_id).await {
        Ok(metadata) => (StatusCode::OK, Json(metadata)).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(crate) async fn api_download_artifact(
    State(state): State<SharedState>,
    AxumPath(artifact_id): AxumPath<String>,
) -> Response {
    let metadata = match load_metadata(&state.config.artifact_dir, &artifact_id).await {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let artifact_dir = match artifact_directory(&state.config.artifact_dir, &artifact_id) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let file_path = artifact_dir.join(&metadata.file_name);
    let bytes = match fs::read(&file_path).await {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json_error(StatusCode::NOT_FOUND, "Artifact file not found.");
        }
        Err(error) => {
            LogEvent::error("artifact_download_failed")
                .field("artifactId", metadata.artifact_id.clone())
                .field("error", error.to_string())
                .emit();
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read artifact.",
            );
        }
    };

    let content_type = match HeaderValue::from_str(&metadata.content_type) {
        Ok(value) => value,
        Err(_) => HeaderValue::from_static("application/octet-stream"),
    };
    let content_disposition = format!("attachment; filename=\"{}\"", metadata.file_name);
    let content_disposition = HeaderValue::from_str(&content_disposition)
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    let mut response_headers = HeaderMap::new();
    response_headers.insert(CONTENT_TYPE, content_type);
    response_headers.insert(CONTENT_DISPOSITION, content_disposition);

    (StatusCode::OK, response_headers, bytes).into_response()
}

async fn load_metadata(
    base_dir: &Path,
    artifact_id: &str,
) -> Result<ArtifactMetadata, ArtifactError> {
    let artifact_dir = artifact_directory(base_dir, artifact_id)?;
    let metadata_path = artifact_dir.join("metadata.json");
    let bytes = fs::read(&metadata_path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ArtifactError::new(StatusCode::NOT_FOUND, "Artifact not found.")
        } else {
            LogEvent::error("artifact_metadata_read_failed")
                .field("artifactId", artifact_id.to_string())
                .field("error", error.to_string())
                .emit();
            ArtifactError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read artifact metadata.",
            )
        }
    })?;

    serde_json::from_slice(&bytes).map_err(|error| {
        LogEvent::error("artifact_metadata_parse_failed")
            .field("artifactId", artifact_id.to_string())
            .field("error", error.to_string())
            .emit();
        ArtifactError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to parse artifact metadata.",
        )
    })
}

fn artifact_directory(base_dir: &Path, artifact_id: &str) -> Result<PathBuf, ArtifactError> {
    if Uuid::parse_str(artifact_id).is_err() {
        return Err(ArtifactError::new(
            StatusCode::BAD_REQUEST,
            "artifactId must be a UUID.",
        ));
    }

    Ok(base_dir.join(artifact_id))
}

fn normalized_content_type(headers: &HeaderMap) -> Result<&'static str, &'static str> {
    let Some(value) = headers.get(CONTENT_TYPE) else {
        return Err("Content-Type is required.");
    };
    let Ok(value) = value.to_str() else {
        return Err("Content-Type is invalid.");
    };
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match media_type.as_str() {
        "image/png" => Ok("image/png"),
        "image/jpeg" => Ok("image/jpeg"),
        "application/octet-stream" => Ok("application/octet-stream"),
        _ => Err(
            "Unsupported Content-Type. Supported types: image/png, image/jpeg, application/octet-stream.",
        ),
    }
}

fn sanitize_file_name(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }

    let mut output = String::with_capacity(value.len().min(160));
    let mut last_was_underscore = false;
    for character in value.chars() {
        let sanitized = if character.is_ascii_alphanumeric()
            || character == '.'
            || character == '-'
            || character == '_'
        {
            character
        } else {
            '_'
        };

        if sanitized == '_' {
            if !last_was_underscore {
                output.push(sanitized);
            }
            last_was_underscore = true;
        } else {
            output.push(sanitized);
            last_was_underscore = false;
        }

        if output.len() >= 160 {
            break;
        }
    }

    let output = output
        .trim_matches(|character| character == '.' || character == '_' || character == '-')
        .to_string();
    if output.is_empty()
        || is_reserved_windows_name(&output)
        || output.eq_ignore_ascii_case("metadata.json")
    {
        None
    } else {
        Some(output)
    }
}

fn fallback_file_name(kind: &str, content_type: &str) -> String {
    let extension = match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "bin",
    };
    let base = sanitize_file_name(Some(kind)).unwrap_or_else(|| "artifact".to_string());
    format!("{base}.{extension}")
}

fn is_reserved_windows_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    matches!(
        stem.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    format!("{hash:x}")
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({
            "error": message.into(),
        })),
    )
        .into_response()
}

struct ArtifactError {
    status: StatusCode,
    message: String,
}

impl ArtifactError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn into_response(self) -> Response {
        json_error(self.status, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::{fallback_file_name, sanitize_file_name};

    #[test]
    fn sanitize_file_name_blocks_path_traversal() {
        assert_eq!(
            sanitize_file_name(Some("..\\..//screen.png")),
            Some("screen.png".to_string())
        );
    }

    #[test]
    fn sanitize_file_name_rejects_metadata_sidecar_name() {
        assert_eq!(sanitize_file_name(Some("metadata.json")), None);
    }

    #[test]
    fn fallback_file_name_uses_kind_and_content_type() {
        assert_eq!(
            fallback_file_name("screenshot", "image/png"),
            "screenshot.png"
        );
    }
}
