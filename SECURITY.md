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
- A restricted token (`exclude_client_data`) reading or changing a handoff it neither sent nor received
- Arbitrary code execution
- Path traversal or file access
- Denial of service via crafted MCP tool parameters

The following are out of scope:
- Issues requiring physical access to the server
- Social engineering
- Issues in dependencies (report those upstream, but feel free to let me know)

## Security Design

One ops-brain deployment is one trusted coordination domain. Per-agent tokens
bind caller identity and provenance; they do not isolate tenants. Deploy
separate instances when clients or operators must not be able to access one
another's data.

Key security features:

- **Restricted tokens**: a token with `exclude_client_data` is limited, in the queries themselves, to handoffs it sent or received
- **Audit logging**: every handoff deletion leaves a tombstone in `audit_log`
- **Bearer token auth**: HTTP transport requires authentication; scoped machine tokens and identity-bound agent tokens reduce accidental misuse
- **No outbound content**: the server sends handoff text to no other service; its only outbound request is the optional OAuth JWKS fetch
- **No secrets in code**: All credentials via environment variables
- **Input validation**: agent-slug shape, UUID parsing, and bounded text/context validation on tool and REST inputs
