# Disclaimer

## No Warranty

LuperIQ Sentinel is provided "as is" and "as available" without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement.

The full warranty disclaimer is included in the [LICENSE](LICENSE) file.

## Experimental Software

Sentinel is under active development. It has not been independently audited for security, reliability, or correctness.

Specifically:

- The **capability checker** enforces security boundaries at the application level on Linux. A compromised Sentinel process could potentially bypass its own checks. Kernel-enforced security requires [LuperIQ Agent OS](https://github.com/LuperIQ/luperiq-agent-os).
- The **audit logger** records tool calls and security events, but on Linux the log is managed by the application, not the kernel. Log integrity is not guaranteed on Linux.
- The **tool executor** runs system commands and performs file operations within allowlist boundaries. Bugs in the executor could allow unintended access.
- The **LLM integration** sends conversation data to third-party API providers (Anthropic, OpenAI). You are responsible for understanding the privacy and data handling policies of these providers.

## Third-Party Services

Sentinel connects to external services including:

- **LLM providers** (Anthropic, OpenAI) — Your prompts and conversations are sent to these services.
- **Messaging platforms** (Telegram, Discord, Slack) — Messages are transmitted through these platforms.

LuperIQ has no control over these services and makes no representations about their security, privacy, or availability. Review each provider's terms of service and privacy policy before use.

## Not a Security Product

Sentinel is designed with security as a priority, but it is not a certified security product. Do not rely on it as your sole defense against prompt injection, data exfiltration, or other AI agent security threats without your own independent security review.

## Assumption of Risk

By downloading, installing, or using Sentinel, you acknowledge that:

1. The software is experimental and may not work as expected.
2. The software may contain security vulnerabilities.
3. Data sent to third-party services is subject to their respective policies.
4. You are solely responsible for evaluating the software's suitability for your intended use.
5. You assume all risk associated with using the software.

## Limitation of Liability

In no event shall LuperIQ or its contributors be liable for any direct, indirect, incidental, special, exemplary, or consequential damages arising from the use of this software, including but not limited to loss of data, unauthorized access, API charges incurred, or disclosure of sensitive information.

## Questions

For questions about this disclaimer or the project's status, contact info@luperiq.com.
