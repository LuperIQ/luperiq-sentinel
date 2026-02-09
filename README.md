# LuperIQ Sentinel

**Secure AI Agent Runtime — Built from Scratch in Rust**

LuperIQ Sentinel is a Rust-native AI agent runtime that connects Claude to Telegram with capability-based security. It replaces the Node.js/Chromium stack used by projects like OpenClaw with ~2,700 lines of Rust and only 2 crate dependencies. When running on [LuperIQ OS](https://github.com/LuperIQ/luperiq-kernel), the kernel enforces security boundaries that software alone cannot.

## Current Status: MVP Working

The MVP is implemented and compiles. It connects to Claude via the Anthropic Messages API, communicates with users via Telegram Bot API, and executes tools (file read/write, directory listing, command execution) with capability-checked security boundaries and JSON-line audit logging.

```
cargo build    # compiles clean
cargo test     # 12 tests pass
cargo run      # starts polling Telegram
```

### What's Built (v0.1.0)

| Component | Status | Lines |
|-----------|--------|-------|
| JSON parser/serializer | Done | 681 |
| HTTPS client (rustls) | Done | 345 |
| Anthropic Messages API client | Done | 318 |
| Telegram Bot API client | Done | 201 |
| TOML config loader | Done | 317 |
| Capability checker (path/command allowlists) | Done | 143 |
| Audit logger (JSON-line) | Done | 86 |
| Tool executor (4 tools) | Done | 335 |
| Main agent loop | Done | 228 |
| **Total** | **Working MVP** | **~2,700** |

### What's Next

See [STATUS.md](STATUS.md) for the full development roadmap. The short version:

| Priority | Feature | Effort | Status |
|----------|---------|--------|--------|
| 1 | Streaming responses (SSE) | Medium | Not started |
| 2 | Discord connector | Medium | Not started |
| 3 | Slack connector | Medium | Not started |
| 4 | OpenAI/GPT provider | Small | Not started |
| 5 | WebSocket control plane | Medium | Not started |
| 6 | seccomp/landlock sandboxing (Linux) | Large | Not started |
| 7 | LuperIQ OS capability integration | Large | Not started |
| 8 | Skill/plugin system | Large | Not started |
| 9 | Signal connector | Medium | Not started |
| 10 | Web-based permission dashboard | Large | Not started |

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

### Sentinel: 144KB, 2,700 Lines, 14 Files

| | OpenClaw | Sentinel |
|---|---|---|
| Language | TypeScript | Rust |
| Source lines | 136,689 | 2,661 |
| Source files | 2,631 | 14 |
| Source size on disk | 57 MB | 144 KB |
| Runtime dependencies | 1,200+ npm packages | 2 crates (rustls, webpki-roots) |
| Memory at runtime | 300 MB - 2 GB | ~20 MB |
| Binary size | ~200 MB (node + deps) | ~5 MB |
| Platforms | Desktop, iOS, macOS, CLI | CLI (Linux first) |
| Messaging | Telegram, Discord, Slack, Web, browser | Telegram (MVP) |
| Security model | Docker (opt-in, already bypassed) | Capability allowlists (mandatory) |
| JSON | V8 built-in | From-scratch parser (681 lines) |
| HTTP/TLS | Node.js + OpenSSL | From-scratch HTTP + rustls |
| Config | YAML + many npm packages | From-scratch TOML parser (100 lines) |

Sentinel is **~400x smaller** because it does one thing well (connect an LLM to messaging with security) instead of trying to be a full-stack multi-platform application with a marketplace, browser automation, and desktop apps.

The point is not that OpenClaw is bad software. It does a lot. The point is that **for running an AI agent securely, most of that isn't necessary**, and every line of code is attack surface.

## Quick Start

### Prerequisites

- Rust (stable, 2021 edition)
- An Anthropic API key
- A Telegram bot token (from [@BotFather](https://t.me/BotFather))

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
├── main.rs              # Entry point, agent loop, conversation management
├── config.rs            # TOML parser + env var config loading
├── net/
│   ├── json.rs          # JSON parser/serializer (recursive descent, builder pattern)
│   └── http.rs          # HTTPS client over rustls (TLS 1.3)
├── llm/
│   └── anthropic.rs     # Anthropic Messages API (tool use, content blocks)
├── messaging/
│   └── telegram.rs      # Telegram Bot API (long polling, message splitting)
├── agent/
│   └── tools.rs         # Tool definitions + execution (4 tools)
└── security/
    ├── capability.rs    # Path/command allowlist with canonicalization
    └── audit.rs         # JSON-line audit logging to stderr + file
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

On LuperIQ OS (future), these checks will be enforced by the kernel via capability handles — the agent process literally won't have the ability to access resources outside its grant. On Linux, these are application-level checks.

## The Vision

Sentinel on Linux is step one. The full security story requires [LuperIQ OS](https://github.com/LuperIQ/luperiq-kernel):

| Layer | Linux (current) | LuperIQ OS (planned) |
|-------|----------------|---------------------|
| Capability enforcement | Application-level allowlists | Kernel-enforced handle tables |
| Audit log | File-based (agent-managed) | Kernel-managed (immutable) |
| Skill sandboxing | Not yet implemented | Separate processes with restricted handle tables |
| Network isolation | Not yet implemented | Per-host capability handles |
| Config protection | File permissions | Capability-gated writes |
| Approval UI | Not yet implemented | Native OS dialog |

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

- **Add an OpenAI/GPT provider** — `src/llm/openai.rs`, similar structure to `anthropic.rs`
- **Add streaming support** — Parse SSE events from Claude's streaming API
- **Add Discord connector** — `src/messaging/discord.rs`, Discord Gateway + REST API
- **Improve error messages** — Better user-facing errors when config is wrong
- **Add more tests** — JSON parser edge cases, config parsing, capability checks

Bigger projects:

- **seccomp/landlock sandboxing** — Linux-level process isolation
- **WebSocket control plane** — OpenClaw protocol compatibility
- **Skill/plugin system** — Run third-party tools in sandboxed subprocesses
- **LuperIQ OS integration** — Wire up kernel capability handles

## License

MIT License. See [LICENSE](LICENSE) for details.

The full security story (OS-enforced capabilities, immutable audit log, kernel-level isolation) requires [LuperIQ OS](https://github.com/LuperIQ/luperiq-kernel), which is dual-licensed under GPLv3 + Commercial.

## Links

- **LuperIQ OS** — https://github.com/LuperIQ/luperiq-kernel
- **Architecture docs** — [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Security model** — [docs/SECURITY.md](docs/SECURITY.md)
- **Development status** — [STATUS.md](STATUS.md)
- **Website** — https://luperiq.com
- **Commercial Licensing** — licensing@luperiq.com
