# LuperIQ Sentinel

**Secure AI Agent Runtime — Built from Scratch in Rust**

LuperIQ Sentinel is a Rust-native AI agent runtime that connects LLMs to messaging platforms with capability-based security. It replaces the Node.js/Chromium stack used by projects like OpenClaw with ~3,500 lines of Rust and only 2 crate dependencies. When running on [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os), the kernel enforces security boundaries that software alone cannot.

## Current Status: v0.2.0 — Full-Featured Agent Runtime

The agent runtime is fully functional with multi-provider LLM support, multi-platform messaging, Linux sandboxing, and a skill/plugin system. ~3,500 lines of Rust, 29 source files, 63 tests.

```
cargo build    # compiles clean
cargo test     # 63 tests pass
cargo run      # starts polling configured messaging platforms
```

### What's Built

| Component | Status | Description |
|-----------|--------|-------------|
| JSON parser/serializer | Done | Recursive descent, builder pattern, unicode escapes |
| HTTPS client (rustls) | Done | HTTP/1.1, keep-alive, TLS stream caching, chunked encoding |
| SSE parser | Done | Server-Sent Events for streaming responses |
| Anthropic Messages API | Done | Streaming (SSE), tool use, content blocks |
| OpenAI-compatible API | Done | Chat Completions, tool calls, works with Ollama/vLLM/LM Studio |
| LLM Provider trait | Done | Common interface for any LLM backend |
| Telegram connector | Done | Long polling, message editing for streaming, 4096-char split |
| Discord connector | Done | REST API v10 polling, rate limiting, 2000-char split |
| Slack connector | Done | Web API polling, bot detection, chronological ordering |
| Connector trait | Done | Common interface for all messaging platforms |
| Multi-connector support | Done | Round-robin polling, per-platform auth, conversation keying |
| TOML config loader | Done | Parser + env var fallback, section/array support |
| Capability checker | Done | Path canonicalization, prefix matching, command allowlists |
| Audit logger (JSON-line) | Done | Events to stderr + optional file |
| Tool executor (4 tools) | Done | read_file, write_file, list_directory, run_command (with timeout) |
| seccomp BPF sandbox | Done | ~80 syscall allowlist, architecture verification |
| Landlock filesystem rules | Done | Read/write/execute path restrictions (Linux 5.13+) |
| Skill manifest parser | Done | skill.toml with capabilities + parameters |
| Skill loader | Done | Directory-based discovery and validation |
| Skill sandbox | Done | Fork subprocess, env_clear, piped stdio, Drop cleanup |
| Skill IPC | Done | JSON-line stdin/stdout with timeout + kill |
| Platform abstraction | Done | Linux (std) and LuperIQ OS (kernel syscall) backends |
| App orchestrator | Done | Multi-connector agent loop with conversation management |
| **Total** | **~3,500 lines** | **29 files, 63 tests, 2 dependencies** |

## Why This Exists

### OpenClaw: 57MB, 137,000 Lines, 5,138 Files

OpenClaw is the most popular open-source AI agent framework. It's also a security disaster:

- **40,000+ instances** exposed on the public internet, 12,800 vulnerable to RCE
- **3 critical CVEs** in two weeks (token theft, Docker escape, SSH injection)
- **341 malicious skills** in the marketplace (7-26% of all skills)
- **Persistent AI backdoors** via SOUL.md prompt injection
- **1.5 million API tokens leaked** from the Moltbook social network

CrowdStrike, Cisco, Palo Alto Networks, and Bitdefender have all published enterprise security advisories. The root cause is architectural: **the agent runs with full user permissions on a general-purpose OS** and the entire stack is built on Node.js with 1,200+ npm dependencies.

### Where All That Size Goes

We looked at OpenClaw's codebase to understand the bloat:

| Component | Size | What It Is |
|-----------|------|-----------|
| `src/` (TypeScript) | 21 MB | 136,689 lines across 2,631 files — agents, gateway, CLI, config, browser automation, Discord, Slack, memory, plugins, auto-reply, commands, channels, web UI |
| `docs/` | 14 MB | Documentation, screenshots, Chinese translation, showcase PNGs |
| `apps/` | 11 MB | macOS app wrapper (.icns icon alone is 1.8MB), iOS app, desktop packaging |
| `extensions/` | 5 MB | Twitch, LanceDB, voice-call, task planning plugins |
| `vendor/` | 2 MB | Vendored dependencies |
| `ui/` | 1.6 MB | Frontend dashboard |
| Image assets | 15 MB | App icons, DMG backgrounds, screenshots |
| **Total** | **57 MB** | **Before `npm install` adds node_modules** |

### Sentinel: ~3,500 Lines, 29 Files

| | OpenClaw | Sentinel |
|---|---|---|
| Language | TypeScript | Rust |
| Source lines | 136,689 | ~3,500 |
| Source files | 2,631 | 29 |
| Source size on disk | 57 MB | ~200 KB |
| Runtime dependencies | 1,200+ npm packages | 2 crates (rustls, webpki-roots) |
| Memory at runtime | 300 MB - 2 GB | ~20 MB |
| Binary size | ~200 MB (node + deps) | ~5 MB |
| Platforms | Desktop, iOS, macOS, CLI | CLI (Linux, LuperIQ OS) |
| LLM providers | OpenAI, Anthropic, etc. | Anthropic + OpenAI-compatible (Ollama, vLLM, LM Studio) |
| Messaging | Telegram, Discord, Slack, Web, browser | Telegram, Discord, Slack |
| Security model | Docker (opt-in, already bypassed) | seccomp + Landlock (Linux), kernel capabilities (LuperIQ OS) |
| JSON | V8 built-in | From-scratch parser (681 lines) |
| HTTP/TLS | Node.js + OpenSSL | From-scratch HTTP + rustls, connection pooling |
| Config | YAML + many npm packages | From-scratch TOML parser (~100 lines) |
| Plugins | ClawHub marketplace (341 malicious skills found) | Sandboxed subprocess IPC with declared capabilities |

Sentinel is **~40x smaller** because it does one thing well (connect an LLM to messaging with security) instead of trying to be a full-stack multi-platform application with a marketplace, browser automation, and desktop apps.

The point is not that OpenClaw is bad software. It does a lot. The point is that **for running an AI agent securely, most of that isn't necessary**, and every line of code is attack surface.

## Quick Start

### Prerequisites

- Rust (stable, 2021 edition)
- At least one LLM API key (Anthropic, OpenAI, or compatible)
- At least one messaging platform token (Telegram, Discord, or Slack)

### Build and Run

```bash
git clone https://github.com/LuperIQ/luperiq-sentinel.git
cd luperiq-sentinel

# Set required environment variables
export ANTHROPIC_API_KEY="sk-ant-..."
export TELEGRAM_BOT_TOKEN="123456:ABC..."

# Optional: restrict which Telegram users can talk to the bot
export SENTINEL_ALLOWED_USERS="123456789,987654321"

# Optional: allow the agent to read/write files and run commands
export SENTINEL_READ_PATHS="/tmp,/home/user/projects"
export SENTINEL_WRITE_PATHS="/tmp"
export SENTINEL_COMMANDS="ls,cat,echo,date,wc"

cargo run
```

Or use a config file — copy `sentinel.toml.example` to `sentinel.toml` and customize.

### What You Can Do

Once running, message your Telegram bot:

- **Chat normally** — Claude responds via the Anthropic API
- **"List the files in /tmp"** — Claude calls the `list_directory` tool
- **"Read the file /tmp/notes.txt"** — Claude calls `read_file` (if /tmp is in allowed paths)
- **"What's today's date?"** — Claude calls `run_command` with `date` (if allowed)
- **"/clear"** — Resets conversation history

Any attempt to access paths or commands outside the allowlist is denied and logged.

## Architecture

```
src/
├── main.rs              # Entry point
├── app.rs               # Multi-connector agent loop, conversation management
├── config.rs            # TOML parser + env var config loading
├── net/
│   ├── json.rs          # JSON parser/serializer (recursive descent, builder pattern)
│   ├── http.rs          # HTTPS client, keep-alive connection pooling
│   └── sse.rs           # Server-Sent Events parser (streaming responses)
├── llm/
│   ├── provider.rs      # LlmProvider trait + shared types
│   ├── anthropic.rs     # Anthropic Messages API (streaming, tool use)
│   └── openai.rs        # OpenAI Chat Completions (compatible with Ollama/vLLM)
├── messaging/
│   ├── mod.rs           # Connector trait, IncomingMessage, ConnectorError
│   ├── telegram.rs      # Telegram Bot API (long polling, live editing)
│   ├── discord.rs       # Discord REST API v10 (polling, rate limiting)
│   └── slack.rs         # Slack Web API (polling, bot detection)
├── agent/
│   └── tools.rs         # Tool definitions + execution (4 tools, configurable timeout)
├── platform/
│   ├── mod.rs           # Platform trait (8 operations)
│   ├── linux.rs         # Linux backend (std::fs, std::process, std::net)
│   └── luperiq.rs       # LuperIQ OS backend (kernel syscalls)
├── security/
│   ├── capability.rs    # Path/command allowlist with canonicalization
│   ├── audit.rs         # JSON-line audit logging to stderr + file
│   └── linux.rs         # seccomp BPF + Landlock filesystem rules
└── skills/
    ├── mod.rs           # SkillRunner (load, execute, merge tool definitions)
    ├── manifest.rs      # skill.toml parser (capabilities + parameters)
    ├── loader.rs        # Directory-based skill discovery
    ├── sandbox.rs       # Forked subprocess with env_clear, piped stdio
    └── ipc.rs           # JSON-line stdin/stdout communication
```

### How the Agent Loop Works

```
1. Load config (TOML file or env vars)
2. Initialize: Auditor, CapabilityChecker, ToolExecutor, AnthropicClient, TelegramClient
3. Loop forever:
   a. Poll Telegram for new messages (30s long poll)
   b. For each message:
      - Check user authorization
      - Handle /clear command
      - Add user message to conversation history
      - Send history + tool definitions to Claude
      - If Claude returns text → send to Telegram
      - If Claude wants to use tools → execute each tool
        (with capability check) → send results back to Claude → repeat
      - Max 10 tool rounds per turn (prevents infinite loops)
      - Split response at 4096 chars (Telegram limit)
      - Trim history at 40 messages
   c. On error: log, wait 5s, continue
```

### Security Model

The MVP implements **allowlist-based capability checking**:

- **File read/write**: paths are canonicalized (resolves `../` traversal) and checked against configured prefixes
- **Command execution**: command names checked against an explicit allowlist
- **All tool calls**: logged as JSON-line audit events (allowed and denied)
- **User authorization**: Telegram user IDs checked against allowlist (empty = allow all)

On [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os) (future), these checks will be enforced by the kernel via capability handles — the agent process literally won't have the ability to access resources outside its grant. On Linux, these are application-level checks.

## The Vision

Sentinel on Linux is step one. The full security story requires [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os):

| Layer | Linux (current) | LuperIQ OS (working) |
|-------|----------------|---------------------|
| Capability enforcement | seccomp + Landlock + application allowlists | Kernel-enforced handle tables (FilePolicy, SpawnPolicy per Job) |
| Audit log | File-based (agent-managed, append-only via Landlock) | Kernel-managed (immutable, SHA-256 hash chain) |
| Skill sandboxing | Forked subprocess, inherits seccomp/Landlock | Separate processes with restricted handle tables |
| Network isolation | Landlock path rules | Per-host capability handles |
| Config protection | File permissions + Landlock | Capability-gated writes |
| Approval UI | Config-driven auto-approve/deny | Web management UI (dashboard, capability manager, audit viewer) |

### The Agent Appliance

The end goal: a Raspberry Pi 5 on your desk running LuperIQ Agent OS with Sentinel pre-installed. Physically isolated hardware. OS-enforced capabilities. Full audit trail. Physical kill switch (unplug it). ~$80 total cost.

```
┌──────────────────────────┐
│   Raspberry Pi 5 (16GB)  │
│   ┌────────────────────┐ │
│   │  LuperIQ Agent OS  │ │
│   │  ┌──────────────┐  │ │
│   │  │   Sentinel    │  │ │
│   │  │   Runtime     │  │ │
│   │  └──────────────┘  │ │
│   └────────────────────┘ │
│                          │
│   Power  │  Ethernet     │
└────┬─────┴──────┬────────┘
     │            │
   [wall]    [your network]
```

## Contributing

**We want contributors.** Sentinel is MIT licensed, no CLA required. See [CONTRIBUTING.md](CONTRIBUTING.md) for how to get started.

Good first issues:

- **Signal connector** — `src/messaging/signal.rs`, Signal CLI bridge via JSON-RPC
- **Matrix connector** — `src/messaging/matrix.rs`, Matrix client-server API
- **Improve error messages** — Better user-facing errors when config is wrong
- **Config validation** — Warn about common misconfigurations
- **More tests** — Edge cases in skills, platform abstraction, multi-connector scenarios

Bigger projects:

- **WebSocket control plane** — OpenClaw protocol compatibility
- **Web-based permission dashboard** — Approve/deny capabilities from a browser
- **Streaming for Discord/Slack** — Live message editing as tokens arrive (already works for Telegram)
- **Conversation persistence** — Save/load conversation history across restarts

## Disclaimer

Sentinel is experimental software under active development. It is provided "as is" without warranty of any kind.

- **Not production-ready.** The capability checker and security model are functional but have not been independently audited.
- **Linux security is best-effort.** On Linux, security boundaries are enforced by application-level allowlists, not the OS kernel. A compromised Sentinel process could potentially bypass its own checks. The full security guarantee requires [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os).
- **Do not use for sensitive data** without your own security review. Sentinel connects to third-party APIs (Anthropic, Telegram) and processes user messages — evaluate the privacy implications for your use case.
- **Use at your own risk.** By using this software, you accept full responsibility for any consequences. See [DISCLAIMER.md](DISCLAIMER.md) for details.

We are building toward production quality and welcome security researchers, testers, and contributors who want to help us get there.

## License

MIT License. See [LICENSE](LICENSE) for details.

The full security story (OS-enforced capabilities, immutable audit log, kernel-level isolation) requires [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os), which is dual-licensed under GPLv3 + Commercial.

## Links

- **LuperIQ Agent OS** — https://github.com/LuperIQ/luperiq-agent-os
- **LuperIQ Kernel** — https://github.com/LuperIQ/luperiq-kernel
- **Architecture docs** — [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Security model** — [docs/SECURITY.md](docs/SECURITY.md)
- **Development status** — [STATUS.md](STATUS.md)
- **Website** — https://luperiq.com
- **Commercial Licensing** — licensing@luperiq.com
