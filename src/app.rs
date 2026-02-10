use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::agent::tools::ToolExecutor;
use crate::config::Config;
use crate::llm::anthropic::AnthropicClient;
use crate::llm::openai::OpenAiClient;
use crate::llm::provider::{ContentBlock, LlmError, LlmProvider, Message, Role, StopReason, ToolDef};
use crate::messaging::Connector;
use crate::messaging::discord::DiscordConnector;
use crate::messaging::slack::SlackConnector;
use crate::messaging::telegram::TelegramClient;
use crate::net::http::HttpClient;
use crate::platform::linux::LinuxPlatform;
use crate::security::audit::{AuditEvent, Auditor};

const MAX_TOOL_ROUNDS: usize = 10;
const MAX_HISTORY_MESSAGES: usize = 40;

pub fn run() {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sentinel: fatal: {}", e);
            std::process::exit(1);
        }
    };

    let platform = LinuxPlatform::new(
        config.allowed_read_paths.clone(),
        config.allowed_write_paths.clone(),
        config.allowed_commands.clone(),
        config.audit_log_path.as_deref(),
    );

    // Apply OS-level sandboxing (seccomp + landlock)
    #[cfg(target_os = "linux")]
    if config.sandbox {
        let result = crate::security::linux::apply_sandbox(
            &config.allowed_read_paths,
            &config.allowed_write_paths,
            true,  // enable seccomp
            true,  // enable landlock
        );
        if result.seccomp_applied || result.landlock_applied {
            eprintln!("sentinel: sandbox active (seccomp={}, landlock={})",
                result.seccomp_applied, result.landlock_applied);
        }
    } else {
        eprintln!("sentinel: sandbox disabled (--no-sandbox)");
    }

    let mut auditor = Auditor::new(&platform);

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

    let tool_defs = ToolExecutor::tool_definitions();
    let tool_executor = ToolExecutor::new(&platform, config.command_timeout);

    // Build connectors based on config
    let mut connectors: Vec<Box<dyn Connector>> = Vec::new();

    if let Some(ref token) = config.telegram_token {
        let http = match HttpClient::new() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("sentinel: fatal: failed to initialize HTTP client: {}", e);
                std::process::exit(1);
            }
        };
        connectors.push(Box::new(TelegramClient::new(http, token)));
        eprintln!("sentinel: telegram connector enabled");
    }

    if let Some(ref token) = config.discord_token {
        if config.discord_channel_ids.is_empty() {
            eprintln!("sentinel: warning: discord token set but no channel_ids configured");
        } else {
            let http = match HttpClient::new() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("sentinel: fatal: failed to initialize HTTP client: {}", e);
                    std::process::exit(1);
                }
            };
            match DiscordConnector::new(http, token, &config.discord_channel_ids) {
                Ok(dc) => {
                    connectors.push(Box::new(dc));
                    eprintln!("sentinel: discord connector enabled");
                }
                Err(e) => {
                    eprintln!("sentinel: warning: failed to initialize discord: {}", e);
                }
            }
        }
    }

    if let Some(ref token) = config.slack_bot_token {
        if config.slack_channel_ids.is_empty() {
            eprintln!("sentinel: warning: slack token set but no channel_ids configured");
        } else {
            let http = match HttpClient::new() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("sentinel: fatal: failed to initialize HTTP client: {}", e);
                    std::process::exit(1);
                }
            };
            match SlackConnector::new(http, token, &config.slack_channel_ids) {
                Ok(sc) => {
                    connectors.push(Box::new(sc));
                    eprintln!("sentinel: slack connector enabled");
                }
                Err(e) => {
                    eprintln!("sentinel: warning: failed to initialize slack: {}", e);
                }
            }
        }
    }

    if connectors.is_empty() {
        eprintln!("sentinel: fatal: no messaging connectors available");
        std::process::exit(1);
    }

    // Per-conversation history keyed by "platform:channel_id"
    let mut conversations: HashMap<String, Vec<Message>> = HashMap::new();

    // Use short poll timeout when multiple connectors are active
    let poll_timeout = if connectors.len() > 1 { 2 } else { 30 };

    eprintln!(
        "sentinel: started with {} connector(s), polling...",
        connectors.len()
    );

    loop {
        let mut had_messages = false;

        for i in 0..connectors.len() {
            let updates = match connectors[i].poll_messages(poll_timeout) {
                Ok(msgs) => msgs,
                Err(e) => {
                    eprintln!(
                        "sentinel: {} poll error: {}",
                        connectors[i].platform_name(),
                        e
                    );
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            };

            if !updates.is_empty() {
                had_messages = true;
            }

            for msg in updates {
                let platform = connectors[i].platform_name();
                let username = msg.username.as_deref().unwrap_or("unknown");

                auditor.log(AuditEvent::MessageReceived {
                    chat_id: msg.channel_id.parse::<i64>().unwrap_or(0),
                    user_id: msg.user_id.parse::<i64>().unwrap_or(0),
                    username,
                });

                // Authorization check
                if !is_authorized(&config, platform, &msg.user_id) {
                    auditor.log(AuditEvent::UnauthorizedUser {
                        user_id: msg.user_id.parse::<i64>().unwrap_or(0),
                        username,
                    });
                    let _ = connectors[i].send_message(&msg.channel_id, "Unauthorized.");
                    continue;
                }

                let conv_key = format!("{}:{}", platform, msg.channel_id);

                // Handle /clear command
                if msg.text.trim() == "/clear" {
                    conversations.remove(&conv_key);
                    let _ = connectors[i]
                        .send_message(&msg.channel_id, "Conversation cleared.");
                    continue;
                }

                // Get or create conversation history
                let history = conversations.entry(conv_key).or_default();

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
                    &*connectors[i],
                    &msg.channel_id,
                ) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("sentinel: agent error: {}", e);
                        let error_msg = format!("Error: {}", e);
                        let _ = connectors[i].send_message(&msg.channel_id, &error_msg);
                    }
                }

                // Trim history if too long
                if history.len() > MAX_HISTORY_MESSAGES {
                    let drain_count = history.len() - MAX_HISTORY_MESSAGES;
                    history.drain(..drain_count);
                }
            }
        }

        // For HTTP-polling connectors without long-poll, avoid tight loops
        if !had_messages && connectors.len() > 1 {
            thread::sleep(Duration::from_secs(1));
        }
    }
}

fn is_authorized(config: &Config, platform: &str, user_id: &str) -> bool {
    match platform {
        "telegram" => {
            if config.telegram_allowed_users.is_empty() {
                return true;
            }
            if let Ok(id) = user_id.parse::<i64>() {
                config.telegram_allowed_users.contains(&id)
            } else {
                false
            }
        }
        "discord" => {
            config.discord_allowed_users.is_empty()
                || config.discord_allowed_users.iter().any(|u| u == user_id)
        }
        "slack" => {
            config.slack_allowed_users.is_empty()
                || config.slack_allowed_users.iter().any(|u| u == user_id)
        }
        _ => false,
    }
}

fn run_agent_turn(
    llm: &dyn LlmProvider,
    history: &mut Vec<Message>,
    config: &Config,
    tool_defs: &[ToolDef],
    tool_executor: &ToolExecutor,
    auditor: &mut Auditor,
    connector: &dyn Connector,
    channel_id: &str,
) -> Result<(), String> {
    let system = config.system_prompt.as_deref();

    for _round in 0..MAX_TOOL_ROUNDS {
        // Streaming state for real-time message updates
        let mut streamed_text = String::new();
        let mut platform_msg_id: Option<String> = None;
        let mut last_edit = Instant::now();

        let api_resp = {
            let streamed_text_ref = &mut streamed_text;
            let platform_msg_id_ref = &mut platform_msg_id;
            let last_edit_ref = &mut last_edit;

            let mut on_text = |delta: &str| {
                streamed_text_ref.push_str(delta);

                // Send/edit message periodically (every 500ms)
                let should_update = last_edit_ref.elapsed() >= Duration::from_millis(500);
                if !should_update {
                    return;
                }

                if let Some(ref msg_id) = *platform_msg_id_ref {
                    let _ =
                        connector.edit_message_text(channel_id, msg_id, streamed_text_ref);
                } else if streamed_text_ref.len() >= 10 {
                    // Wait for at least 10 chars before sending initial message
                    match connector.send_message_get_id(channel_id, streamed_text_ref) {
                        Ok(id) => *platform_msg_id_ref = Some(id),
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

                // Send final text via connector
                if let Some(ref msg_id) = platform_msg_id {
                    // Edit with final complete text
                    let _ = connector.edit_message_text(channel_id, msg_id, &text);
                } else {
                    // No streaming happened (or very short response) — send normally
                    if let Err(e) = connector.send_message(channel_id, &text) {
                        eprintln!("sentinel: failed to send message: {}", e);
                    }
                }
                return Ok(());
            }
            StopReason::ToolUse => {
                // If we streamed partial text, finalize it
                if let Some(ref msg_id) = platform_msg_id {
                    let text = extract_text(&api_resp.content);
                    if !text.is_empty() {
                        let _ = connector.edit_message_text(channel_id, msg_id, &text);
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
