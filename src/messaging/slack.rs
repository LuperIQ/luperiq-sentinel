use std::collections::HashMap;

use crate::messaging::{split_message, Connector, ConnectorError, IncomingMessage};
use crate::net::http::HttpClient;
use crate::net::json::{self, json_obj};

const SLACK_API: &str = "https://slack.com/api";
const SLACK_MSG_LIMIT: usize = 40000;

// ── Client ──────────────────────────────────────────────────────────────────

pub struct SlackConnector {
    http: HttpClient,
    token: String,
    channel_ids: Vec<String>,
    bot_user_id: String,
    last_timestamps: HashMap<String, String>,
    initialized_channels: HashMap<String, bool>,
}

impl SlackConnector {
    pub fn new(
        http: HttpClient,
        token: &str,
        channel_ids: &[String],
    ) -> Result<Self, ConnectorError> {
        // Get bot user ID via auth.test
        let auth = format!("Bearer {}", token);
        let url = format!("{}/auth.test", SLACK_API);
        let resp = http.post_json(&url, "{}", &[("Authorization", &auth)])?;
        let body = resp
            .body_string()
            .map_err(|e| ConnectorError::Http(e))?;
        let json_val =
            json::parse(&body).map_err(|e| ConnectorError::Json(e.to_string()))?;

        if !json_val
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let error = json_val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ConnectorError::Api(format!(
                "Slack auth.test failed: {}",
                error
            )));
        }

        let bot_user_id = json_val
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError::Api("missing user_id in auth.test".into()))?
            .to_string();

        eprintln!(
            "sentinel: slack connector ready (bot_user_id={}, channels={})",
            bot_user_id,
            channel_ids.len()
        );

        Ok(SlackConnector {
            http,
            token: token.to_string(),
            channel_ids: channel_ids.to_vec(),
            bot_user_id,
            last_timestamps: HashMap::new(),
            initialized_channels: HashMap::new(),
        })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

// ── Connector impl ──────────────────────────────────────────────────────────

impl Connector for SlackConnector {
    fn poll_messages(
        &mut self,
        _timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, ConnectorError> {
        let mut all_messages = Vec::new();
        let auth = self.auth_header();

        for channel_id in &self.channel_ids.clone() {
            // First poll: record the latest timestamp without processing messages
            if !self.initialized_channels.contains_key(channel_id) {
                let url = format!(
                    "{}/conversations.history?channel={}&limit=1",
                    SLACK_API, channel_id
                );
                match self.http.get(&url, &[("Authorization", &auth)]) {
                    Ok(resp) => {
                        if let Ok(body) = resp.body_string() {
                            if let Ok(json_val) = json::parse(&body) {
                                if let Some(msgs) =
                                    json_val.get("messages").and_then(|v| v.as_array())
                                {
                                    if let Some(latest) = msgs.first() {
                                        if let Some(ts) =
                                            latest.get("ts").and_then(|v| v.as_str())
                                        {
                                            self.last_timestamps
                                                .insert(channel_id.clone(), ts.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "sentinel: slack init error for {}: {}",
                            channel_id, e
                        );
                    }
                }
                self.initialized_channels.insert(channel_id.clone(), true);
                continue;
            }

            // Normal poll: fetch messages after the last seen timestamp
            let mut url = format!(
                "{}/conversations.history?channel={}&limit=100",
                SLACK_API, channel_id
            );
            if let Some(last_ts) = self.last_timestamps.get(channel_id) {
                url.push_str(&format!("&oldest={}", last_ts));
            }

            let resp = match self.http.get(&url, &[("Authorization", &auth)]) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sentinel: slack poll error for {}: {}", channel_id, e);
                    continue;
                }
            };

            let body = match resp.body_string() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let json_val = match json::parse(&body) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if !json_val
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let error = json_val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                eprintln!(
                    "sentinel: slack history error for {}: {}",
                    channel_id, error
                );
                continue;
            }

            let empty = Vec::new();
            let messages = json_val
                .get("messages")
                .and_then(|v| v.as_array())
                .unwrap_or(&empty);

            // Slack returns messages newest-first; reverse to process in chronological order
            let mut msgs_vec: Vec<_> = messages.iter().collect();
            msgs_vec.reverse();

            for msg in msgs_vec {
                // Only process regular messages
                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if msg_type != "message" {
                    continue;
                }

                // Skip messages with subtype (bot_message, channel_join, etc.)
                if msg.get("subtype").is_some() {
                    continue;
                }

                let user_id = msg.get("user").and_then(|v| v.as_str()).unwrap_or("");
                if user_id.is_empty() || user_id == self.bot_user_id {
                    continue;
                }

                let ts = match msg.get("ts").and_then(|v| v.as_str()) {
                    Some(ts) => ts,
                    None => continue,
                };

                // Skip messages at or before the last seen timestamp
                if let Some(last_ts) = self.last_timestamps.get(channel_id) {
                    if ts <= last_ts.as_str() {
                        continue;
                    }
                }

                let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if text.is_empty() {
                    self.last_timestamps
                        .insert(channel_id.clone(), ts.to_string());
                    continue;
                }

                self.last_timestamps
                    .insert(channel_id.clone(), ts.to_string());

                all_messages.push(IncomingMessage {
                    channel_id: channel_id.clone(),
                    user_id: user_id.to_string(),
                    username: None, // Slack doesn't include username in history
                    text: text.to_string(),
                });
            }
        }

        Ok(all_messages)
    }

    fn send_message(&self, channel_id: &str, text: &str) -> Result<(), ConnectorError> {
        let auth = self.auth_header();
        let url = format!("{}/chat.postMessage", SLACK_API);

        for chunk in split_message(text, SLACK_MSG_LIMIT) {
            let body = json_obj()
                .field_str("channel", channel_id)
                .field_str("text", &chunk)
                .build();
            let resp =
                self.http
                    .post_json(&url, &body.to_json_string(), &[("Authorization", &auth)])?;
            let body_str = resp.body_string().map_err(|e| ConnectorError::Http(e))?;
            let json_val =
                json::parse(&body_str).map_err(|e| ConnectorError::Json(e.to_string()))?;
            if !json_val
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let error = json_val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Err(ConnectorError::Api(format!(
                    "Slack send error: {}",
                    error
                )));
            }
        }
        Ok(())
    }

    fn send_message_get_id(
        &self,
        channel_id: &str,
        text: &str,
    ) -> Result<String, ConnectorError> {
        let auth = self.auth_header();
        let url = format!("{}/chat.postMessage", SLACK_API);
        let body = json_obj()
            .field_str("channel", channel_id)
            .field_str("text", text)
            .build();
        let resp =
            self.http
                .post_json(&url, &body.to_json_string(), &[("Authorization", &auth)])?;
        let body_str = resp.body_string().map_err(|e| ConnectorError::Http(e))?;
        let json_val =
            json::parse(&body_str).map_err(|e| ConnectorError::Json(e.to_string()))?;

        if !json_val
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let error = json_val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ConnectorError::Api(format!(
                "Slack send error: {}",
                error
            )));
        }

        // Slack uses the message timestamp as its ID
        let ts = json_val
            .get("ts")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError::Api("missing ts in response".into()))?;
        Ok(ts.to_string())
    }

    fn edit_message_text(
        &self,
        channel_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), ConnectorError> {
        let auth = self.auth_header();
        let url = format!("{}/chat.update", SLACK_API);
        let body = json_obj()
            .field_str("channel", channel_id)
            .field_str("ts", message_id)
            .field_str("text", text)
            .build();
        let resp =
            self.http
                .post_json(&url, &body.to_json_string(), &[("Authorization", &auth)])?;
        let body_str = resp.body_string().map_err(|e| ConnectorError::Http(e))?;
        let json_val =
            json::parse(&body_str).map_err(|e| ConnectorError::Json(e.to_string()))?;

        if !json_val
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let error = json_val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            // "message_not_modified" is not a real error
            if error != "message_not_modified" {
                return Err(ConnectorError::Api(format!(
                    "Slack edit error: {}",
                    error
                )));
            }
        }
        Ok(())
    }

    fn platform_name(&self) -> &'static str {
        "slack"
    }
}
