use std::collections::HashMap;

use crate::messaging::{split_message, Connector, ConnectorError, IncomingMessage};
use crate::net::http::HttpClient;
use crate::net::json::{self, json_obj};

const DISCORD_API: &str = "https://discord.com/api/v10";
const DISCORD_MSG_LIMIT: usize = 2000;

// ── Client ──────────────────────────────────────────────────────────────────

pub struct DiscordConnector {
    http: HttpClient,
    token: String,
    channel_ids: Vec<String>,
    bot_user_id: String,
    last_message_ids: HashMap<String, String>,
    initialized_channels: HashMap<String, bool>,
}

impl DiscordConnector {
    pub fn new(
        http: HttpClient,
        token: &str,
        channel_ids: &[String],
    ) -> Result<Self, ConnectorError> {
        // Get bot user ID via GET /users/@me
        let auth = format!("Bot {}", token);
        let url = format!("{}/users/@me", DISCORD_API);
        let resp = http.get(&url, &[("Authorization", &auth)])?;
        let body = resp
            .body_string()
            .map_err(|e| ConnectorError::Http(e))?;
        let json_val =
            json::parse(&body).map_err(|e| ConnectorError::Json(e.to_string()))?;
        let bot_user_id = json_val
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError::Api("failed to get bot user ID from /users/@me".into()))?
            .to_string();

        eprintln!(
            "sentinel: discord connector ready (bot_user_id={}, channels={})",
            bot_user_id,
            channel_ids.len()
        );

        Ok(DiscordConnector {
            http,
            token: token.to_string(),
            channel_ids: channel_ids.to_vec(),
            bot_user_id,
            last_message_ids: HashMap::new(),
            initialized_channels: HashMap::new(),
        })
    }

    fn auth_header(&self) -> String {
        format!("Bot {}", self.token)
    }
}

// ── Connector impl ──────────────────────────────────────────────────────────

impl Connector for DiscordConnector {
    fn poll_messages(
        &mut self,
        _timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, ConnectorError> {
        let mut all_messages = Vec::new();
        let auth = self.auth_header();

        for channel_id in &self.channel_ids.clone() {
            // First poll for this channel: just record the latest message ID
            if !self.initialized_channels.contains_key(channel_id) {
                let url = format!(
                    "{}/channels/{}/messages?limit=1",
                    DISCORD_API, channel_id
                );
                match self.http.get(&url, &[("Authorization", &auth)]) {
                    Ok(resp) if resp.status == 200 => {
                        if let Ok(body) = resp.body_string() {
                            if let Ok(json_val) = json::parse(&body) {
                                if let Some(msgs) = json_val.as_array() {
                                    if let Some(latest) = msgs.first() {
                                        if let Some(id) =
                                            latest.get("id").and_then(|v| v.as_str())
                                        {
                                            self.last_message_ids
                                                .insert(channel_id.clone(), id.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.initialized_channels.insert(channel_id.clone(), true);
                continue;
            }

            // Normal poll: fetch messages after the last seen ID
            let mut url = format!(
                "{}/channels/{}/messages?limit=100",
                DISCORD_API, channel_id
            );
            if let Some(last_id) = self.last_message_ids.get(channel_id) {
                url.push_str(&format!("&after={}", last_id));
            }

            let resp = match self.http.get(&url, &[("Authorization", &auth)]) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sentinel: discord poll error for {}: {}", channel_id, e);
                    continue;
                }
            };

            // Rate limited — skip this cycle
            if resp.status == 429 {
                eprintln!("sentinel: discord rate limited on channel {}", channel_id);
                continue;
            }

            if resp.status != 200 {
                continue;
            }

            let body = resp.body_string().map_err(|e| ConnectorError::Http(e))?;
            let json_val =
                json::parse(&body).map_err(|e| ConnectorError::Json(e.to_string()))?;

            let messages = match json_val.as_array() {
                Some(arr) => arr,
                None => continue,
            };

            // With after=, Discord returns messages sorted by ID ascending.
            for msg in messages {
                let msg_id = match msg.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => continue,
                };

                let author_id = msg
                    .get("author")
                    .and_then(|a| a.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Skip bot's own messages
                if author_id == self.bot_user_id {
                    self.last_message_ids
                        .insert(channel_id.clone(), msg_id.to_string());
                    continue;
                }

                // Only process DEFAULT message type (0)
                let msg_type = msg.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
                if msg_type != 0 {
                    self.last_message_ids
                        .insert(channel_id.clone(), msg_id.to_string());
                    continue;
                }

                let content = msg
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Skip empty messages (attachments-only, embeds, etc.)
                if content.is_empty() {
                    self.last_message_ids
                        .insert(channel_id.clone(), msg_id.to_string());
                    continue;
                }

                let username = msg
                    .get("author")
                    .and_then(|a| a.get("username"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                self.last_message_ids
                    .insert(channel_id.clone(), msg_id.to_string());

                all_messages.push(IncomingMessage {
                    channel_id: channel_id.clone(),
                    user_id: author_id.to_string(),
                    username,
                    text: content.to_string(),
                });
            }
        }

        Ok(all_messages)
    }

    fn send_message(&self, channel_id: &str, text: &str) -> Result<(), ConnectorError> {
        let auth = self.auth_header();
        let url = format!("{}/channels/{}/messages", DISCORD_API, channel_id);

        for chunk in split_message(text, DISCORD_MSG_LIMIT) {
            let body = json_obj().field_str("content", &chunk).build();
            let resp =
                self.http
                    .post_json(&url, &body.to_json_string(), &[("Authorization", &auth)])?;
            if resp.status >= 400 {
                let err_body = resp.body_string().unwrap_or_default();
                return Err(ConnectorError::Api(format!(
                    "Discord send failed ({}): {}",
                    resp.status, err_body
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
        let url = format!("{}/channels/{}/messages", DISCORD_API, channel_id);
        let body = json_obj().field_str("content", text).build();
        let resp =
            self.http
                .post_json(&url, &body.to_json_string(), &[("Authorization", &auth)])?;
        let body_str = resp.body_string().map_err(|e| ConnectorError::Http(e))?;
        let json_val =
            json::parse(&body_str).map_err(|e| ConnectorError::Json(e.to_string()))?;
        let msg_id = json_val
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError::Api("missing message id in response".into()))?;
        Ok(msg_id.to_string())
    }

    fn edit_message_text(
        &self,
        channel_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), ConnectorError> {
        let auth = self.auth_header();
        let url = format!(
            "{}/channels/{}/messages/{}",
            DISCORD_API, channel_id, message_id
        );
        let body = json_obj().field_str("content", text).build();
        let resp = self.http.patch_json(
            &url,
            &body.to_json_string(),
            &[("Authorization", &auth)],
        )?;
        if resp.status >= 400 {
            let err_body = resp.body_string().unwrap_or_default();
            return Err(ConnectorError::Api(format!(
                "Discord edit failed ({}): {}",
                resp.status, err_body
            )));
        }
        Ok(())
    }

    fn platform_name(&self) -> &'static str {
        "discord"
    }
}
