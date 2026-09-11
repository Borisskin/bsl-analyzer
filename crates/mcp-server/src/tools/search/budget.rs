//! Enforce the final wire budget after mandatory indexing telemetry is attached.
use rmcp::{
    model::{CallToolResult, ContentBlock},
    ErrorData as McpError,
};
use serde_json::{json, Value};

fn bytes(response: &CallToolResult) -> usize {
    response
        .content
        .iter()
        .map(|content| match content {
            ContentBlock::Text(text) => text.text.len(),
            _ => 0,
        })
        .sum::<usize>()
        + response
            .structured_content
            .as_ref()
            .map_or(0, |body| serde_json::to_vec(body).expect("JSON serializes").len())
}

fn mirror(response: &mut CallToolResult) {
    if let Some(body) = &response.structured_content {
        response.content =
            vec![ContentBlock::text(serde_json::to_string(body).expect("JSON serializes"))];
    }
}

fn trim(body: &mut Value, count: usize) {
    body["hits"].as_array_mut().expect("hit envelope").truncate(count);
    body["shown"] = json!(count);
    body["budget_exhausted"] = json!(true);
    if let Some(completeness) = body.pointer_mut("/freshness/completeness") {
        completeness["status"] = json!("partial");
        if let Some(reasons) = completeness["reasons"].as_array_mut() {
            if !reasons.iter().any(|reason| reason["code"] == "output_budget") {
                reasons.push(json!({"code":"output_budget", "detail":"hits trimmed to fit max_output_tokens"}));
            }
        }
    }
}

pub(crate) fn finalize_indexed_response(
    mut response: CallToolResult,
    max_output_tokens: usize,
) -> Result<CallToolResult, McpError> {
    let ceiling = max_output_tokens.saturating_mul(4);
    if bytes(&response) <= ceiling {
        return Ok(response);
    }
    // The JSON mirror retains all semantic fields when the human rendering is too large.
    mirror(&mut response);
    if bytes(&response) <= ceiling {
        return Ok(response);
    }
    let count = response
        .structured_content
        .as_ref()
        .and_then(|body| body.get("hits"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut floor = response.clone();
    if count > 0 {
        trim(floor.structured_content.as_mut().expect("hit envelope"), 0);
        mirror(&mut floor);
    }
    let minimum_bytes = bytes(&floor);
    if minimum_bytes > ceiling {
        return Err(McpError::invalid_params(
            "budget_too_small",
            Some(json!({
                "reason":"budget_too_small", "minimum_output_tokens":minimum_bytes.div_ceil(4)
            })),
        ));
    }
    for remaining in (0..count).rev() {
        trim(response.structured_content.as_mut().expect("hit envelope"), remaining);
        mirror(&mut response);
        if bytes(&response) <= ceiling {
            return Ok(response);
        }
    }
    Ok(floor)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(hits: Vec<Value>) -> CallToolResult {
        let body = json!({"hits":hits,"shown":hits.len(),"total":hits.len(),
            "degraded":"provider timeout", "freshness":{"completeness":{"status":"partial",
            "reasons":[{"code":"modality_degraded","detail":"provider timeout"}]}},
            "indexing":{"schema_version":"1","targets":[{"kind":"semantic","state":"ready",
            "phase":null,"progress":null,"pass_id":null,"reason_code":null}]}});
        crate::tools::response::structured_with_text("Пример ".repeat(100), body)
    }
    #[test]
    fn indexing_response_budget() {
        let full = fixture(vec![json!({"text":"данные".repeat(200)}), json!({"text":"end"})]);
        let full_budget = bytes(&full).div_ceil(4);
        let kept = finalize_indexed_response(full.clone(), full_budget).unwrap();
        assert_eq!(kept.content, full.content);
        let mut floor = full.clone();
        trim(floor.structured_content.as_mut().unwrap(), 0);
        mirror(&mut floor);
        let minimum = bytes(&floor).div_ceil(4);
        let limited = finalize_indexed_response(full.clone(), minimum).unwrap();
        assert!(bytes(&limited) <= 4 * minimum);
        let body = limited.structured_content.unwrap();
        assert_eq!(body["shown"], 0);
        assert!(body.get("indexing").is_some());
        assert_eq!(body["degraded"], "provider timeout");
        assert_eq!(body["freshness"]["completeness"]["reasons"].as_array().unwrap().len(), 2);
        let error = finalize_indexed_response(full, minimum - 1).unwrap_err();
        assert_eq!(error.message, "budget_too_small");
        assert_eq!(error.data.unwrap()["minimum_output_tokens"], minimum);
        let mut loading = fixture(vec![]);
        let body = loading.structured_content.as_mut().unwrap().as_object_mut().unwrap();
        body.remove("hits");
        body.remove("shown");
        body.remove("total");
        body.insert("status".to_owned(), json!("loading"));
        for mut empty in [fixture(vec![]), loading] {
            mirror(&mut empty);
            let exact = bytes(&empty).div_ceil(4);
            assert!(finalize_indexed_response(empty.clone(), exact).is_ok());
            assert!(finalize_indexed_response(empty, exact - 1).is_err());
        }
    }
}
