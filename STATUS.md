# LuperIQ Sentinel — Development Status

## Overview

Sentinel is built incrementally. Each phase adds a new capability while keeping the codebase small and auditable.

**Current version:** 0.2.0 (February 2026)
**Total source:** ~3,500 lines of Rust across 29 files
**Dependencies:** 2 crates (rustls 0.23, webpki-roots 0.26)
**Tests:** 63 passing

---

## Phase 1: MVP (Complete)

The core agent loop works end to end.

### What's Built

| Component | File | Lines | Description |
|-----------|------|-------|-------------|
| JSON parser | `src/net/json.rs` | 681 | Recursive descent parser, compact serializer, builder pattern, unicode escapes, surrogate pairs |
| HTTPS client | `src/net/http.rs` | 345 | TLS 1.3 via rustls, URL parsing, chunked transfer encoding, Content-Length, 30s timeout |
| Anthropic client | `src/llm/anthropic.rs` | 318 | Messages API, tool_use/end_turn handling, content block parsing, rate limit retry |
| Telegram client | `src/messaging/telegram.rs` | 201 | Long polling, message sending, 4096-char splitting, update offset tracking |
| Config loader | `src/config.rs` | 317 | Minimal TOML parser, env var fallback, secret indirection (api_key_env) |
| Capability checker | `src/security/capability.rs` | 143 | Path canonicalization, prefix matching, command allowlist |
| Audit logger | `src/security/audit.rs` | 86 | JSON-line events to stderr + optional file |
| Tool executor | `src/agent/tools.rs` | 335 | 4 tools: read_file, write_file, list_directory, run_command |
| Main loop | `src/main.rs` | 228 | Telegram polling, conversation history, agent turn with tool loop |

### What Works

- Send a message to the Telegram bot, get a Claude response
- Claude can use tools (read files, list directories, run commands)
- Capability checker blocks access to paths/commands not in the allowlist
- Audit log records all tool calls (allowed and denied)
- Conversation history persists per chat, /clear resets it
- Rate limiting: auto-retry on 429 with backoff
- Config via TOML file or pure environment variables

### Known Limitations

- No WebSocket control plane (OpenClaw protocol compatibility)
- No web-based permission dashboard
- No Signal or Matrix connectors
- Streaming only for Telegram (Discord/Slack get full responses)
- No conversation persistence across restarts
- Skill system requires external binaries (no dynamic loading)

---

## Phase 2: Streaming and Multi-Provider (Complete)

**Goal:** Real-time token streaming and support for multiple LLM providers.

- [x] SSE parser (`src/net/sse.rs`) — Server-Sent Events for streaming
- [x] Streaming Anthropic — `stream: true`, live Telegram message editing (500ms throttle)
- [x] OpenAI provider (`src/llm/openai.rs`) — Chat Completions API, tool calls
- [x] LlmProvider trait (`src/llm/provider.rs`) — Common interface with `send_streaming()` default
- [x] Command timeout — Kill subprocess after configurable timeout (default 30s)
- [x] Connection pooling — HTTP/1.1 keep-alive, TLS stream caching per host

---

## Phase 3: More Messaging Platforms (Complete)

**Goal:** Discord and Slack connectors.

- [x] Discord connector (`src/messaging/discord.rs`) — REST API v10 polling, rate limiting, 2000-char split
- [x] Slack connector (`src/messaging/slack.rs`) — Web API polling, bot detection, chronological ordering
- [x] Connector trait (`src/messaging/mod.rs`) — 5 methods: poll, send, send_get_id, edit, platform_name
- [x] Multi-connector support (`src/app.rs`) — Round-robin polling, per-platform auth, conversation keying

---

## Phase 4: WebSocket Control Plane (Not Started)

**Goal:** OpenClaw protocol compatibility for management tools.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| WebSocket framing | `src/net/websocket.rs` | ~300 lines | Frame parser/builder, masking, ping/pong |
| Control server | `src/net/ws_server.rs` | ~200 lines | Accept connections on localhost, handle upgrade |
| Protocol handler | `src/messaging/websocket.rs` | ~250 lines | OpenClaw message format, state sync |

---

## Phase 5: Linux Sandboxing (Complete)

**Goal:** OS-level process isolation on Linux.

- [x] seccomp BPF (`src/security/linux.rs`) — ~80 syscall allowlist, x86_64 architecture check, default DENY with EPERM
- [x] Landlock rules (`src/security/linux.rs`) — Read/write/execute path restrictions (Linux 5.13+), system paths (/etc/ssl, /proc/self)
- [x] Enabled by default — `--no-sandbox` to disable, graceful degradation on unsupported kernels

---

## Phase 6: Skill/Plugin System (Complete)

**Goal:** Run third-party tools in sandboxed subprocesses.

- [x] Skill manifest (`src/skills/manifest.rs`) — skill.toml parser with capabilities + parameters
- [x] Skill loader (`src/skills/loader.rs`) — Directory-based discovery, manifest validation
- [x] Skill sandbox (`src/skills/sandbox.rs`) — Forked subprocess, env_clear, piped stdio, Drop cleanup
- [x] IPC channel (`src/skills/ipc.rs`) — JSON-line stdin/stdout with timeout + kill
- [x] SkillRunner (`src/skills/mod.rs`) — Orchestrates loader/sandbox/IPC, merges tool definitions with built-in tools

---

## Phase 7: LuperIQ OS Integration (Complete)

**Goal:** Wire up kernel capability handles for hard security enforcement.

- [x] Platform abstraction (`src/platform/mod.rs`) — Platform trait with 8 operations
- [x] Linux backend (`src/platform/linux.rs`) — std::fs, std::process, std::net
- [x] LuperIQ OS backend (`src/platform/luperiq.rs`) — Kernel syscall interface
- [x] Sentinel runs as 87KB no_std binary on LuperIQ Agent OS
- [x] Full agent loop on kernel: config, Telegram polling, Claude API, tool execution
- [x] Kernel-enforced FilePolicy and SpawnPolicy per Job

---

## Design Decisions

### Why from-scratch JSON instead of serde?

serde_json pulls in 3 crates and ~15,000 lines of generated code. Our JSON parser is 681 lines, handles everything the Anthropic and Telegram APIs need, and keeps the dependency count at 2. For an agent runtime where every dependency is attack surface, this tradeoff makes sense.

### Why from-scratch TOML instead of the toml crate?

Same reasoning. The toml crate pulls in serde + several other crates. Our parser handles `[sections]`, `key = "value"`, `key = 123`, `key = ["a", "b"]`, and `# comments` in ~100 lines. That covers everything sentinel.toml needs.

### Why from-scratch HTTP instead of reqwest/hyper?

reqwest pulls in 50+ transitive dependencies including tokio (async runtime), hyper, http, h2, and more. Our HTTP client is 345 lines, does synchronous HTTPS with rustls, handles chunked encoding, and is sufficient for API calls. We don't need HTTP/2, connection pooling, or async — the agent makes one API call at a time.

### Why synchronous instead of async?

The agent processes one message at a time per chat. Telegram long polling blocks for 30 seconds, then we process any messages sequentially. There's no concurrency benefit from async here, and it would add tokio (~100 crates) to the dependency tree. If we need concurrency later (multiple chats in parallel), std::thread is sufficient.

### Why rustls instead of from-scratch TLS?

LuperIQ OS has its own TLS 1.3 implementation. On Linux, we use rustls because reimplementing TLS for a userspace application isn't worth the security risk — rustls is well-audited, pure Rust, and maintained by professionals. When Sentinel runs on LuperIQ OS, it will use the kernel's TLS instead.

---

## How to Help

Pick a phase and start building. The codebase is small enough to read in an afternoon. Every module follows the same pattern: types at the top, implementation in the middle, tests at the bottom.

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and conventions.
