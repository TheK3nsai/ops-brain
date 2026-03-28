# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in ops-brain, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: **k3nsai@gmail.com** with the subject line `[ops-brain security]`.

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

You should receive an acknowledgment within 48 hours. I'll work with you to understand the issue and coordinate a fix before any public disclosure.

## Scope

The following are in scope:
- SQL injection or query manipulation
- Authentication/authorization bypass (HTTP bearer token, MCP transport)
- Cross-client data leakage (bypassing the client-scope safety gate)
- Arbitrary code execution
- Path traversal or file access
- Denial of service via crafted MCP tool parameters

The following are out of scope:
- Issues requiring physical access to the server
- Social engineering
- Issues in dependencies (report those upstream, but feel free to let me know)

## Security Design

ops-brain handles multi-client data with different compliance domains. Key security features:

- **Client-scope safety gate**: Default-deny cross-client content surfacing with explicit acknowledgment required
- **Audit logging**: All cross-client access attempts are logged
- **Bearer token auth**: HTTP transport requires authentication
- **No secrets in code**: All credentials via environment variables
- **Input validation**: Slug resolution, UUID parsing, and parameter validation on all tool inputs
