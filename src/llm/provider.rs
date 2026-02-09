use crate::net::json::JsonValue;

// ── Shared types for all LLM providers ──────────────────────────────────────

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
pub struct LlmResponse {
    pub stop_reason: StopReason,
    pub content: Vec<ContentBlock>,
    pub usage_input: i64,
    pub usage_output: i64,
}

#[derive(Debug)]
pub enum LlmError {
    Http(crate::net::http::HttpError),
    Json(String),
    Api { status: u16, message: String },
    RateLimit { retry_after: Option<u64> },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(e) => write!(f, "HTTP error: {}", e),
            LlmError::Json(s) => write!(f, "JSON error: {}", s),
            LlmError::Api { status, message } => {
                write!(f, "API error ({}): {}", status, message)
            }
            LlmError::RateLimit { retry_after } => {
                write!(f, "rate limited")?;
                if let Some(s) = retry_after {
                    write!(f, " (retry after {}s)", s)?;
                }
                Ok(())
            }
        }
    }
}

impl From<crate::net::http::HttpError> for LlmError {
    fn from(e: crate::net::http::HttpError) -> Self {
        LlmError::Http(e)
    }
}

// ── Tool definition (shared across providers) ───────────────────────────────

pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: JsonValue,
}

// ── Provider trait ──────────────────────────────────────────────────────────

pub trait LlmProvider {
    fn send(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse, LlmError>;

    /// Send with streaming. Calls `on_text` for each text delta as it arrives.
    /// Returns the complete response when done.
    /// Default implementation falls back to non-streaming `send()`.
    fn send_streaming(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDef],
        on_text: &mut dyn FnMut(&str),
    ) -> Result<LlmResponse, LlmError> {
        let resp = self.send(system, messages, tools)?;
        for block in &resp.content {
            if let ContentBlock::Text { text } = block {
                on_text(text);
            }
        }
        Ok(resp)
    }
}
