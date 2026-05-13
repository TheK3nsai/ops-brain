# TODO

- **Investigate Gemini CLI SSE disconnects:** The Node.js `@modelcontextprotocol/sdk` (which powers Gemini CLI) tends to silently drop idle HTTP SSE connections via the `eventsource` package, resulting in `Session not found` errors on the next tool call. We need to investigate if the Rust `rmcp` server can be configured to send SSE keep-alive/ping frames, or if the client configuration needs tweaking to keep the connection alive behind Caddy.
