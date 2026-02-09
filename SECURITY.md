# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x (current) | Yes |

Only the latest release on the `main` branch receives security updates.

## Reporting a Vulnerability

If you discover a security vulnerability in Sentinel, **please report it privately**. Do not open a public GitHub issue.

**Email:** security@luperiq.com

Please include:

- A description of the vulnerability
- Steps to reproduce
- Affected component(s) (capability checker, audit logger, API client, tool executor, etc.)
- Impact assessment if known

## What to Expect

- **Acknowledgment** within 48 hours
- **Initial assessment** within 7 days
- **Fix or mitigation** timeline communicated after assessment
- **Credit** in release notes (unless you prefer anonymity)

We will not take legal action against security researchers who report vulnerabilities responsibly.

## Scope

The following are in scope:

- Capability checker bypasses (path traversal, allowlist evasion)
- Tool execution escapes (command injection, argument injection)
- Audit log tampering or evasion
- API key exposure or credential leakage
- Prompt injection leading to unauthorized tool use
- Configuration parsing vulnerabilities
- TLS/network security issues

The following are out of scope:

- Vulnerabilities in upstream LLM providers (Anthropic, OpenAI)
- Vulnerabilities in messaging platforms (Telegram, Discord, Slack)
- Social engineering of users to grant excessive capabilities
- Denial of service via API rate limiting

## Important Note

Sentinel's security model on Linux relies on application-level allowlists. These provide defense in depth but are not equivalent to kernel-enforced capabilities. A compromised Sentinel process on Linux could potentially bypass its own checks. The full security guarantee requires [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os), where the kernel enforces capability boundaries.

## Development Status

Sentinel is experimental software under active development. The capability checking system has not been independently audited. We welcome security review and responsible disclosure from the community.
