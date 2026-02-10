use crate::messaging::{Connector, ConnectorError, IncomingMessage};
use crate::net::http::{HttpClient, HttpError};
use crate::net::json::{self, json_obj, JsonValue};

// ── Types ───────────────────────────────────────────────────────────────────

pub struct TelegramMessage {
    pub update_id: i64,
    pub chat_id: i64,
    pub from_id: i64,
    pub from_username: Option<String>,
    pub text: String,
}

#[derive(Debug)]
pub enum TelegramError {
    Http(HttpError),
    Json(String),
    Api(String),
}

impl std::fmt::Display for TelegramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TelegramError::Http(e) => write!(f, "Telegram HTTP error: {}", e),
            TelegramError::Json(s) => write!(f, "Telegram JSON error: {}", s),
            TelegramError::Api(s) => write!(f, "Telegram API error: {}", s),
        }
    }
}

impl From<HttpError> for TelegramError {
    fn from(e: HttpError) -> Self {
        TelegramError::Http(e)
    }
}

// ── Client ──────────────────────────────────────────────────────────────────

const TELEGRAM_MSG_LIMIT: usize = 4096;

pub struct TelegramClient {
    http: HttpClient,
    base_url: String,
    last_offset: i64,
}

impl TelegramClient {
    pub fn new(http: HttpClient, token: &str) -> Self {
        TelegramClient {
            http,
            base_url: format!("https://api.telegram.org/bot{}", token),
            last_offset: 0,
        }
    }

    pub fn get_updates(&mut self, timeout: u32) -> Result<Vec<TelegramMessage>, TelegramError> {
        let url = format!(
            "{}/getUpdates?offset={}&timeout={}&allowed_updates=[\"message\"]",
            self.base_url, self.last_offset, timeout
        );

        let resp = self.http.get(&url, &[])?;
        let body = resp
            .body_string()
            .map_err(|e| TelegramError::Http(e))?;
        let json = json::parse(&body).map_err(|e| TelegramError::Json(e.to_string()))?;

        let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(TelegramError::Api(desc.to_string()));
        }

        let results = json
            .get("result")
            .and_then(|v| v.as_array())
            .ok_or_else(|| TelegramError::Json("missing 'result' array".into()))?;

        let mut messages = Vec::new();
        for update in results {
            if let Some(msg) = parse_update(update) {
                if msg.update_id >= self.last_offset {
                    self.last_offset = msg.update_id + 1;
                }
                messages.push(msg);
            }
        }

        Ok(messages)
    }

    pub fn send_message(&self, chat_id: i64, text: &str) -> Result<(), TelegramError> {
        let chunks = split_message(text);
        for chunk in &chunks {
            self.send_message_raw(chat_id, chunk)?;
        }
        Ok(())
    }

    /// Send a message and return its message_id (for later editing).
    pub fn send_message_get_id(&self, chat_id: i64, text: &str) -> Result<i64, TelegramError> {
        self.send_message_raw(chat_id, text)
    }

    /// Edit an existing message's text.
    pub fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
    ) -> Result<(), TelegramError> {
        let body = json_obj()
            .field_i64("chat_id", chat_id)
            .field_i64("message_id", message_id)
            .field_str("text", text)
            .build();

        let url = format!("{}/editMessageText", self.base_url);
        let resp = self.http.post_json(&url, &body.to_json_string(), &[])?;

        let body_str = resp.body_string().map_err(|e| TelegramError::Http(e))?;
        let json = json::parse(&body_str).map_err(|e| TelegramError::Json(e.to_string()))?;

        let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            // "message is not modified" is not a real error — just means text didn't change
            if !desc.contains("message is not modified") {
                return Err(TelegramError::Api(desc.to_string()));
            }
        }
        Ok(())
    }

    fn send_message_raw(&self, chat_id: i64, text: &str) -> Result<i64, TelegramError> {
        let body = json_obj()
            .field_i64("chat_id", chat_id)
            .field_str("text", text)
            .build();

        let url = format!("{}/sendMessage", self.base_url);
        let resp = self.http.post_json(&url, &body.to_json_string(), &[])?;

        let body_str = resp.body_string().map_err(|e| TelegramError::Http(e))?;
        let json = json::parse(&body_str).map_err(|e| TelegramError::Json(e.to_string()))?;

        let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(TelegramError::Api(desc.to_string()));
        }

        // Extract message_id from result
        let message_id = json
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        Ok(message_id)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_update(update: &JsonValue) -> Option<TelegramMessage> {
    let update_id = update.get("update_id")?.as_i64()?;
    let message = update.get("message")?;
    let text = message.get("text")?.as_str()?;
    let chat = message.get("chat")?;
    let chat_id = chat.get("id")?.as_i64()?;

    let from = message.get("from");
    let from_id = from.and_then(|f| f.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
    let from_username = from
        .and_then(|f| f.get("username"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(TelegramMessage {
        update_id,
        chat_id,
        from_id,
        from_username,
        text: text.to_string(),
    })
}

fn split_message(text: &str) -> Vec<String> {
    if text.len() <= TELEGRAM_MSG_LIMIT {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= TELEGRAM_MSG_LIMIT {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at a newline before the limit
        let split_at = remaining[..TELEGRAM_MSG_LIMIT]
            .rfind('\n')
            .unwrap_or(TELEGRAM_MSG_LIMIT);

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.to_string());

        // Skip the newline we split at
        remaining = if rest.starts_with('\n') {
            &rest[1..]
        } else {
            rest
        };
    }

    chunks
}

// ── Connector trait impl ─────────────────────────────────────────────────────

impl From<TelegramError> for ConnectorError {
    fn from(e: TelegramError) -> Self {
        match e {
            TelegramError::Http(h) => ConnectorError::Http(h),
            TelegramError::Json(s) => ConnectorError::Json(s),
            TelegramError::Api(s) => ConnectorError::Api(s),
        }
    }
}

impl Connector for TelegramClient {
    fn poll_messages(&mut self, timeout_secs: u32) -> Result<Vec<IncomingMessage>, ConnectorError> {
        let msgs = self.get_updates(timeout_secs)?;
        Ok(msgs
            .into_iter()
            .map(|m| IncomingMessage {
                channel_id: m.chat_id.to_string(),
                user_id: m.from_id.to_string(),
                username: m.from_username,
                text: m.text,
            })
            .collect())
    }

    fn send_message(&self, channel_id: &str, text: &str) -> Result<(), ConnectorError> {
        let chat_id: i64 = channel_id
            .parse()
            .map_err(|_| ConnectorError::Api("invalid chat_id".into()))?;
        TelegramClient::send_message(self, chat_id, text)?;
        Ok(())
    }

    fn send_message_get_id(&self, channel_id: &str, text: &str) -> Result<String, ConnectorError> {
        let chat_id: i64 = channel_id
            .parse()
            .map_err(|_| ConnectorError::Api("invalid chat_id".into()))?;
        let msg_id = TelegramClient::send_message_get_id(self, chat_id, text)?;
        Ok(msg_id.to_string())
    }

    fn edit_message_text(
        &self,
        channel_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), ConnectorError> {
        let chat_id: i64 = channel_id
            .parse()
            .map_err(|_| ConnectorError::Api("invalid chat_id".into()))?;
        let msg_id: i64 = message_id
            .parse()
            .map_err(|_| ConnectorError::Api("invalid message_id".into()))?;
        TelegramClient::edit_message_text(self, chat_id, msg_id, text)?;
        Ok(())
    }

    fn platform_name(&self) -> &'static str {
        "telegram"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let chunks = split_message("hello");
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let long = "a".repeat(5000);
        let chunks = split_message(&long);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), TELEGRAM_MSG_LIMIT);
        assert_eq!(chunks[1].len(), 5000 - TELEGRAM_MSG_LIMIT);
    }
}
