# Contributing to LuperIQ Sentinel

Sentinel is MIT licensed. No CLA required. Send a PR.

## Setup

```bash
git clone https://github.com/LuperIQ/luperiq-sentinel.git
cd luperiq-sentinel
cargo build   # should compile clean
cargo test    # should pass all tests
```

You need Rust stable (2021 edition). The correct version is specified in `Cargo.toml`.

## Project Structure

```
src/
├── main.rs              # Entry point
├── app.rs               # Multi-connector agent loop, conversation management
├── config.rs            # TOML parser + env var config
├── net/
│   ├── json.rs          # JSON parser/serializer
│   ├── http.rs          # HTTPS client with connection pooling
│   └── sse.rs           # Server-Sent Events parser
├── llm/
│   ├── provider.rs      # LlmProvider trait
│   ├── anthropic.rs     # Anthropic Messages API (streaming)
│   └── openai.rs        # OpenAI-compatible API
├── messaging/
│   ├── mod.rs           # Connector trait
│   ├── telegram.rs      # Telegram Bot API
│   ├── discord.rs       # Discord REST API
│   └── slack.rs         # Slack Web API
├── agent/
│   └── tools.rs         # Tool definitions + execution (with timeout)
├── platform/
│   ├── mod.rs           # Platform trait (Linux vs LuperIQ OS)
│   ├── linux.rs         # std-based backend
│   └── luperiq.rs       # Kernel syscall backend
├── security/
│   ├── capability.rs    # Path/command allowlists
│   ├── audit.rs         # JSON-line audit logging
│   └── linux.rs         # seccomp BPF + Landlock
└── skills/
    ├── mod.rs           # SkillRunner orchestrator
    ├── manifest.rs      # skill.toml parser
    ├── loader.rs        # Directory-based skill discovery
    ├── sandbox.rs       # Subprocess sandboxing
    └── ipc.rs           # JSON-line IPC with timeout
```

The whole codebase is ~3,500 lines across 29 files. You can read all of it in an afternoon.

## Conventions

### Dependencies

We have 2 crate dependencies: `rustls` and `webpki-roots`. That's it. Do not add dependencies without discussion.

If you need JSON parsing, use `crate::net::json`. If you need HTTP, use `crate::net::http`. If you need config values, use `crate::config`. These exist specifically so we don't need external crates.

If you genuinely need a new dependency, open an issue explaining why the from-scratch approach isn't feasible. The bar is high — every dependency is attack surface for an agent runtime.

### Error Handling

Each module has its own error enum. No global error type, no `anyhow`, no `thiserror`. Example:

```rust
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
```

### Code Style

- No `unsafe`
- No `unwrap()` in non-test code (use `?` or handle the error)
- All logging to stderr via `eprintln!("sentinel: ...")`
- Tests go in a `#[cfg(test)] mod tests` block at the bottom of each file
- Keep functions short. If a function is over 50 lines, consider splitting it.

### Adding a New LLM Provider

1. Create `src/llm/your_provider.rs`
2. Implement the `LlmProvider` trait from `src/llm/provider.rs` (see `anthropic.rs` and `openai.rs` for examples)
3. Use `crate::net::http::HttpClient` for API calls
4. Build request JSON using `crate::net::json::json_obj()` builder
5. Parse response JSON using `crate::net::json::parse()`
6. Add `pub mod your_provider;` to `src/llm/mod.rs`
7. Add provider selection in `src/app.rs` based on config

### Adding a New Messaging Platform

1. Create `src/messaging/your_platform.rs`
2. Implement the `Connector` trait from `src/messaging/mod.rs` (5 methods: `poll_messages`, `send_message`, `send_message_get_id`, `edit_message_text`, `platform_name`)
3. Use `crate::net::http::HttpClient` for API calls
4. Add `pub mod your_platform;` to `src/messaging/mod.rs`
5. Add connector initialization in `src/app.rs` based on config

### Adding a New Tool

1. Add a `ToolDef` to the `tool_definitions()` vec in `src/agent/tools.rs`
2. Add a match arm in the `execute()` method
3. Implement the tool function following the pattern of existing tools:
   - Extract parameters from `JsonValue`
   - Check capabilities via `self.caps`
   - Log via auditor
   - Return `Ok(output)` or `Err(error_message)`

## Testing

```bash
cargo test              # Run all tests
cargo test net::json    # Run only JSON tests
cargo test config       # Run only config tests
```

To test the full agent loop, you need real API keys:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export TELEGRAM_BOT_TOKEN="123456:ABC..."
cargo run
```

Then message your Telegram bot.

## What Needs Work

See [STATUS.md](STATUS.md) for the full roadmap. High-impact contributions:

### Good First Issues

- **Signal connector** — `src/messaging/signal.rs`, implements `Connector` trait, Signal CLI JSON-RPC bridge
- **Matrix connector** — `src/messaging/matrix.rs`, Matrix client-server API
- **Config validation** — Warn about common misconfigurations (empty API key, unreachable paths)
- **More tests** — Skills edge cases, multi-connector scenarios, platform abstraction
- **Streaming for Discord/Slack** — Live message editing as tokens arrive (already works for Telegram)

### Medium Issues

- **Conversation persistence** — Save/load conversation history across restarts
- **Ollama auto-detection** — Detect local Ollama instance and offer as default provider
- **Skill examples** — Reference skill implementations for common tasks (web search, code execution)

### Big Projects

- **WebSocket control plane** — OpenClaw protocol compatibility, needs from-scratch WebSocket framing
- **Web-based permission dashboard** — Approve/deny capabilities from a browser UI
- **Multi-agent support** — Run multiple independent agents with different configs

## Questions?

Open an issue on GitHub. We're building this in the open.
