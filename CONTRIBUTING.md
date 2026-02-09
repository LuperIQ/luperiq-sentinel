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
├── main.rs              # Entry point and agent loop
├── config.rs            # TOML parser + env var config
├── net/
│   ├── json.rs          # JSON parser/serializer
│   └── http.rs          # HTTPS client over rustls
├── llm/
│   └── anthropic.rs     # Anthropic Messages API
├── messaging/
│   └── telegram.rs      # Telegram Bot API
├── agent/
│   └── tools.rs         # Tool definitions + execution
└── security/
    ├── capability.rs    # Path/command allowlists
    └── audit.rs         # JSON-line audit logging
```

The whole codebase is ~2,700 lines. You can read all of it in an afternoon.

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
2. Use the same `ContentBlock`, `Message`, `Role` types from `anthropic.rs` (we'll extract a common trait when we have 2+ providers)
3. Implement the API call using `crate::net::http::HttpClient`
4. Build request JSON using `crate::net::json::json_obj()` builder
5. Parse response JSON using `crate::net::json::parse()`
6. Add `pub mod your_provider;` to `src/llm/mod.rs`
7. Wire it up in `main.rs` based on config

### Adding a New Messaging Platform

1. Create `src/messaging/your_platform.rs`
2. Implement polling/receiving messages and sending responses
3. Use `crate::net::http::HttpClient` for API calls
4. Add `pub mod your_platform;` to `src/messaging/mod.rs`
5. Wire it up in `main.rs` based on config

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

- **Add OpenAI provider** — Similar to `anthropic.rs`, ~250 lines. The Chat Completions API uses a similar tool_calls model.
- **Add streaming** — Parse SSE events from Claude's streaming endpoint. Makes the bot feel responsive.
- **Add command timeout** — `run_command` currently has no timeout. Use `std::process::Command` with a spawned thread and timeout.
- **More JSON tests** — Edge cases like deeply nested objects, empty arrays, unicode, large numbers.
- **Config validation** — Warn about common misconfigurations (empty API key, unreachable paths).

### Medium Issues

- **Discord connector** — Discord Gateway (WebSocket) + REST API. ~300 lines.
- **Slack connector** — Slack Web API + Socket Mode. ~250 lines.
- **Connection reuse** — Keep TLS connections alive for repeated API calls to the same host.

### Big Projects

- **seccomp/landlock** — Linux kernel-level sandboxing. Significant but well-documented.
- **Skill system** — Plugin architecture with subprocess isolation. Needs design discussion first.
- **WebSocket server** — For OpenClaw protocol compatibility. Needs from-scratch WebSocket framing.

## Questions?

Open an issue on GitHub. We're building this in the open.
