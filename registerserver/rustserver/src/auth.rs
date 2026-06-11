use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::State,
    http::{
        HeaderMap, StatusCode,
        header::{COOKIE, SET_COOKIE},
    },
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::{
    state::SharedState,
    util::{parse_body_or_empty, value_to_opt_string},
};

const WEB_AUTH_COOKIE: &str = "qa_web_console_token";

pub(crate) async fn api_web_auth(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let token_required = state.config.web_console_token.is_some();
    let authenticated = is_web_console_authorized(&state, &headers);
    (
        StatusCode::OK,
        axum::Json(json!({
            "tokenRequired": token_required,
            "authenticated": authenticated,
        })),
    )
}

pub(crate) async fn api_web_login(State(state): State<SharedState>, body: Bytes) -> Response {
    let Some(expected_token) = state.config.web_console_token.as_deref() else {
        return (
            StatusCode::OK,
            axum::Json(json!({
                "tokenRequired": false,
                "authenticated": true,
            })),
        )
            .into_response();
    };

    let body = match parse_body_or_empty(&body) {
        Ok(value) => value,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "error": message })),
            )
                .into_response();
        }
    };
    let token = value_to_opt_string(body.get("token")).unwrap_or_default();
    if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "Invalid web console token." })),
        )
            .into_response();
    }

    let cookie = format!(
        "{WEB_AUTH_COOKIE}={}; HttpOnly; SameSite=Lax; Path=/",
        hex_encode(expected_token.as_bytes())
    );
    (
        StatusCode::OK,
        [(SET_COOKIE, cookie)],
        axum::Json(json!({
            "tokenRequired": true,
            "authenticated": true,
        })),
    )
        .into_response()
}

pub(crate) async fn api_web_logout() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            SET_COOKIE,
            format!("{WEB_AUTH_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"),
        )],
        axum::Json(json!({ "authenticated": false })),
    )
}

pub(crate) fn is_browser_web_socket(query: &HashMap<String, String>) -> bool {
    let role = query.get("role").map(String::as_str).unwrap_or("");
    let controller_type = query
        .get("controllerType")
        .map(String::as_str)
        .unwrap_or("web");
    role == "web" && controller_type != "mcp"
}

pub(crate) fn is_web_console_authorized(state: &SharedState, headers: &HeaderMap) -> bool {
    let Some(expected_token) = state.config.web_console_token.as_deref() else {
        return true;
    };
    let Some(cookie_value) = cookie_value(headers, WEB_AUTH_COOKIE) else {
        return false;
    };
    constant_time_eq(
        cookie_value.as_bytes(),
        hex_encode(expected_token.as_bytes()).as_bytes(),
    )
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for part in value.split(';') {
            let trimmed = part.trim();
            let Some((cookie_name, cookie_value)) = trimmed.split_once('=') else {
                continue;
            };
            if cookie_name == name {
                return Some(cookie_value.to_string());
            }
        }
    }
    None
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff = 0u8;
    for index in 0..left.len() {
        diff |= left[index] ^ right[index];
    }
    diff == 0
}
