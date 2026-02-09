use crate::net::http::{HttpClient, HttpError};
use crate::net::json::{self, JsonValue, json_obj, json_arr};

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: JsonValue },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

#[derive(Debug)]
pub struct ApiResponse {
    pub stop_reason: StopReason,
    pub content: Vec<ContentBlock>,
    pub usage_input: i64,
    pub usage_output: i64,
}

#[derive(Debug)]
pub enum AnthropicError {
    Http(HttpError),
    Json(String),
    Api { status: u16, message: String },
    RateLimit { retry_after: Option<u64> },
}

impl std::fmt::Display for AnthropicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnthropicError::Http(e) => write!(f, "HTTP error: {}", e),
            AnthropicError::Json(s) => write!(f, "JSON error: {}", s),
            AnthropicError::Api { status, message } => {
                write!(f, "API error ({}): {}", status, message)
            }
            AnthropicError::RateLimit { retry_after } => {
                write!(f, "rate limited")?;
                if let Some(s) = retry_after {
                    write!(f, " (retry after {}s)", s)?;
                }
                Ok(())
            }
        }
    }
}

impl From<HttpError> for AnthropicError {
    fn from(e: HttpError) -> Self {
        AnthropicError::Http(e)
    }
}

// ── Tool definition ─────────────────────────────────────────────────────────

pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: JsonValue,
}

impl ToolDef {
    fn to_json(&self) -> JsonValue {
        json_obj()
            .field_str("name", &self.name)
            .field_str("description", &self.description)
            .field("input_schema", self.input_schema.clone())
            .build()
    }
}

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

    pub fn send(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<ApiResponse, AnthropicError> {
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
            return Err(AnthropicError::RateLimit { retry_after });
        }

        let body_str = resp.body_string().map_err(|e| AnthropicError::Http(e))?;
        let json_val =
            json::parse(&body_str).map_err(|e| AnthropicError::Json(e.to_string()))?;

        if resp.status != 200 {
            let msg = json_val
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(AnthropicError::Api {
                status: resp.status,
                message: msg.to_string(),
            });
        }

        parse_api_response(&json_val)
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
                tool_arr = tool_arr.push(t.to_json());
            }
            body = body.field("tools", tool_arr.build());
        }

        body.build()
    }
}

// ── JSON serialization helpers ──────────────────────────────────────────────

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

fn parse_api_response(json: &JsonValue) -> Result<ApiResponse, AnthropicError> {
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

    Ok(ApiResponse {
        stop_reason,
        content,
        usage_input,
        usage_output,
    })
}

fn parse_content_blocks(json: &JsonValue) -> Result<Vec<ContentBlock>, AnthropicError> {
    let content_arr = json
        .get("content")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AnthropicError::Json("missing 'content' array".into()))?;

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
