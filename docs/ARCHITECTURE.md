# LuperIQ Sentinel — Architecture

## Overview

Sentinel is a Rust-native AI agent runtime designed to be secure by default. It replaces OpenClaw's Node.js stack with a zero-dependency Rust implementation that integrates with LuperIQ OS's capability-based security model.

## Design Principles

1. **Zero ambient authority** — The agent has no permissions by default. Every resource (file, network host, process) requires an explicit capability handle.
2. **Zero external dependencies** — No npm, no Chromium, no OpenSSL. Everything is either built from scratch or uses LuperIQ's existing implementations.
3. **Defense in depth** — On LuperIQ OS, the kernel enforces capabilities. On Linux, seccomp + landlock + namespaces provide best-effort isolation.
4. **Audit everything** — Every capability use is logged immutably by the kernel (on LuperIQ OS) or by the runtime (on Linux).
5. **Fail closed** — If a capability check fails, the operation is denied. No fallback to permissive mode.

## Component Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     CLI / Web UI                         │
│  sentinel start | sentinel config | sentinel grant       │
├─────────────────────────────────────────────────────────┤
│                    Agent Runtime                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Conversation Manager                             │   │
│  │  - System prompt + SOUL.md personality            │   │
│  │  - Tool definitions (what the agent can do)       │   │
│  │  - Context window management                      │   │
│  │  - Response parsing (tool calls, text)            │   │
│  └──────────────────────────────────────────────────┘   │
│                         │                                │
│              ┌──────────┴──────────┐                     │
│              ▼                     ▼                      │
│  ┌────────────────┐    ┌────────────────┐                │
│  │  LLM Client    │    │  Tool Executor │                │
│  │  - Anthropic    │    │  - Commands    │                │
│  │  - OpenAI       │    │  - File ops    │                │
│  │  - Streaming    │    │  - Web browse  │                │
│  └───────┬────────┘    └───────┬────────┘                │
│          │                     │                          │
├──────────┴─────────────────────┴──────────────────────────┤
│                  Security Layer                            │
│  ┌────────────────────────────────────────────────────┐   │
│  │  Capability Manager                                 │   │
│  │  - Handle table (what this agent can access)        │   │
│  │  - Approval queue (pending user confirmations)      │   │
│  │  - Audit log (every capability use recorded)        │   │
│  │                                                     │   │
│  │  LuperIQ: kernel capability handles (hard enforce)  │   │
│  │  Linux: seccomp + landlock + namespaces (best-effort)│  │
│  └────────────────────────────────────────────────────┘   │
├───────────────────────────────────────────────────────────┤
│                  Messaging Layer                           │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐    │
│  │ Telegram │ │ Discord  │ │  Slack   │ │WebSocket │    │
│  │ Bot API  │ │ Gateway  │ │ Web API  │ │ Control  │    │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘    │
│       └─────────────┴────────────┴─────────────┘          │
├───────────────────────────────────────────────────────────┤
│                  Network Layer                             │
│  HTTP Client │ WebSocket │ JSON Parser │ TLS              │
└───────────────────────────────────────────────────────────┘
```

## Agent Lifecycle

### 1. Startup

```
1. Load configuration (sentinel.toml)
2. Initialize security layer:
   - LuperIQ: request capability handles from kernel
   - Linux: set up seccomp/landlock/namespace sandbox
3. Connect to messaging platforms
4. Load SOUL.md personality file
5. Build tool definitions from granted capabilities
6. Enter main event loop
```

### 2. Message Processing

```
1. Receive message from messaging connector
2. Security check: is this sender authorized?
3. Build conversation context:
   - System prompt + SOUL.md
   - Available tools (derived from granted capabilities)
   - Recent conversation history
4. Send to LLM provider
5. Parse response:
   - Text → send back to messaging platform
   - Tool call → execute via Tool Executor
6. Tool execution:
   a. Capability check: does agent have handle for this resource?
   b. If no: deny and log
   c. If requires approval: queue for user confirmation
   d. If approved: execute in sandboxed subprocess
   e. Log result to audit trail
7. Send tool result back to LLM for next response
8. Repeat until LLM returns final text response
```

### 3. Skill Execution

Skills (plugins) run as separate processes with their own restricted capability sets:

```
Agent Process (capabilities: A, B, C, D)
    │
    └── Spawns: Skill Process (capabilities: B only)
            │
            ├── Can access resource B
            ├── Cannot access resources A, C, D
            ├── Cannot access agent's environment
            └── Communication via IPC channel only
```

On LuperIQ OS, skill processes get a new handle table with only the handles the agent explicitly transfers. On Linux, skill processes get a new seccomp/landlock profile.

## Security Model

### Capability Categories

| Category | Examples | Risk Level |
|----------|----------|-----------|
| **Network Connect** | `api.anthropic.com:443`, `api.telegram.org:443` | Medium |
| **Network Listen** | `localhost:8443` | High |
| **File Read** | `/data/projects`, `/config/soul.md` | Medium |
| **File Write** | `/data/output`, `/tmp/scratch` | High |
| **Process Spawn** | `python3`, `git`, `curl` | Very High |
| **Config Modify** | Write to `soul.md`, `sentinel.toml` | Critical |

### Approval Levels

| Level | Behavior | Use For |
|-------|----------|---------|
| **Always Allow** | Execute immediately, log | Low-risk reads, LLM API calls |
| **Allow Once** | Execute once, then revoke | One-time file access |
| **Ask Every Time** | Queue for user approval | Process spawning, file writes |
| **Always Deny** | Block immediately, alert | Sensitive paths, unknown hosts |

### Prompt Injection Defense

OpenClaw's SOUL.md backdoor works because the agent can freely modify its own personality file. Sentinel defenses:

1. **SOUL.md is read-only by default** — Agent has `FileRead("/config/soul.md")` but NOT `FileWrite`. Modification requires explicit user approval.
2. **Config hash verification** — On startup, Sentinel computes SHA-256 of SOUL.md and alerts if it has changed since last approved modification.
3. **Tool call filtering** — Before executing any tool call, Sentinel checks if the target resource matches granted capabilities. An LLM response saying "write to /config/soul.md" is blocked if no write handle exists.
4. **Output filtering** — Responses are scanned for known prompt injection patterns (base64 commands, curl|bash, encoded payloads).

## Data Flow

### LLM API Call

```
Agent                  Security Layer              Network
  │                        │                         │
  ├──── LLM request ──────►│                         │
  │                        ├── Check: has NetConnect  │
  │                        │   for api.anthropic.com? │
  │                        │   YES ──────────────────►│
  │                        │                         ├── HTTPS POST
  │                        │                         │   /v1/messages
  │                        │◄── Response ────────────┤
  │◄── LLM response ──────┤                         │
  │                        ├── Audit: logged          │
```

### Tool Execution (File Write)

```
Agent                  Security Layer              Kernel/OS
  │                        │                         │
  ├──── Write /data/x ────►│                         │
  │                        ├── Check: has FileWrite   │
  │                        │   for /data/?            │
  │                        │   YES, approval=ask ─────┤
  │                        │                         ├── Queue approval
  │                        │                         │
  │     [User approves via UI]                       │
  │                        │                         │
  │                        │◄── Approved ────────────┤
  │                        ├── Execute write ────────►│
  │◄── Success ────────────┤                         │
  │                        ├── Audit: logged          │
```

### Tool Execution (Blocked)

```
Agent                  Security Layer              Kernel/OS
  │                        │                         │
  ├──── Read /home/.ssh ──►│                         │
  │                        ├── Check: has FileRead    │
  │                        │   for /home/.ssh?        │
  │                        │   NO HANDLE              │
  │◄── PermissionDenied ──┤                         │
  │                        ├── Audit: BLOCKED logged  │
  │                        ├── Alert: suspicious      │
```

## Configuration

### sentinel.toml

```toml
[agent]
name = "my-sentinel"
soul = "/config/soul.md"

[llm]
provider = "anthropic"       # anthropic, openai
model = "claude-sonnet-4-5-20250929"
max_tokens = 4096
api_key_env = "ANTHROPIC_API_KEY"

[messaging.telegram]
enabled = true
token_env = "TELEGRAM_BOT_TOKEN"
allowed_users = [123456789]  # Telegram user IDs

[messaging.discord]
enabled = false

[messaging.websocket]
enabled = true
bind = "127.0.0.1:8443"     # NOT 0.0.0.0

[security]
mode = "strict"              # strict, permissive (Linux only)
audit_log = "/var/log/sentinel/audit.log"
approval_timeout = 300       # seconds before auto-deny

[capabilities]
# Explicit allowlist — everything else is denied
net_connect = [
    "api.anthropic.com:443",
    "api.telegram.org:443",
]
file_read = [
    "/data/projects",
    "/config/soul.md",
]
file_write = [
    "/data/output",
]
process_spawn = []           # Empty = no command execution
```

## Platform Differences

| Feature | LuperIQ OS | Linux |
|---------|-----------|-------|
| Capability enforcement | Kernel-level (mandatory) | seccomp/landlock (best-effort) |
| Audit log | Immutable (kernel-managed) | File-based (agent-managed) |
| Skill sandboxing | Separate handle tables | Separate seccomp profiles |
| Approval UI | Native OS dialog | Web dashboard |
| Browser engine | Built-in (LuperIQ browser) | Not available (no Chromium) |
| Network isolation | Capability handles per host | Network namespaces |
| Config protection | Capability-gated writes | File permissions |

## Comparison with OpenClaw Architecture

| Layer | OpenClaw | Sentinel |
|-------|----------|----------|
| Language | TypeScript/JavaScript | Rust |
| Runtime | Node.js 22 | Native binary |
| Package manager | npm (1,200+ deps) | cargo (0 runtime deps) |
| Browser | Chromium (~30M LOC) | LuperIQ browser (on LuperIQ OS) / none |
| TLS | OpenSSL (via Node.js) | LuperIQ TLS 1.3 / rustls |
| JSON | Built-in V8 | Custom parser (~500 LOC) |
| WebSocket | ws npm package | Custom implementation (~300 LOC) |
| Process isolation | Docker (opt-in) | OS capabilities (mandatory) |
| Memory footprint | 300MB-2GB | ~20MB |
| Startup time | 2-5 seconds | <100ms |
| Binary size | ~200MB (node + deps) | ~5MB |
