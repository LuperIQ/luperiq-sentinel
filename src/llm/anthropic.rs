use crate::net::http::HttpClient;
use crate::net::json::{self, JsonValue, json_obj, json_arr};
use crate::net::sse;
use crate::llm::provider::{
    ContentBlock, LlmError, LlmProvider, LlmResponse, Message, Role, StopReason, ToolDef,
};

// ── Client ──────────────────────────────────────────────────────────────────

pub struct AnthropicClient {
    http: HttpClient,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    pub fn new(
        http: HttpClient,
        api_key: String,
        model: String,
        max_tokens: u32,
    ) -> Self {
        AnthropicClient {
            http,
            api_key,
            model,
            max_tokens,
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
            .field_i64("max_tokens", self.max_tokens as i64)
            .field_bool("stream", false);

        if let Some(sys) = system {
            body = body.field_str("system", sys);
        }

        // Messages
        let mut msgs = json_arr();
        for msg in messages {
            msgs = msgs.push(message_to_json(msg));
        }
        body = body.field("messages", msgs.build());

        // Tools
        if !tools.is_empty() {
            let mut tool_arr = json_arr();
            for t in tools {
                tool_arr = tool_arr.push(tool_def_to_json(t));
            }
            body = body.field("tools", tool_arr.build());
        }

        body.build()
    }
}

impl LlmProvider for AnthropicClient {
    fn send(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse, LlmError> {
        let body = self.build_request_body(system, messages, tools);
        let body_str = body.to_json_string();

        let headers = [
            ("X-Api-Key", self.api_key.as_str()),
            ("anthropic-version", "2023-06-01"),
        ];

        let resp = self
            .http
            .post_json(
                "https://api.anthropic.com/v1/messages",
                &body_str,
                &headers,
            )?;

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

        parse_api_response(&json_val)
    }

    fn send_streaming(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
        on_text: &mut dyn FnMut(&str),
    ) -> Result<LlmResponse, LlmError> {
        let mut body = self.build_request_body(system, messages, tools);
        // Override stream to true
        if let JsonValue::Object(ref mut pairs) = body {
            for (k, v) in pairs.iter_mut() {
                if k == "stream" {
                    *v = JsonValue::Bool(true);
                    break;
                }
            }
        }
        let body_str = body.to_json_string();

        let headers = [
            ("X-Api-Key", self.api_key.as_str()),
            ("anthropic-version", "2023-06-01"),
        ];

        let mut stream_resp = self
            .http
            .post_json_streaming(
                "https://api.anthropic.com/v1/messages",
                &body_str,
                &headers,
            )?;

        if stream_resp.status == 429 {
            let retry_after = stream_resp
                .headers
                .iter()
                .find(|(k, _)| k == "retry-after")
                .and_then(|(_, v)| v.parse::<u64>().ok());
            return Err(LlmError::RateLimit { retry_after });
        }

        if stream_resp.status != 200 {
            // Read the error body
            let mut error_data = String::new();
            for _ in 0..100 {
                let line = stream_resp.read_line().map_err(LlmError::Http)?;
                if line.is_empty() { break; }
                error_data.push_str(&line);
            }
            return Err(LlmError::Api {
                status: stream_resp.status,
                message: error_data,
            });
        }

        // Parse SSE events and accumulate the response
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut stop_reason = StopReason::Other("incomplete".into());
        let mut usage_input: i64 = 0;
        let mut usage_output: i64 = 0;

        // Accumulator for the current content block
        let mut current_text = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_json = String::new();
        let mut current_block_type = String::new();

        loop {
            let event = match sse::read_event(&mut stream_resp) {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => return Err(LlmError::Http(e)),
            };

            let data = &event.data;

            match event.event_type.as_str() {
                "message_start" => {
                    // Extract usage from initial message
                    if let Ok(json) = json::parse(data) {
                        if let Some(msg) = json.get("message") {
                            usage_input = msg
                                .get("usage")
                                .and_then(|u| u.get("input_tokens"))
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                        }
                    }
                }
                "content_block_start" => {
                    if let Ok(json) = json::parse(data) {
                        if let Some(block) = json.get("content_block") {
                            current_block_type = block
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            if current_block_type == "tool_use" {
                                current_tool_id = block
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                current_tool_name = block
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                current_tool_json.clear();
                            } else {
                                current_text.clear();
                            }
                        }
                    }
                }
                "content_block_delta" => {
                    if let Ok(json) = json::parse(data) {
                        if let Some(delta) = json.get("delta") {
                            let delta_type = delta
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            if delta_type == "text_delta" {
                                if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                    current_text.push_str(text);
                                    on_text(text);
                                }
                            } else if delta_type == "input_json_delta" {
                                if let Some(json_part) =
                                    delta.get("partial_json").and_then(|v| v.as_str())
                                {
                                    current_tool_json.push_str(json_part);
                                }
                            }
                        }
                    }
                }
                "content_block_stop" => {
                    if current_block_type == "text" {
                        content_blocks.push(ContentBlock::Text {
                            text: current_text.clone(),
                        });
                        current_text.clear();
                    } else if current_block_type == "tool_use" {
                        let input = json::parse(&current_tool_json).unwrap_or(JsonValue::Null);
                        content_blocks.push(ContentBlock::ToolUse {
                            id: current_tool_id.clone(),
                            name: current_tool_name.clone(),
                            input,
                        });
                        current_tool_json.clear();
                    }
                    current_block_type.clear();
                }
                "message_delta" => {
                    if let Ok(json) = json::parse(data) {
                        if let Some(delta) = json.get("delta") {
                            stop_reason =
                                match delta.get("stop_reason").and_then(|v| v.as_str()) {
                                    Some("end_turn") => StopReason::EndTurn,
                                    Some("tool_use") => StopReason::ToolUse,
                                    Some("max_tokens") => StopReason::MaxTokens,
                                    Some(other) => StopReason::Other(other.to_string()),
                                    None => StopReason::Other("missing".into()),
                                };
                        }
                        usage_output = json
                            .get("usage")
                            .and_then(|u| u.get("output_tokens"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                    }
                }
                "message_stop" => {
                    break;
                }
                _ => {
                    // ping, error, etc. — skip
                }
            }
        }

        Ok(LlmResponse {
            stop_reason,
            content: content_blocks,
            usage_input,
            usage_output,
        })
    }
}

// ── JSON serialization helpers ──────────────────────────────────────────────

fn tool_def_to_json(def: &ToolDef) -> JsonValue {
    json_obj()
        .field_str("name", &def.name)
        .field_str("description", &def.description)
        .field("input_schema", def.input_schema.clone())
        .build()
}

fn message_to_json(msg: &Message) -> JsonValue {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };

    let mut content_arr = json_arr();
    for block in &msg.content {
        content_arr = content_arr.push(content_block_to_json(block));
    }

    json_obj()
        .field_str("role", role)
        .field("content", content_arr.build())
        .build()
}

fn content_block_to_json(block: &ContentBlock) -> JsonValue {
    match block {
        ContentBlock::Text { text } => json_obj()
            .field_str("type", "text")
            .field_str("text", text)
            .build(),
        ContentBlock::ToolUse { id, name, input } => json_obj()
            .field_str("type", "tool_use")
            .field_str("id", id)
            .field_str("name", name)
            .field("input", input.clone())
            .build(),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let mut b = json_obj()
                .field_str("type", "tool_result")
                .field_str("tool_use_id", tool_use_id)
                .field_str("content", content);
            if *is_error {
                b = b.field_bool("is_error", true);
            }
            b.build()
        }
    }
}

// ── Response parsing ────────────────────────────────────────────────────────

fn parse_api_response(json: &JsonValue) -> Result<LlmResponse, LlmError> {
    let stop_reason = match json.get("stop_reason").and_then(|v| v.as_str()) {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some(other) => StopReason::Other(other.to_string()),
        None => StopReason::Other("missing".to_string()),
    };

    let content = parse_content_blocks(json)?;

    let usage = json.get("usage");
    let usage_input = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let usage_output = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Ok(LlmResponse {
        stop_reason,
        content,
        usage_input,
        usage_output,
    })
}

fn parse_content_blocks(json: &JsonValue) -> Result<Vec<ContentBlock>, LlmError> {
    let content_arr = json
        .get("content")
        .and_then(|v| v.as_array())
        .ok_or_else(|| LlmError::Json("missing 'content' array".into()))?;

    let mut blocks = Vec::new();
    for item in content_arr {
        let block_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                let text = item
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                blocks.push(ContentBlock::Text { text });
            }
            "tool_use" => {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = item.get("input").cloned().unwrap_or(JsonValue::Null);
                blocks.push(ContentBlock::ToolUse { id, name, input });
            }
            _ => {
                // Skip unknown block types
            }
        }
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_end_turn_response() {
        let json_str = r#"{
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello there!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }"#;
        let json = json::parse(json_str).unwrap();
        let resp = parse_api_response(&json).unwrap();

        assert!(matches!(resp.stop_reason, StopReason::EndTurn));
        assert_eq!(resp.content.len(), 1);
        match &resp.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello there!"),
            _ => panic!("expected text block"),
        }
        assert_eq!(resp.usage_input, 10);
        assert_eq!(resp.usage_output, 5);
    }

    #[test]
    fn test_parse_tool_use_response() {
        let json_str = r#"{
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Let me check that."},
                {"type": "tool_use", "id": "tu_789", "name": "read_file", "input": {"path": "/tmp/test"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 15}
        }"#;
        let json = json::parse(json_str).unwrap();
        let resp = parse_api_response(&json).unwrap();

        assert!(matches!(resp.stop_reason, StopReason::ToolUse));
        assert_eq!(resp.content.len(), 2);
        match &resp.content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "tu_789");
                assert_eq!(name, "read_file");
                assert_eq!(input.get("path").unwrap().as_str().unwrap(), "/tmp/test");
            }
            _ => panic!("expected tool_use block"),
        }
    }

    #[test]
    fn test_parse_max_tokens_response() {
        let json_str = r#"{
            "content": [{"type": "text", "text": "truncated..."}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 100, "output_tokens": 4096}
        }"#;
        let json = json::parse(json_str).unwrap();
        let resp = parse_api_response(&json).unwrap();
        assert!(matches!(resp.stop_reason, StopReason::MaxTokens));
    }

    #[test]
    fn test_message_to_json_roundtrip() {
        let msg = Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: "Hello".into() }],
        };
        let json = message_to_json(&msg);
        assert_eq!(json.get("role").unwrap().as_str().unwrap(), "user");
        let content = json.get("content").unwrap().as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].get("type").unwrap().as_str().unwrap(), "text");
        assert_eq!(content[0].get("text").unwrap().as_str().unwrap(), "Hello");
    }

    #[test]
    fn test_tool_result_to_json() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_123".into(),
            content: "file contents here".into(),
            is_error: false,
        };
        let json = content_block_to_json(&block);
        assert_eq!(json.get("type").unwrap().as_str().unwrap(), "tool_result");
        assert_eq!(json.get("tool_use_id").unwrap().as_str().unwrap(), "tu_123");
        assert!(json.get("is_error").is_none());
    }

    #[test]
    fn test_tool_result_error_to_json() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_456".into(),
            content: "access denied".into(),
            is_error: true,
        };
        let json = content_block_to_json(&block);
        assert_eq!(json.get("is_error").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_tool_def_to_json() {
        let def = ToolDef {
            name: "test_tool".into(),
            description: "A test tool".into(),
            input_schema: json_obj()
                .field_str("type", "object")
                .build(),
        };
        let json = tool_def_to_json(&def);
        assert_eq!(json.get("name").unwrap().as_str().unwrap(), "test_tool");
        assert_eq!(json.get("description").unwrap().as_str().unwrap(), "A test tool");
        assert!(json.get("input_schema").is_some());
    }
}
