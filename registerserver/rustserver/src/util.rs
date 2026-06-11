use std::env;

use axum::body::Bytes;
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) fn normalize_arguments(raw: &Value) -> Vec<String> {
    raw.as_array()
        .map(|arguments| arguments.iter().map(value_to_string_value).collect())
        .unwrap_or_default()
}

pub(crate) fn normalize_delay_ms(raw: Option<&Value>, max_value: u64) -> u64 {
    let delay = match raw {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(0.0),
        Some(Value::String(value)) => value.parse::<f64>().unwrap_or(0.0),
        Some(Value::Bool(value)) if *value => 1.0,
        _ => 0.0,
    };

    if !delay.is_finite() {
        return 0;
    }
    delay.floor().clamp(0.0, max_value as f64) as u64
}

pub(crate) fn parse_body_or_empty(body: &Bytes) -> Result<Value, String> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(body).map_err(|_| "Invalid JSON body.".to_string())
}

pub(crate) fn parse_message(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}

pub(crate) fn parse_message_bytes(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(bytes).ok()
}

pub(crate) fn value_to_opt_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        other => Some(other.to_string()),
    }
}

pub(crate) fn value_to_string(value: Option<&Value>) -> String {
    value_to_opt_string(value).unwrap_or_default()
}

pub(crate) fn value_to_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn value_to_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|value| value as u64)),
        Value::String(value) => value.parse::<u64>().ok(),
        Value::Bool(value) => Some(u64::from(*value)),
        _ => None,
    }
}

pub(crate) fn merge_json_object(target: &mut Value, source: Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
}

pub(crate) fn iso_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn uuid() -> String {
    Uuid::new_v4().to_string()
}

pub(crate) fn env_positive_u64(name: &str, fallback: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

pub(crate) fn env_positive_u32(name: &str, fallback: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

pub(crate) fn env_positive_usize(name: &str, fallback: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

pub(crate) fn env_positive_u16(name: &str, fallback: u16) -> u16 {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

pub(crate) fn env_bool(name: &str, fallback: bool) -> bool {
    env::var(name)
        .ok()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "enabled" => true,
            "0" | "false" | "no" | "off" | "disabled" => false,
            _ => fallback,
        })
        .unwrap_or(fallback)
}
