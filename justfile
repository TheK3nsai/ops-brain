# ops-brain development commands

# Default: show available commands
default:
    @just --list

# Build in debug mode
build:
    cargo build

# Build in release mode
release:
    cargo build --release

# Run with stdio transport (local dev)
run:
    cargo run

# Run with auto-reload on changes
watch:
    watchexec -r -e rs,sql -- cargo run

# Run database migrations
migrate:
    cargo run -- --transport stdio 2>&1 | head -5

# Start local PostgreSQL for development
db-up:
    docker compose up -d postgres
    @echo "Waiting for PostgreSQL..."
    @sleep 2
    @echo "PostgreSQL ready at localhost:5432"

# Stop local PostgreSQL
db-down:
    docker compose down

# Connect to local database
db-shell:
    docker compose exec postgres psql -U ops_brain -d ops_brain

# Seed the database with sample data
seed:
    @echo "Seeding database..."
    docker compose exec postgres psql -U ops_brain -d ops_brain -f /seed/seed.sql
    @echo "Done!"

# Run tests (unit only, no DB required)
test:
    cargo test --lib

# Run all tests including integration (requires PostgreSQL)
test-all:
    cargo test

# Run clippy
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Clean build artifacts
clean:
    cargo clean

# Full check: format, lint, test
check: fmt lint test

# Generate changelog
changelog:
    git-cliff -o CHANGELOG.md

# Count lines of code
loc:
    tokei
