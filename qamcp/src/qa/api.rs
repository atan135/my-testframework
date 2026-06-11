use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Map, Value, json};

use crate::{config::get_qa_server_url, constants::HTTP_TIMEOUT_MS};

pub async fn get_health_data() -> Result<Value> {
    let health = request_json("/api/health").await?;
    Ok(json!({
        "serverUrl": get_qa_server_url()?.to_string(),
        "health": health,
    }))
}

pub async fn list_unity_clients_data(include_methods: bool) -> Result<Value> {
    let payload = request_json("/api/unity-clients").await?;
    let clients = payload
        .get("clients")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|client| {
            if include_methods {
                return client;
            }

            let method_count = client
                .get("methods")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let mut summary = value_object(client);
            summary.remove("methods");
            summary.insert("methodCount".to_string(), json!(method_count));
            Value::Object(summary)
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "count": clients.len(),
        "clients": clients,
    }))
}

pub async fn find_methods_data(
    client_id: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Value> {
    if let Some(client_id) = client_id {
        refresh_methods_if_needed(client_id).await?;
    }

    let payload = request_json("/api/unity-clients").await?;
    let normalized_query = query.trim().to_lowercase();
    let mut methods = collect_methods(&payload, client_id)
        .into_iter()
        .filter(|method| normalized_query.is_empty() || method.haystack.contains(&normalized_query))
        .collect::<Vec<_>>();
    let count = methods.len();
    let truncated = count > limit;
    methods.truncate(limit);
    let methods = methods
        .into_iter()
        .map(|method| Value::Object(method.record))
        .collect::<Vec<_>>();

    Ok(json!({
        "query": query,
        "count": count,
        "methods": methods,
        "truncated": truncated,
    }))
}

pub async fn refresh_methods_if_needed(client_id: &str) -> Result<Value> {
    request_json_with_method(
        "POST",
        &format!(
            "/api/unity-clients/{}/refresh-methods-if-needed",
            encode_path_segment(client_id)
        ),
    )
    .await
}

pub async fn get_method_data(client_id: Option<&str>, query: &str) -> Result<Value> {
    let queries = split_method_queries(query);
    if queries.is_empty() {
        bail!("describe requires --query <methodId-or-methodName>.");
    }

    let payload = request_json("/api/unity-clients").await?;
    let methods = collect_methods(&payload, client_id);
    if queries.len() == 1 {
        let method_match = resolve_method_query(&methods, &queries[0])?;
        return Ok(to_single_method_data(&method_match));
    }

    let matches = queries
        .iter()
        .map(|query| {
            resolve_method_query(&methods, query)
                .map(|method_match| to_method_match_data(&method_match))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(json!({
        "query": query,
        "count": matches.len(),
        "methods": matches,
    }))
}

fn split_method_queries(query: &str) -> Vec<String> {
    query
        .split('|')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

struct MethodMatch<'a> {
    query: &'a str,
    match_type: &'static str,
    method: &'a MethodRecord,
}

fn resolve_method_query<'a>(
    methods: &'a [MethodRecord],
    query: &'a str,
) -> Result<MethodMatch<'a>> {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        bail!("describe requires --query <methodId-or-methodName>.");
    }

    let exact_id_matches = matching_methods(methods, |method| {
        method.normalized_method_id == normalized_query
    });
    if let Some(method_match) = resolve_exact_matches(query, "methodId", exact_id_matches)? {
        return Ok(method_match);
    }

    let short_id_matches = matching_methods(methods, |method| {
        method.normalized_short_method_id == normalized_query
    });
    if let Some(method_match) = resolve_exact_matches(query, "shortMethodId", short_id_matches)? {
        return Ok(method_match);
    }

    let name_matches = matching_methods(methods, |method| {
        method.normalized_method_name == normalized_query
    });
    if let Some(method_match) = resolve_exact_matches(query, "methodName", name_matches)? {
        return Ok(method_match);
    }

    bail!(
        "No method exactly matched query: {query}. Use qamcp search first, then pass an exact methodId or methodName to qamcp describe."
    );
}

fn resolve_exact_matches<'a>(
    query: &'a str,
    match_type: &'static str,
    matches: Vec<&'a MethodRecord>,
) -> Result<Option<MethodMatch<'a>>> {
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() == 1 {
        return Ok(Some(MethodMatch {
            query,
            match_type,
            method: matches[0],
        }));
    }

    bail!(
        "Multiple methods exactly matched query: {query}. Candidates: {}. Use --client or a more specific methodId.",
        format_method_candidates(&matches)
    );
}

fn matching_methods<'a>(
    methods: &'a [MethodRecord],
    matches: impl Fn(&MethodRecord) -> bool,
) -> Vec<&'a MethodRecord> {
    methods.iter().filter(|method| matches(method)).collect()
}

fn format_method_candidates(methods: &[&MethodRecord]) -> String {
    let mut candidates = methods
        .iter()
        .take(8)
        .map(|method| {
            let method_id = method
                .record
                .get("methodId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let method_name = method
                .record
                .get("methodName")
                .and_then(Value::as_str)
                .unwrap_or("");
            let client_id = method
                .record
                .get("clientId")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("{method_name} [{method_id}] client={client_id}")
        })
        .collect::<Vec<_>>();

    if methods.len() > candidates.len() {
        candidates.push(format!("... and {} more", methods.len() - candidates.len()));
    }

    candidates.join("; ")
}

pub async fn get_results_data(
    client_id: Option<&str>,
    status: Option<&str>,
    sequence_id: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let payload = request_json("/api/results").await?;
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|result| matches_string(result, "clientId", client_id))
        .filter(|result| matches_string(result, "status", status))
        .filter(|result| matches_string(result, "sequenceId", sequence_id))
        .take(limit)
        .collect::<Vec<_>>();

    Ok(json!({
        "count": results.len(),
        "results": results,
    }))
}

pub async fn request_json(pathname: &str) -> Result<Value> {
    request_json_with_method("GET", pathname).await
}

async fn request_json_with_method(method: &str, pathname: &str) -> Result<Value> {
    let client = Client::builder()
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .build()
        .context("Failed to create HTTP client")?;
    let url = get_qa_server_url()?.join(pathname.trim_start_matches('/'))?;
    let request = match method {
        "POST" => client.post(url),
        _ => client.get(url),
    };
    let response = request
        .header("accept", "application/json")
        .send()
        .await
        .with_context(|| {
            format!("QA server request timed out or failed after {HTTP_TIMEOUT_MS} ms.")
        })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("Failed to read QA server response")?;
    let body = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&text).context("QA server returned invalid JSON")?
    };

    if !status.is_success() {
        let message = body
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("QA server request failed with HTTP {}.", status.as_u16()));
        bail!(message);
    }

    Ok(body)
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

#[derive(Clone)]
struct MethodRecord {
    record: Map<String, Value>,
    normalized_method_id: String,
    normalized_short_method_id: String,
    normalized_method_name: String,
    haystack: String,
}

fn collect_methods(payload: &Value, client_id: Option<&str>) -> Vec<MethodRecord> {
    payload
        .get("clients")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|client| matches_string(client, "clientId", client_id))
        .flat_map(|client| {
            client
                .get("methods")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|method| to_method_record(client, method))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn to_method_record(client: &Value, method: &Value) -> MethodRecord {
    let mut record = Map::new();
    record.insert("clientId".to_string(), get_value(client, "clientId"));
    record.insert("clientName".to_string(), get_value(client, "name"));
    record.insert(
        "clientIpAddress".to_string(),
        string_value(
            client
                .get("ipAddress")
                .or_else(|| client.get("remoteAddress"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
    );
    record.insert("methodId".to_string(), get_value(method, "id"));
    record.insert("methodName".to_string(), get_value(method, "name"));
    record.insert(
        "declaringType".to_string(),
        get_value(method, "declaringType"),
    );
    record.insert("description".to_string(), get_value(method, "description"));
    record.insert("returnType".to_string(), get_value(method, "returnType"));
    record.insert("isStatic".to_string(), get_value(method, "isStatic"));
    record.insert(
        "allowParallelExecution".to_string(),
        get_value(method, "allowParallelExecution"),
    );
    record.insert(
        "parameters".to_string(),
        method
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!([])),
    );

    let method_id = record
        .get("methodId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let normalized_method_id = method_id.to_lowercase();
    let normalized_short_method_id = short_method_id(method_id).to_lowercase();
    let normalized_method_name = record
        .get("methodName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let haystack = [
        record.get("methodId"),
        record.get("methodName"),
        record.get("declaringType"),
        record.get("description"),
        record.get("returnType"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase();

    MethodRecord {
        record,
        normalized_method_id,
        normalized_short_method_id,
        normalized_method_name,
        haystack,
    }
}

fn short_method_id(method_id: &str) -> &str {
    method_id
        .split_once('(')
        .map(|(name, _)| name)
        .unwrap_or(method_id)
        .trim()
}

fn to_single_method_data(method_match: &MethodMatch<'_>) -> Value {
    to_method_match_data(method_match)
}

fn to_method_match_data(method_match: &MethodMatch<'_>) -> Value {
    json!({
        "query": method_match.query,
        "matchType": method_match.match_type,
        "method": method_match.method.record,
    })
}

fn matches_string(value: &Value, key: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| value.get(key).and_then(Value::as_str) == Some(expected))
}

fn get_value(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

fn string_value(value: &str) -> Value {
    Value::String(value.to_string())
}

fn value_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_method_queries_uses_pipe_separator() {
        assert_eq!(
            split_method_queries("Method1| Method2 || Method3 "),
            vec!["Method1", "Method2", "Method3"]
        );
    }

    #[test]
    fn resolve_method_query_matches_multiple_terms_independently() {
        let methods = vec![
            method_record("Ping()", "连通性检查"),
            method_record(
                "OperateListItemAndGetData(System.String)",
                "操作列表项并获取数据",
            ),
        ];

        let ping = resolve_method_query(&methods, "连通性检查").unwrap();
        assert_eq!(ping.match_type, "methodName");
        assert_eq!(
            ping.method.record.get("methodId").and_then(Value::as_str),
            Some("Ping()")
        );

        let operate = resolve_method_query(&methods, "OperateListItemAndGetData").unwrap();
        assert_eq!(operate.match_type, "shortMethodId");
        assert_eq!(
            operate
                .method
                .record
                .get("methodName")
                .and_then(Value::as_str),
            Some("操作列表项并获取数据")
        );
    }

    #[test]
    fn resolve_method_query_prefers_exact_short_method_id() {
        let methods = vec![
            method_record(
                "OperateListItemAndGetData(System.String)",
                "操作列表项并获取数据",
            ),
            method_record("OperateListItem(System.String)", "操作列表项"),
        ];

        let method_match = resolve_method_query(&methods, "OperateListItemAndGetData").unwrap();

        assert_eq!(method_match.match_type, "shortMethodId");
        assert_eq!(
            method_match
                .method
                .record
                .get("methodId")
                .and_then(Value::as_str),
            Some("OperateListItemAndGetData(System.String)")
        );
    }

    #[test]
    fn resolve_method_query_reports_candidates_for_multiple_exact_matches() {
        let methods = vec![
            method_record("Ping(System.String)", "字符串连通性检查"),
            method_record("Ping(System.Int32)", "数字连通性检查"),
        ];

        let error = match resolve_method_query(&methods, "Ping") {
            Ok(_) => panic!("expected multiple matches"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("Multiple methods exactly matched query: Ping."));
        assert!(error.contains("Ping(System.String)"));
        assert!(error.contains("Ping(System.Int32)"));
    }

    #[test]
    fn resolve_method_query_does_not_use_fuzzy_matching() {
        let methods = vec![
            method_record("Ping()", "连通性检查"),
            method_record("PingAdvanced()", "高级连通性检查"),
        ];

        let error = match resolve_method_query(&methods, "连通性") {
            Ok(_) => panic!("expected no exact match"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("No method exactly matched query: 连通性."));
    }

    fn method_record(method_id: &str, method_name: &str) -> MethodRecord {
        let mut record = Map::new();
        record.insert("methodId".to_string(), json!(method_id));
        record.insert("methodName".to_string(), json!(method_name));
        let haystack = format!("{method_id}\n{method_name}").to_lowercase();

        MethodRecord {
            record,
            normalized_method_id: method_id.to_lowercase(),
            normalized_short_method_id: short_method_id(method_id).to_lowercase(),
            normalized_method_name: method_name.to_lowercase(),
            haystack,
        }
    }
}
