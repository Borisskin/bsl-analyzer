use crate::state::SharedState;
use crate::tools::response::structured;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as McpError;
use serde_json::json;
use std::collections::HashMap;
use std::fmt::Write;

/// Static contract for cold-start discovery, mirroring `graph`/`diagnostics` schema so the
/// query tool is self-describing instead of revealing its actions only through an error.
pub fn schema() -> CallToolResult {
    structured(json!({
        "schema_version": "1",
        "actions": ["validate", "execute", "schema"],
        "validate": "syntax-check an SDBL query. Works offline via the local parser; with --onec-url it additionally runs live platform validation.",
        "execute": "run a SELECT query against the live 1C base (requires --onec-url). `limit` caps rows; `parameters` binds named query parameters.",
        "params": {
            "query": "the SDBL text (required for validate and execute)",
            "limit": "max rows for execute (optional)",
            "parameters": "object of name → value bindings for execute (optional)"
        },
        "prerequisites": "validate needs nothing for offline syntax checks; execute (and live validation) need --onec-url / --onec-user / --onec-password"
    }))
}

pub async fn validate_query(
    state: &SharedState,
    query: &str,
    connection: Option<&str>,
) -> Result<CallToolResult, McpError> {
    if query.trim().is_empty() {
        return Err(McpError::invalid_params("Пустой запрос", None));
    }

    if connection.is_some() {
        let selected =
            state.onec_connection(connection).map_err(|e| McpError::invalid_params(e, None))?;
        return validate_query_remote(selected.client(), query).await;
    }
    if let Ok(selected) = state.onec_connection(None) {
        return validate_query_remote(selected.client(), query).await;
    }

    validate_query_local(query)
}

async fn validate_query_remote(
    client: &onec_client::Client,
    query: &str,
) -> Result<CallToolResult, McpError> {
    let request = onec_client::ValidateQueryRequest { query: query.to_string() };

    let result = client.validate_query(&request).await.map_err(|e| {
        McpError::internal_error(format!("Ошибка проверки запроса в 1С: {e}"), None)
    })?;

    if result.valid {
        Ok(CallToolResult::success(vec![Content::text("✓ Запрос синтаксически корректен")]))
    } else {
        let mut out = "✗ Ошибки в запросе:\n\n".to_string();
        for err in &result.errors {
            let _ = writeln!(out, "- {err}");
        }
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }
}

fn validate_query_local(query: &str) -> Result<CallToolResult, McpError> {
    let parse = parser::parse_sdbl(query);
    let root = parse.syntax_node();

    let error_nodes: Vec<_> = root
        .descendants()
        .filter(|node| {
            matches!(node.kind(), syntax::SyntaxKind::ERROR | syntax::SyntaxKind::SDBL_ERROR)
        })
        .collect();

    if error_nodes.is_empty() {
        Ok(CallToolResult::success(vec![Content::text("✓ Запрос синтаксически корректен")]))
    } else {
        // The parser can emit overlapping ERROR/SDBL_ERROR nodes at the same offset (e.g. a
        // trailing `ГДЕ` yields two "unexpected end" nodes at the same position), which rendered
        // the identical diagnostic twice. Collapse byte-identical lines, preserving first order,
        // so the count and listing reflect distinct errors.
        let mut seen = std::collections::HashSet::new();
        let mut lines: Vec<String> = Vec::new();
        for node in &error_nodes {
            let range = node.text_range();
            let start = u32::from(range.start()) as usize;
            let end = u32::from(range.end()) as usize;
            let fragment = query.get(start..end).unwrap_or("…");
            let line = if fragment.trim().is_empty() {
                format!("- [{start}] неожиданный конец выражения")
            } else {
                format!("- [{start}..{end}] неожиданный фрагмент: `{fragment}`")
            };
            if seen.insert(line.clone()) {
                lines.push(line);
            }
        }

        let mut out = format!("✗ Найдено ошибок: {}\n\n", lines.len());
        for line in &lines {
            let _ = writeln!(out, "{line}");
        }
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }
}

const DEFAULT_QUERY_LIMIT: u32 = 100;
const MAX_QUERY_LIMIT: u32 = 1000;

pub async fn execute_query(
    state: &SharedState,
    query: &str,
    limit: Option<u32>,
    parameters: Option<HashMap<String, serde_json::Value>>,
    connection: Option<&str>,
) -> Result<CallToolResult, McpError> {
    let selected =
        state.onec_connection(connection).map_err(|e| McpError::invalid_params(e, None))?;

    if query.trim().is_empty() {
        return Err(McpError::invalid_params("Пустой запрос", None));
    }

    let prefix = query.trim();
    let upper_start: String = prefix.chars().take(30).collect::<String>().to_uppercase();
    if !upper_start.starts_with("ВЫБРАТЬ") && !upper_start.starts_with("SELECT") {
        return Err(McpError::invalid_params("Только SELECT/ВЫБРАТЬ запросы разрешены", None));
    }

    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT);

    let request = onec_client::QueryRequest {
        query: query.to_string(),
        limit,
        parameters: parameters.unwrap_or_default(),
    };

    let result = selected.client().execute_query(&request).await.map_err(|e| {
        McpError::internal_error(format!("Ошибка выполнения запроса в 1С: {e}"), None)
    })?;

    Ok(CallToolResult::success(vec![Content::text(format_query_result(&result))]))
}

#[cfg(test)]
fn test_shared_state() -> crate::SharedState {
    crate::SharedState::shared()
}

fn format_query_result(result: &onec_client::QueryResult) -> String {
    if result.columns.is_empty() {
        return "Запрос выполнен, результат пуст.".to_string();
    }

    let mut out = format!("## Результат запроса ({} записей", result.total);
    if result.truncated {
        out.push_str(", результат усечён");
    }
    out.push_str(")\n\n");

    let _ = write!(out, "|");
    for col in &result.columns {
        let _ = write!(out, " {col} |");
    }
    out.push('\n');

    let _ = write!(out, "|");
    for _ in &result.columns {
        let _ = write!(out, "-----|");
    }
    out.push('\n');

    for row in &result.rows {
        let _ = write!(out, "|");
        for val in row {
            let s = match val {
                serde_json::Value::Null => "—".to_string(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let _ = write!(out, " {s} |");
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_text(result: &CallToolResult) -> &str {
        result.content[0].raw.as_text().expect("expected text content").text.as_str()
    }

    #[test]
    fn schema_advertises_every_action() {
        let result = schema();
        let body = result.structured_content.expect("schema is structured");
        let actions = body["actions"].as_array().expect("actions array");
        for action in ["validate", "execute", "schema"] {
            assert!(
                actions.iter().any(|a| a == action),
                "schema must advertise `{action}`: {body}",
            );
        }
    }

    #[test]
    fn test_validate_query_valid() {
        let _state = test_shared_state();
        let result = validate_query_local("ВЫБРАТЬ 1").unwrap();
        let text = extract_text(&result);
        assert!(text.contains("✓"), "valid query should pass");
    }

    #[test]
    fn test_validate_query_garbage_input() {
        let result = validate_query_local("}{}{}{").unwrap();
        let text = extract_text(&result);
        assert!(text.contains("✗"), "garbage input should produce errors");
    }

    #[test]
    fn test_validate_query_not_a_query() {
        let result = validate_query_local("это вообще не запрос").unwrap();
        let text = extract_text(&result);
        assert!(text.contains("✗"), "arbitrary text should produce errors");
    }

    #[test]
    fn test_validate_query_incomplete_where() {
        let result =
            validate_query_local("ВЫБРАТЬ Наименование ИЗ Справочник.Номенклатура ГДЕ").unwrap();
        let text = extract_text(&result);
        assert!(text.contains("✗"), "incomplete WHERE should produce errors");
    }

    #[test]
    fn validate_does_not_report_the_same_error_twice() {
        // `ВЫБРАТЬ ИЗ ГДЕ` made the parser emit overlapping error nodes at the same offset,
        // which previously listed the identical diagnostic twice. Each rendered line must be
        // unique, and the reported count must match the listed lines.
        let result = validate_query_local("ВЫБРАТЬ ИЗ ГДЕ").unwrap();
        let text = extract_text(&result);
        let error_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("- [")).collect();
        let mut unique = error_lines.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(error_lines.len(), unique.len(), "duplicate error line(s): {text}");
        assert!(
            text.contains(&format!("Найдено ошибок: {}", error_lines.len())),
            "reported count must match listed lines: {text}",
        );
    }

    #[test]
    fn test_validate_query_empty() {
        let state = test_shared_state();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(validate_query(&state, "", None));
        assert!(result.is_err(), "empty query should fail");
    }

    #[test]
    fn test_validate_query_whitespace() {
        let state = test_shared_state();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(validate_query(&state, "   ", None));
        assert!(result.is_err(), "whitespace-only query should fail");
    }

    #[test]
    fn test_validate_query_select_with_fields() {
        let result =
            validate_query_local("ВЫБРАТЬ Ссылка, Наименование ИЗ Справочник.Номенклатура")
                .unwrap();
        let text = extract_text(&result);
        assert!(text.contains("✓"), "select with fields should pass");
    }

    #[test]
    fn test_validate_query_english() {
        let result = validate_query_local("SELECT 1").unwrap();
        let text = extract_text(&result);
        assert!(text.contains("✓"), "English SELECT should pass");
    }
}
