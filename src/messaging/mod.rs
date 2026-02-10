#[cfg(feature = "tls")]
pub mod telegram;
#[cfg(feature = "tls")]
pub mod discord;
#[cfg(feature = "tls")]
pub mod slack;

use crate::net::http::HttpError;

// ── Common types ─────────────────────────────────────────────────────────────

/// A message received from a messaging platform.
pub struct IncomingMessage {
    pub channel_id: String,
    pub user_id: String,
    pub username: Option<String>,
    pub text: String,
}

/// Error from a messaging connector.
#[derive(Debug)]
pub enum ConnectorError {
    Http(HttpError),
    Api(String),
    Json(String),
}

impl std::fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectorError::Http(e) => write!(f, "HTTP error: {}", e),
            ConnectorError::Api(s) => write!(f, "API error: {}", s),
            ConnectorError::Json(s) => write!(f, "JSON error: {}", s),
        }
    }
}

impl From<HttpError> for ConnectorError {
    fn from(e: HttpError) -> Self {
        ConnectorError::Http(e)
    }
}

// ── Connector trait ──────────────────────────────────────────────────────────

/// Trait for messaging platform connectors (Telegram, Discord, Slack).
pub trait Connector {
    /// Poll for new messages. For long-polling platforms (Telegram), `timeout_secs`
    /// controls the poll duration. For HTTP-polling platforms, it is ignored.
    fn poll_messages(&mut self, timeout_secs: u32) -> Result<Vec<IncomingMessage>, ConnectorError>;

    /// Send a text message to a channel.
    fn send_message(&self, channel_id: &str, text: &str) -> Result<(), ConnectorError>;

    /// Send a message and return its platform-specific ID (for later editing).
    fn send_message_get_id(&self, channel_id: &str, text: &str) -> Result<String, ConnectorError>;

    /// Edit an existing message's text.
    fn edit_message_text(
        &self,
        channel_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), ConnectorError>;

    /// Platform name for logging (e.g., "telegram", "discord", "slack").
    fn platform_name(&self) -> &'static str;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Split a message into chunks respecting a maximum length, preferring line boundaries.
pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let split_at = remaining[..max_len]
            .rfind('\n')
            .unwrap_or(max_len);

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.to_string());

        remaining = if rest.starts_with('\n') {
            &rest[1..]
        } else {
            rest
        };
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let chunks = split_message("hello", 100);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let long = "a".repeat(5000);
        let chunks = split_message(&long, 2000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 2000);
        assert_eq!(chunks[2].len(), 1000);
    }

    #[test]
    fn test_split_at_newline() {
        let text = format!("{}line1\n{}line2", "a".repeat(95), "b".repeat(95));
        let chunks = split_message(&text, 105);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].ends_with("line1"));
        assert!(chunks[1].ends_with("line2"));
    }
}
