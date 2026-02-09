# LuperIQ Sentinel — Security Model

## Threat Model

### What We Defend Against

| Threat | Attack Vector | Sentinel Defense |
|--------|--------------|-----------------|
| **Credential theft** | Agent reads ~/.ssh, .env, API keys | No FileRead handle for those paths → kernel denies |
| **Data exfiltration** | Agent sends data to attacker's server | No NetConnect handle for unknown hosts → kernel denies |
| **Persistent backdoor** | Prompt injection modifies SOUL.md | SOUL.md is read-only by default; writes require user approval |
| **Malicious skills** | ClawHub skill contains reverse shell | Skill runs in sandboxed process with minimal capabilities |
| **Remote code execution** | Exploiting exposed control API | WebSocket binds to 127.0.0.1, requires explicit NetListen capability |
| **Privilege escalation** | Agent spawns privileged subprocess | ProcessSpawn requires explicit capability per binary |
| **Supply chain attack** | Compromised npm dependency | No npm. Zero runtime dependencies. |
| **Sandbox escape** | Breaking out of Docker container | No Docker. OS kernel enforces capabilities directly. |

### What We Do NOT Defend Against

- **Compromised LLM provider** — If the LLM API itself is compromised, it can return malicious tool calls. Sentinel mitigates this by enforcing capabilities on tool execution, but cannot prevent the LLM from generating plausible-looking but harmful instructions within the agent's granted permissions.
- **Compromised kernel** — If the LuperIQ kernel itself is compromised, all bets are off. This is the same for any OS.
- **Physical access** — If an attacker has physical access to the hardware, they can extract data from storage. Use disk encryption for sensitive deployments.
- **Social engineering** — If a user grants excessive capabilities, the agent can misuse them within those bounds. Sentinel warns about overly broad grants but cannot prevent a user from overriding.

## Capability System (LuperIQ OS)

### How Capabilities Work

On LuperIQ OS, every resource is accessed through a **handle** — an opaque integer that references a kernel object with associated **rights**. A process can only use resources it has handles for. There is no way to "discover" or "create" a handle for a resource you don't already have access to.

```
Process Handle Table
──────────────────────────────
Handle 0: NetConnect → api.anthropic.com:443 [CONNECT, SEND, RECV]
Handle 1: NetConnect → api.telegram.org:443  [CONNECT, SEND, RECV]
Handle 2: FileRead   → /data/projects        [READ, STAT]
Handle 3: FileWrite  → /data/output          [READ, WRITE, CREATE]
Handle 4: FileRead   → /config/soul.md       [READ]
──────────────────────────────
Anything not in this table → PermissionDenied
```

### Capability Granting

Capabilities are granted at startup based on `sentinel.toml` configuration. The kernel creates handles and inserts them into the agent process's handle table. Additional capabilities can be requested at runtime through the approval workflow.

```
Startup:
1. Kernel reads sentinel.toml
2. For each configured capability:
   a. Create kernel object (file, network endpoint, etc.)
   b. Create handle with appropriate rights
   c. Insert handle into agent process's handle table
3. Agent process starts with exactly these handles
4. No additional handles can be created without user approval
```

### Runtime Capability Requests

When the agent needs a capability it doesn't have:

```
1. Agent calls tool that requires unconfigured resource
2. Security layer detects: no handle for this resource
3. Creates approval request:
   - What: "File write to /data/new-project/output.txt"
   - Why: "Agent wants to save generated code"
   - Risk: Medium (file write)
4. User sees approval dialog (native OS dialog or web dashboard)
5. User can:
   a. Allow once → temporary handle, revoked after use
   b. Allow always → permanent handle added to config
   c. Deny → PermissionDenied returned to agent
   d. Deny and alert → block + log as suspicious
```

## Prompt Injection Defense

### The SOUL.md Problem

OpenClaw's `SOUL.md` defines the agent's personality and behavioral boundaries. Zenity Labs demonstrated that prompt injection can modify SOUL.md to install a persistent backdoor:

1. Agent processes a web page containing hidden instructions
2. Hidden instructions tell the agent to modify SOUL.md
3. Modified SOUL.md contains attacker's instructions
4. Agent now follows attacker's instructions on every future interaction
5. A scheduled task re-injects the payload if SOUL.md is manually fixed

### Sentinel's Defense Layers

**Layer 1: Read-only config by default**
SOUL.md is loaded with a `FileRead` handle. The agent has no `FileWrite` handle for the config directory. Any attempt to write returns `PermissionDenied` from the kernel.

**Layer 2: Hash verification**
On startup, Sentinel computes SHA-256 of SOUL.md and stores it. Before each LLM call, the hash is verified. If the file has been modified outside of Sentinel (impossible on LuperIQ OS without the write handle, but possible on Linux), the agent halts and alerts.

**Layer 3: Tool call validation**
Before executing any tool call from the LLM, Sentinel checks:
- Does the agent have the required capability?
- Is the target resource in the allowlist?
- Does this tool call match known prompt injection patterns?

**Layer 4: Output scanning**
Responses from the LLM are scanned for patterns commonly used in prompt injection:
- Base64-encoded shell commands
- `curl | bash` or `wget | sh` patterns
- Attempts to modify config files
- Attempts to exfiltrate environment variables

**Layer 5: No scheduled tasks**
On LuperIQ OS, the agent cannot create scheduled tasks (cron jobs, systemd timers) because there is no such mechanism available without explicit capability. The persistence vector used in the "OpenDoor" attack is not available.

## Skill Sandboxing

### How Skills Run

Each skill runs as a **separate OS process** with its own restricted capability set. The agent transfers a minimal subset of its own handles to the skill process.

```
Agent Process
├── Handle 0: NetConnect(api.anthropic.com:443)
├── Handle 1: NetConnect(api.telegram.org:443)
├── Handle 2: FileRead(/data/projects)
├── Handle 3: FileWrite(/data/output)
│
└── Spawns Skill: "code-formatter"
    ├── Handle 0: FileRead(/data/projects)   ← transferred from agent
    ├── Handle 1: FileWrite(/data/output)    ← transferred from agent
    │
    ├── NO NetConnect handles                ← cannot phone home
    ├── NO access to agent's API keys        ← separate process
    └── Communication only via IPC channel   ← sandboxed
```

### Skill Manifest

Each skill declares what capabilities it needs:

```toml
[skill]
name = "code-formatter"
version = "1.0.0"
description = "Formats source code files"

[capabilities.required]
file_read = ["/data/projects"]
file_write = ["/data/output"]

[capabilities.optional]
net_connect = []    # Doesn't need network

[capabilities.never]
process_spawn = true  # Will never ask to spawn processes
```

The agent validates the manifest against its own capabilities and the user's configuration before granting handles.

## Audit Logging

### Log Format

Every security-relevant event is logged:

```json
{
  "timestamp": "2026-02-09T15:30:45.123Z",
  "event": "capability_use",
  "agent": "my-sentinel",
  "capability": "FileRead",
  "resource": "/data/projects/main.rs",
  "result": "allowed",
  "trigger": "tool_call",
  "llm_request_id": "msg_01XYZ..."
}

{
  "timestamp": "2026-02-09T15:30:46.789Z",
  "event": "capability_denied",
  "agent": "my-sentinel",
  "capability": "FileRead",
  "resource": "/home/user/.ssh/id_rsa",
  "result": "denied",
  "reason": "no_handle",
  "trigger": "tool_call",
  "llm_request_id": "msg_01XYZ...",
  "alert": true
}
```

### On LuperIQ OS

The audit log is written by the kernel, not by the agent process. The agent cannot modify, delete, or suppress audit entries. The log is stored in a kernel-managed buffer and flushed to a protected file.

### On Linux

The audit log is written by the Sentinel process to a file. The process's landlock profile prevents it from modifying the audit log after writing (append-only). This is weaker than kernel-managed logging but still provides a useful record.

## Network Security

### Default Configuration

```toml
[messaging.websocket]
bind = "127.0.0.1:8443"    # Loopback only, NOT 0.0.0.0
```

Compare to OpenClaw's default: `0.0.0.0:18789` (all interfaces, all IPs).

### Network Capability Model

Each outbound connection requires an explicit `NetConnect` handle:

```
Configured:
  NetConnect(api.anthropic.com:443)   → Allowed
  NetConnect(api.telegram.org:443)    → Allowed

Not configured:
  NetConnect(evil.com:443)            → PermissionDenied
  NetConnect(10.0.0.1:22)            → PermissionDenied
  NetConnect(metadata.google:80)     → PermissionDenied (cloud metadata)
```

This prevents:
- Data exfiltration to attacker servers
- SSRF attacks against internal networks
- Cloud metadata service access (common in cloud exploitation)

## Hardening Recommendations

### Minimal Configuration

For maximum security, grant the minimum capabilities needed:

```toml
[capabilities]
net_connect = [
    "api.anthropic.com:443",   # LLM provider only
]
file_read = []                 # No file access
file_write = []                # No file access
process_spawn = []             # No command execution
```

This creates an agent that can only chat — it cannot access files, run commands, or connect to anything except the LLM API.

### Production Configuration

For a production deployment with Telegram integration:

```toml
[capabilities]
net_connect = [
    "api.anthropic.com:443",
    "api.telegram.org:443",
]
file_read = [
    "/data/knowledge-base",    # Read-only reference data
]
file_write = [
    "/data/output",            # Scoped output directory
]
process_spawn = [
    "python3",                 # Only Python, nothing else
]
```

### What NOT to Do

```toml
# DANGEROUS: Do not do this
[capabilities]
net_connect = ["*:*"]          # All hosts, all ports
file_read = ["/"]              # Entire filesystem
file_write = ["/"]             # Entire filesystem
process_spawn = ["*"]          # Any binary
```

Sentinel will warn if overly broad capabilities are configured and require explicit `--i-know-what-im-doing` flag to start.
