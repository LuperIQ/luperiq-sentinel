use crate::net::http::{HttpError, StreamingResponse};

// ── Server-Sent Events parser ───────────────────────────────────────────────

pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

/// Read a single SSE event from the streaming response.
/// Returns None on end of stream.
pub fn read_event(response: &mut StreamingResponse) -> Result<Option<SseEvent>, HttpError> {
    let mut event_type = String::new();
    let mut data_parts: Vec<String> = Vec::new();
    let mut got_content = false;

    loop {
        let line = response.read_line()?;

        // EOF
        if line.is_empty() && !got_content {
            return Ok(None);
        }

        let line = line.trim_end_matches('\r');

        // Empty line = end of event
        if line.is_empty() {
            if got_content {
                return Ok(Some(SseEvent {
                    event_type,
                    data: data_parts.join("\n"),
                }));
            }
            continue;
        }

        // Comment lines (starting with :)
        if line.starts_with(':') {
            continue;
        }

        if let Some(value) = line.strip_prefix("event:") {
            event_type = value.trim().to_string();
            got_content = true;
        } else if let Some(value) = line.strip_prefix("data:") {
            data_parts.push(value.trim().to_string());
            got_content = true;
        } else if let Some(value) = line.strip_prefix("event: ") {
            event_type = value.to_string();
            got_content = true;
        } else if let Some(value) = line.strip_prefix("data: ") {
            data_parts.push(value.to_string());
            got_content = true;
        }
        // Ignore other fields (id:, retry:)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: create a StreamingResponse from raw bytes for testing
    // We can't easily test this without a real stream, so we test the parsing logic
    // through integration with the anthropic module instead.

    #[test]
    fn test_sse_event_struct() {
        let event = SseEvent {
            event_type: "message_start".to_string(),
            data: r#"{"type":"message_start"}"#.to_string(),
        };
        assert_eq!(event.event_type, "message_start");
        assert!(event.data.contains("message_start"));
    }
}
