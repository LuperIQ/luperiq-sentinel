# LuperIQ Sentinel — Development Status

## Overview

Sentinel is being built incrementally. The MVP is a working Claude-to-Telegram agent with capability-checked tool execution. Each phase adds a new capability while keeping the codebase small and auditable.

**Current version:** 0.1.0 (February 2026)
**Total source:** ~2,700 lines of Rust across 14 files
**Dependencies:** 2 crates (rustls 0.23, webpki-roots 0.26)
**Tests:** 12 passing

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

- No streaming (responses arrive all at once after Claude finishes)
- Only Anthropic/Claude — no OpenAI, no other providers
- Only Telegram — no Discord, Slack, Signal, or WebSocket
- No Linux-level sandboxing (seccomp, landlock, namespaces)
- No skill/plugin system
- No web dashboard
- run_command has no timeout (a long-running command will block)
- No connection pooling (new TLS connection per request)

---

## Phase 2: Streaming and Multi-Provider (Not Started)

**Goal:** Real-time token streaming and support for multiple LLM providers.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| SSE parser | `src/net/sse.rs` | ~150 lines | Parse `text/event-stream` for Claude streaming |
| Streaming Anthropic | `src/llm/anthropic.rs` | Modify | Use `stream: true`, parse delta events |
| OpenAI provider | `src/llm/openai.rs` | ~250 lines | Chat Completions API, tool calls, streaming |
| Provider trait | `src/llm/mod.rs` | ~50 lines | Common trait for LLM providers |
| Command timeout | `src/agent/tools.rs` | Modify | Kill subprocess after configurable timeout |

**Why this matters:** Streaming makes the bot feel responsive — users see tokens arrive in real-time instead of waiting 10-30 seconds for a complete response. Multi-provider lets users choose their LLM.

---

## Phase 3: More Messaging Platforms (Not Started)

**Goal:** Discord and Slack connectors.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| Discord connector | `src/messaging/discord.rs` | ~300 lines | Gateway WebSocket + REST API, slash commands |
| Slack connector | `src/messaging/slack.rs` | ~250 lines | Web API + Events API (or Socket Mode) |
| Connector trait | `src/messaging/mod.rs` | ~50 lines | Common trait for messaging platforms |
| Signal connector | `src/messaging/signal.rs` | ~200 lines | Signal CLI bridge via JSON-RPC |

**Why this matters:** Most teams use Discord or Slack. Supporting them makes Sentinel useful to a much wider audience.

---

## Phase 4: WebSocket Control Plane (Not Started)

**Goal:** OpenClaw protocol compatibility for management tools.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| WebSocket framing | `src/net/websocket.rs` | ~300 lines | Frame parser/builder, masking, ping/pong |
| Control server | `src/net/ws_server.rs` | ~200 lines | Accept connections on localhost, handle upgrade |
| Protocol handler | `src/messaging/websocket.rs` | ~250 lines | OpenClaw message format, state sync |

**Why this matters:** Existing OpenClaw management dashboards and tools can connect to Sentinel without modification.

---

## Phase 5: Linux Sandboxing (Not Started)

**Goal:** OS-level process isolation on Linux.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| seccomp filters | `src/security/linux.rs` | ~200 lines | Restrict available syscalls |
| Landlock rules | `src/security/linux.rs` | ~150 lines | Filesystem access restrictions (Linux 5.13+) |
| Sandbox wrapper | `src/security/sandbox.rs` | ~100 lines | Apply seccomp + landlock before entering main loop |

**Why this matters:** Application-level capability checks can be bypassed if there's a bug in Sentinel. seccomp/landlock are kernel-enforced and cannot be bypassed from userspace.

---

## Phase 6: Skill/Plugin System (Not Started)

**Goal:** Run third-party tools in sandboxed subprocesses.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| Skill manifest | `src/skills/manifest.rs` | ~100 lines | Parse skill.toml declaring required capabilities |
| Skill loader | `src/skills/loader.rs` | ~150 lines | Discover and load skills from directory |
| Skill sandbox | `src/skills/sandbox.rs` | ~200 lines | Fork subprocess with restricted capabilities |
| IPC channel | `src/skills/ipc.rs` | ~150 lines | JSON-line communication with skill process |

**Why this matters:** Skills are how agents become useful — web search, code execution, database queries. The plugin system lets third parties add capabilities without modifying Sentinel's core.

---

## Phase 7: LuperIQ OS Integration (Not Started)

**Goal:** Wire up kernel capability handles for hard security enforcement.

| Task | File | Effort | Description |
|------|------|--------|-------------|
| Capability syscalls | `src/security/luperiq.rs` | ~200 lines | Interface to LuperIQ kernel capability system |
| Handle management | `src/security/capability.rs` | Modify | Use kernel handles instead of application-level checks |
| Audit integration | `src/security/audit.rs` | Modify | Delegate to kernel audit log |
| OS detection | `src/security/mod.rs` | ~50 lines | Detect LuperIQ vs Linux, select security backend |

**Why this matters:** This is the whole point. On LuperIQ OS, the agent process literally cannot access resources outside its capability set. No application bug can bypass kernel enforcement.

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
