use std::time::SystemTime;

use crate::net::json::json_obj;
use crate::platform::Platform;

// ── Types ───────────────────────────────────────────────────────────────────

pub struct Auditor<'a> {
    platform: &'a dyn Platform,
}

#[derive(Debug)]
pub enum AuditEvent<'a> {
    ToolCallAllowed { tool: &'a str, params: &'a str },
    ToolCallDenied { tool: &'a str, params: &'a str, reason: &'a str },
    MessageReceived { chat_id: i64, user_id: i64, username: &'a str },
    UnauthorizedUser { user_id: i64, username: &'a str },
    ApiCall { endpoint: &'a str, status: u16 },
}

// ── Implementation ──────────────────────────────────────────────────────────

impl<'a> Auditor<'a> {
    pub fn new(platform: &'a dyn Platform) -> Self {
        Auditor { platform }
    }

    pub fn log(&mut self, event: AuditEvent) {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let json = match event {
            AuditEvent::ToolCallAllowed { tool, params } => json_obj()
                .field_str("event", "tool_call_allowed")
                .field_i64("ts", timestamp as i64)
                .field_str("tool", tool)
                .field_str("params", params)
                .build(),
            AuditEvent::ToolCallDenied { tool, params, reason } => json_obj()
                .field_str("event", "tool_call_denied")
                .field_i64("ts", timestamp as i64)
                .field_str("tool", tool)
                .field_str("params", params)
                .field_str("reason", reason)
                .build(),
            AuditEvent::MessageReceived { chat_id, user_id, username } => json_obj()
                .field_str("event", "message_received")
                .field_i64("ts", timestamp as i64)
                .field_i64("chat_id", chat_id)
                .field_i64("user_id", user_id)
                .field_str("username", username)
                .build(),
            AuditEvent::UnauthorizedUser { user_id, username } => json_obj()
                .field_str("event", "unauthorized_user")
                .field_i64("ts", timestamp as i64)
                .field_i64("user_id", user_id)
                .field_str("username", username)
                .build(),
            AuditEvent::ApiCall { endpoint, status } => json_obj()
                .field_str("event", "api_call")
                .field_i64("ts", timestamp as i64)
                .field_str("endpoint", endpoint)
                .field_i64("status", status as i64)
                .build(),
        };

        let line = json.to_json_string();

        // Delegate to platform — on Linux: eprintln + file append,
        // on LuperIQ: kernel AuditWrite syscall
        let _ = self.platform.audit_event(&line);
    }
}
