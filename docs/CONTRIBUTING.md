# Contributing to ops-brain

On-demand guide for adding tools, branch conventions, and PR workflow.

## Branch Naming

```
<type>/<short-description>
```

Types: `feat/`, `fix/`, `refactor/`, `docs/`, `chore/`

Examples: `feat/delete-server-tool`, `fix/vendor-dedup-slug`, `docs/runbook-template`

## Commit Messages

```
<type>: <imperative description>

<optional body -- explain why, not what>
```

Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`

Examples:
- `feat: add delete_server tool with cascade safety`
- `fix: handle null embedding in hybrid search`
- `chore: update pgvector to 0.5`

## How to Add a New Tool (End-to-End Recipe)

Follow these steps in order:

**1. Migration (if schema changes needed)**

Create a new file in `migrations/` with the next sequence number:
```
migrations/YYYYMMDDHHMMSS_description.sql
```
- Use `IF NOT EXISTS` / `CREATE OR REPLACE` for idempotency
- Never modify existing migration files -- checksums are SHA-384 and will break

**2. Model (if new table/columns)**

Add or update struct in `src/models/`. Must derive:
```rust
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
```

**3. Repository function**

Add to the appropriate `src/repo/*.rs`. Pattern:
```rust
pub async fn delete_thing(pool: &PgPool, id: &Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM things WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
```
- Always use runtime `sqlx::query` / `sqlx::query_as` (not compile-time macros)
- Return `Result<T>` with `anyhow`

**4. Parameter struct**

Add to the appropriate `src/tools/*.rs` (NOT mod.rs). Pattern:
```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteThingParams {
    /// The slug of the thing to delete
    pub slug: String,
    /// Must be true to confirm deletion (safety gate)
    pub confirm: bool,
}
```

**5. Handler function**

Add the handler implementation to the appropriate category file (e.g., `src/tools/inventory.rs`):
```rust
pub(crate) async fn handle_delete_thing(brain: &super::OpsBrain, params: DeleteThingParams) -> CallToolResult {
    // 1. Resolve slug to entity (brain.pool)
    // 2. Check for FK references (safety gate)
    // 3. Require confirm=true
    // 4. Delete
    // 5. Return success message via json_result()
}
```

**Visibility note:** Use `pub(crate)` by default. If you want to write *handler-level* integration tests that call the handler directly (not just the repo layer), promote the handler to `pub` **and** promote the module declaration in `src/tools/mod.rs` from `mod foo` to `pub mod foo` — integration tests live in a separate crate and cannot reach `pub(crate)` items. See `src/tools/cc_team.rs` + `src/tools/knowledge.rs` for the established pattern, and `tests/integration.rs::check_in_tests` / `knowledge_provenance_tests` for example handler-level test modules.

**6. Tool stub**

Add a thin stub to the `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs`:
```rust
#[tool(description = "Delete a thing by slug. Requires confirm=true.")]
async fn delete_thing(&self, params: Parameters<inventory::DeleteThingParams>) -> Result<CallToolResult, McpError> {
    Ok(inventory::handle_delete_thing(self, params.0).await)
}
```
- Tool stubs MUST be in the single `#[tool_router] impl OpsBrain` block -- rmcp macro requirement
- Stubs only delegate -- all logic lives in the category handler
- Handler returns `CallToolResult` directly; stub wraps in `Ok()`
- Handler accesses `brain.pool`, `brain.embedding_client`, etc.

**7. Integration test**

Add to `tests/integration.rs`. Pattern:
```rust
#[tokio::test]
async fn test_delete_thing() {
    let pool = common::test_pool().await;
    // 1. Create test data
    // 2. Call delete
    // 3. Assert it's gone
    // 4. Assert FK safety gate works
}
```

**8. Update counts**

- Update tool count in `CLAUDE.md` and `README.md`

## PR Checklist

Before opening a PR, verify:

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] New tools have integration tests
- [ ] CLAUDE.md updated if tool count changed
- [ ] README.md updated if tool count changed
- [ ] No hardcoded credentials, URLs, or tokens
- [ ] Migration files are idempotent (`IF NOT EXISTS`, etc.)
- [ ] Cross-client safety considered: if the tool touches runbooks/knowledge, does it need `client_slug` and `acknowledge_cross_client` params?
- [ ] Handoff created to stealth for review/merge (PRs don't notify -- handoffs do)
