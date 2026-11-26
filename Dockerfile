# Build stage — match the runtime Debian codename so glibc symbols stay aligned.
FROM rust:1.94-slim-trixie AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
# Create dummy main for dependency caching
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy actual source and rebuild
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:trixie-slim

# Official MCP Registry ownership check (must match `name` in server.json).
LABEL io.modelcontextprotocol.server.name="io.github.TheK3nsai/ops-brain"

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false ops-brain

COPY --from=builder /app/target/release/ops-brain /app/ops-brain
COPY --from=builder /app/migrations /app/migrations

WORKDIR /app
USER ops-brain

EXPOSE 3000
CMD ["/app/ops-brain"]
