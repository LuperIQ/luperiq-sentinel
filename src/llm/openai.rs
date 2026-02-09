use crate::net::http::HttpClient;
use crate::net::json::{self, JsonValue, json_obj, json_arr};
use crate::llm::provider::{
    ContentBlock, LlmError, LlmProvider, LlmResponse, Message, Role, StopReason, ToolDef,
};

// ── OpenAI-compatible client ────────────────────────────────────────────────
//
// Works with OpenAI, Ollama, vLLM, LM Studio, and other OpenAI-compatible APIs.

pub struct OpenAiClient {
    http: HttpClient,
    api_key: String,
    model: String,
    max_tokens: u32,
    base_url: String,
}

impl OpenAiClient {
    pub fn new(
        http: HttpClient,
        api_key: String,
        model: String,
        max_tokens: u32,
        base_url: String,
    ) -> Self {
        // Strip trailing slash
        let base_url = base_url.trim_end_matches('/').to_string();
        OpenAiClient {
            http,
            api_key,
            model,
            max_tokens,
            base_url,
        }
    }

    fn build_request_body(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> JsonValue {
        let mut body = json_obj()
            .field_str("model", &self.model)
            .field_i64("max_tokens", self.max_tokens as i64);

        // Messages
        let mut msgs = json_arr();

        // System message as first message in OpenAI format
        if let Some(sys) = system {
            msgs = msgs.push(
                json_obj()
                    .field_str("role", "system")
                    .field_str("content", sys)
                    .build(),
            );
        }

        for msg in messages {
            msgs = msgs.push(message_to_openai_json(msg));
        }
        body = body.field("messages", msgs.build());

        // Tools (OpenAI function calling format)
        if !tools.is_empty() {
            let mut tool_arr = json_arr();
            for t in tools {
                tool_arr = tool_arr.push(
                    json_obj()
                        .field_str("type", "function")
                        .field(
                            "function",
                            json_obj()
                                .field_str("name", &t.name)
                                .field_str("description", &t.description)
                                .field("parameters", t.input_schema.clone())
                                .build(),
                        )
                        .build(),
                );
            }
            body = body.field("tools", tool_arr.build());
        }

        body.build()
    }
}

impl LlmProvider for OpenAiClient {
    fn send(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse, LlmError> {
        let body = self.build_request_body(system, messages, tools);
        let body_str = body.to_json_string();

        let url = format!("{}/chat/completions", self.base_url);
        let auth_value = format!("Bearer {}", self.api_key);
        let headers = [("Authorization", auth_value.as_str())];

        let resp = self.http.post_json(&url, &body_str, &headers)?;

        if resp.status == 429 {
            let retry_after = resp
                .headers
                .iter()
                .find(|(k, _)| k == "retry-after")
                .and_then(|(_, v)| v.parse::<u64>().ok());
            return Err(LlmError::RateLimit { retry_after });
        }

        let body_str = resp.body_string().map_err(|e| LlmError::Http(e))?;
        let json_val =
            json::parse(&body_str).map_err(|e| LlmError::Json(e.to_string()))?;

        if resp.status != 200 {
            let msg = json_val
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(LlmError::Api {
                status: resp.status,
                message: msg.to_string(),
            });
        }

        parse_openai_response(&json_val)
    }
}

// ── JSON serialization (Sentinel → OpenAI format) ───────────────────────────

fn message_to_openai_json(msg: &Message) -> JsonValue {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };

    // Check if this message contains tool results (user role with ToolResult blocks)
    let has_tool_results = msg.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }));
    if has_tool_results {
        // OpenAI expects separate "tool" role messages for each tool result
        // but we need to return a single JSON value, so we return the first one
        // The main loop handles multiple tool results by sending them separately
        for block in &msg.content {
            if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                return json_obj()
                    .field_str("role", "tool")
                    .field_str("tool_call_id", tool_use_id)
                    .field_str("content", content)
                    .build();
            }
        }
    }

    // Check if assistant message has tool calls
    let has_tool_use = msg.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    if has_tool_use {
        let mut text_parts = Vec::new();
        let mut tool_calls = json_arr();
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => text_parts.push(text.as_str()),
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls = tool_calls.push(
                        json_obj()
                            .field_str("id", id)
                            .field_str("type", "function")
                            .field(
                                "function",
                                json_obj()
                                    .field_str("name", name)
                                    .field_str("arguments", &input.to_json_string())
                                    .build(),
                            )
                            .build(),
                    );
                }
                _ => {}
            }
        }
        let mut obj = json_obj()
            .field_str("role", "assistant")
            .field("tool_calls", tool_calls.build());
        if !text_parts.is_empty() {
            obj = obj.field_str("content", &text_parts.join("\n"));
        }
        return obj.build();
    }

    // Simple text message
    let mut text_parts = Vec::new();
    for block in &msg.content {
        if let ContentBlock::Text { text } = block {
            text_parts.push(text.as_str());
        }
    }

    json_obj()
        .field_str("role", role)
        .field_str("content", &text_parts.join("\n"))
        .build()
}

// ── Response parsing (OpenAI → Sentinel format) ─────────────────────────────

fn parse_openai_response(json: &JsonValue) -> Result<LlmResponse, LlmError> {
    let choices = json
        .get("choices")
        .and_then(|v| v.as_array())
        .ok_or_else(|| LlmError::Json("missing 'choices' array".into()))?;

    if choices.is_empty() {
        return Err(LlmError::Json("empty choices array".into()));
    }

    let choice = &choices[0];
    let message = choice
        .get("message")
        .ok_or_else(|| LlmError::Json("missing 'message' in choice".into()))?;

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("stop");

    let stop_reason = match finish_reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    };

    let mut content = Vec::new();

    // Text content
    if let Some(text) = message.get("content").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
    }

    // Tool calls
    if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let function = tc.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args_str = function
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let input = json::parse(args_str).unwrap_or(JsonValue::Null);

            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    // Usage
    let usage = json.get("usage");
    let usage_input = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let usage_output = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Ok(LlmResponse {
        stop_reason,
        content,
        usage_input,
        usage_output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_openai_text_response() {
        let json_str = r#"{
            "id": "chatcmpl-123",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        }"#;
        let json = json::parse(json_str).unwrap();
        let resp = parse_openai_response(&json).unwrap();

        assert!(matches!(resp.stop_reason, StopReason::EndTurn));
        assert_eq!(resp.content.len(), 1);
        match &resp.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello!"),
            _ => panic!("expected text"),
        }
        assert_eq!(resp.usage_input, 10);
        assert_eq!(resp.usage_output, 5);
    }

    #[test]
    fn test_parse_openai_tool_call() {
        let json_str = r#"{
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"/tmp/test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10}
        }"#;
        let json = json::parse(json_str).unwrap();
        let resp = parse_openai_response(&json).unwrap();

        assert!(matches!(resp.stop_reason, StopReason::ToolUse));
        assert_eq!(resp.content.len(), 1);
        match &resp.content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read_file");
                assert_eq!(input.get("path").unwrap().as_str().unwrap(), "/tmp/test");
            }
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn test_parse_openai_max_tokens() {
        let json_str = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": "truncated..."},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 4096}
        }"#;
        let json = json::parse(json_str).unwrap();
        let resp = parse_openai_response(&json).unwrap();
        assert!(matches!(resp.stop_reason, StopReason::MaxTokens));
    }

    #[test]
    fn test_message_to_openai_simple() {
        let msg = Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: "Hello".into() }],
        };
        let json = message_to_openai_json(&msg);
        assert_eq!(json.get("role").unwrap().as_str().unwrap(), "user");
        assert_eq!(json.get("content").unwrap().as_str().unwrap(), "Hello");
    }

    #[test]
    fn test_message_to_openai_tool_result() {
        let msg = Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_123".into(),
                content: "file data".into(),
                is_error: false,
            }],
        };
        let json = message_to_openai_json(&msg);
        assert_eq!(json.get("role").unwrap().as_str().unwrap(), "tool");
        assert_eq!(json.get("tool_call_id").unwrap().as_str().unwrap(), "call_123");
        assert_eq!(json.get("content").unwrap().as_str().unwrap(), "file data");
    }
}
