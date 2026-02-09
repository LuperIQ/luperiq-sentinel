mod agent;
mod config;
mod llm;
mod messaging;
mod net;
mod security;

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::agent::tools::ToolExecutor;
use crate::config::Config;
use crate::llm::anthropic::AnthropicClient;
use crate::llm::openai::OpenAiClient;
use crate::llm::provider::{ContentBlock, LlmError, LlmProvider, Message, Role, StopReason, ToolDef};
use crate::messaging::telegram::TelegramClient;
use crate::net::http::HttpClient;
use crate::security::audit::{AuditEvent, Auditor};
use crate::security::capability::CapabilityChecker;

const MAX_TOOL_ROUNDS: usize = 10;
const MAX_HISTORY_MESSAGES: usize = 40;

fn main() {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sentinel: fatal: {}", e);
            std::process::exit(1);
        }
    };

    let mut auditor = Auditor::new(config.audit_log_path.as_deref());

    let caps = CapabilityChecker::new(
        config.allowed_read_paths.clone(),
        config.allowed_write_paths.clone(),
        config.allowed_commands.clone(),
    );

    let http = match HttpClient::new() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("sentinel: fatal: failed to initialize HTTP client: {}", e);
            std::process::exit(1);
        }
    };

    // Create LLM provider based on config
    let llm: Box<dyn LlmProvider> = match config.provider.as_str() {
        "openai" => {
            let llm_http = match HttpClient::new() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("sentinel: fatal: {}", e);
                    std::process::exit(1);
                }
            };
            eprintln!("sentinel: using OpenAI provider ({})", config.openai_base_url);
            Box::new(OpenAiClient::new(
                llm_http,
                config.api_key.clone(),
                config.model.clone(),
                config.max_tokens,
                config.openai_base_url.clone(),
            ))
        }
        _ => {
            let llm_http = match HttpClient::new() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("sentinel: fatal: {}", e);
                    std::process::exit(1);
                }
            };
            eprintln!("sentinel: using Anthropic provider");
            Box::new(AnthropicClient::new(
                llm_http,
                config.api_key.clone(),
                config.model.clone(),
                config.max_tokens,
            ))
        }
    };

    let mut telegram = TelegramClient::new(http, &config.telegram_token);

    let tool_defs = ToolExecutor::tool_definitions();
    let tool_executor = ToolExecutor::new(&caps, config.command_timeout);

    // Per-chat conversation history
    let mut conversations: HashMap<i64, Vec<Message>> = HashMap::new();

    eprintln!("sentinel: started, polling Telegram...");

    loop {
        let updates = match telegram.get_updates(30) {
            Ok(msgs) => msgs,
            Err(e) => {
                eprintln!("sentinel: telegram poll error: {}", e);
                thread::sleep(Duration::from_secs(5));
                continue;
            }
        };

        for msg in updates {
            let username = msg.from_username.as_deref().unwrap_or("unknown");

            auditor.log(AuditEvent::MessageReceived {
                chat_id: msg.chat_id,
                user_id: msg.from_id,
                username,
            });

            // Authorization check
            if !config.telegram_allowed_users.is_empty()
                && !config.telegram_allowed_users.contains(&msg.from_id)
            {
                auditor.log(AuditEvent::UnauthorizedUser {
                    user_id: msg.from_id,
                    username,
                });
                let _ = telegram.send_message(msg.chat_id, "Unauthorized.");
                continue;
            }

            // Handle /clear command
            if msg.text.trim() == "/clear" {
                conversations.remove(&msg.chat_id);
                let _ = telegram.send_message(msg.chat_id, "Conversation cleared.");
                continue;
            }

            // Get or create conversation history
            let history = conversations.entry(msg.chat_id).or_default();

            // Add user message
            history.push(Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: msg.text.clone(),
                }],
            });

            // Run agent turn with streaming
            match run_agent_turn(
                llm.as_ref(),
                history,
                &config,
                &tool_defs,
                &tool_executor,
                &mut auditor,
                &telegram,
                msg.chat_id,
            ) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("sentinel: agent error: {}", e);
                    let error_msg = format!("Error: {}", e);
                    let _ = telegram.send_message(msg.chat_id, &error_msg);
                }
            }

            // Trim history if too long
            if history.len() > MAX_HISTORY_MESSAGES {
                let drain_count = history.len() - MAX_HISTORY_MESSAGES;
                history.drain(..drain_count);
            }
        }
    }
}

fn run_agent_turn(
    llm: &dyn LlmProvider,
    history: &mut Vec<Message>,
    config: &Config,
    tool_defs: &[ToolDef],
    tool_executor: &ToolExecutor,
    auditor: &mut Auditor,
    telegram: &TelegramClient,
    chat_id: i64,
) -> Result<(), String> {
    let system = config.system_prompt.as_deref();

    for _round in 0..MAX_TOOL_ROUNDS {
        // Streaming state for real-time Telegram updates
        let mut streamed_text = String::new();
        let mut telegram_msg_id: Option<i64> = None;
        let mut last_edit = Instant::now();

        let api_resp = {
            let streamed_text_ref = &mut streamed_text;
            let telegram_msg_id_ref = &mut telegram_msg_id;
            let last_edit_ref = &mut last_edit;

            let mut on_text = |delta: &str| {
                streamed_text_ref.push_str(delta);

                // Send/edit Telegram message periodically (every 500ms)
                let should_update = last_edit_ref.elapsed() >= Duration::from_millis(500);
                if !should_update {
                    return;
                }

                if let Some(msg_id) = *telegram_msg_id_ref {
                    let _ = telegram.edit_message_text(chat_id, msg_id, streamed_text_ref);
                } else if streamed_text_ref.len() >= 10 {
                    // Wait for at least 10 chars before sending initial message
                    match telegram.send_message_get_id(chat_id, streamed_text_ref) {
                        Ok(id) => *telegram_msg_id_ref = Some(id),
                        Err(e) => eprintln!("sentinel: stream send error: {}", e),
                    }
                }
                *last_edit_ref = Instant::now();
            };

            match llm.send_streaming(system, history, tool_defs, &mut on_text) {
                Ok(r) => r,
                Err(LlmError::RateLimit { retry_after }) => {
                    let wait = retry_after.unwrap_or(10);
                    eprintln!("sentinel: rate limited, waiting {}s", wait);
                    thread::sleep(Duration::from_secs(wait));
                    // Retry once (non-streaming fallback)
                    llm.send(system, history, tool_defs)
                        .map_err(|e| format!("LLM API error: {}", e))?
                }
                Err(e) => return Err(format!("LLM API error: {}", e)),
            }
        };

        // Add assistant response to history
        history.push(Message {
            role: Role::Assistant,
            content: api_resp.content.clone(),
        });

        match api_resp.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens => {
                let text = extract_text(&api_resp.content);

                // Send final text via Telegram
                if let Some(msg_id) = telegram_msg_id {
                    // Edit with final complete text
                    let _ = telegram.edit_message_text(chat_id, msg_id, &text);
                } else {
                    // No streaming happened (or very short response) — send normally
                    if let Err(e) = telegram.send_message(chat_id, &text) {
                        eprintln!("sentinel: failed to send message: {}", e);
                    }
                }
                return Ok(());
            }
            StopReason::ToolUse => {
                // If we streamed partial text, finalize it
                if let Some(msg_id) = telegram_msg_id {
                    let text = extract_text(&api_resp.content);
                    if !text.is_empty() {
                        let _ = telegram.edit_message_text(chat_id, msg_id, &text);
                    }
                }

                // Execute each tool call
                let mut tool_results = Vec::new();
                for block in &api_resp.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        eprintln!("sentinel: tool call: {}({})", name, input.to_json_string());
                        let result =
                            tool_executor.execute(id, name, input, auditor);
                        tool_results.push(result);
                    }
                }

                // Add tool results as user message
                if !tool_results.is_empty() {
                    history.push(Message {
                        role: Role::User,
                        content: tool_results,
                    });
                }
            }
            StopReason::Other(ref reason) => {
                return Err(format!("unexpected stop reason: {}", reason));
            }
        }
    }

    Err("max tool rounds exceeded".into())
}

fn extract_text(content: &[ContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in content {
        if let ContentBlock::Text { text } = block {
            parts.push(text.as_str());
        }
    }
    if parts.is_empty() {
        "(no text response)".to_string()
    } else {
        parts.join("\n")
    }
}
