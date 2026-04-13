# Rust Backend Template

**Axum backend** with compile-time SQL and OpenAPI documentation.
Tech Stack: Rust/Axum 0.8 | SQLx 0.8 | PostgreSQL | utoipa

## Core Principles
1. **Compile-Time Safety**: Use `sqlx::query_as!` / `sqlx::query!` macros for compile-time verified SQL -- never raw string queries that skip validation
2. **API-First**: Every endpoint MUST have `#[utoipa::path]` annotations AND be registered in the `#[openapi(...)]` macro in `main.rs`
3. **Context7 First**: NEVER write code without first querying Context7 for up-to-date docs on every crate being used. This is mandatory, not optional
4. **No Emojis**: Never use emojis in code, comments, docs, or commit messages
5. **Makefile Orchestration**: Use `make` commands for all workflow tasks -- never run raw commands when a make target exists

## Critical Rules

### Git Commits
**NEVER commit with Claude authorship or co-authorship.** No `Co-Authored-By: Claude`, no Claude/Anthropic emails, no modifying git config. All commits must appear as solely human-made.

### Docker Safety
- DO: `docker compose up/restart/down`, `docker logs`, `docker exec`, `docker ps`
- **NEVER**: `docker rm`, `docker compose rm`, `docker system prune`, `docker container prune`, `docker volume prune` or any destructive cleanup

### Context7 Documentation Lookup (MANDATORY)
Before writing ANY implementation code using a crate:
1. Call `resolve-library-id` to find the library
2. Call `query-docs` with your specific use case
3. Use the returned docs/examples as the basis for implementation

This applies to: axum, sqlx, utoipa, tower-http, tokio, serde, thiserror, or any crate.

### SQLx Compile-Time Checks
- ALWAYS use `sqlx::query_as!` (returns typed struct) or `sqlx::query!` (returns anonymous record) macros
- NEVER use `sqlx::query_as::<_, T>(...)` runtime string queries -- they skip compile-time validation
- `DATABASE_URL` must be set (in `.env` or environment) for `cargo check` to verify queries at compile time
- For CI without a database: use `cargo sqlx prepare` to generate offline query data in `.sqlx/`, then check it into version control
- After schema changes: run `make migrate` then `cargo check` to verify all queries still compile

### Crate Management
- NEVER manually edit `Cargo.toml` dependency versions. Use `cargo add <crate>`.

## Development Workflow
1. **Research & Documentation (MANDATORY)** -- Query Context7 MCP for every crate. Study existing codebase patterns.
2. **Design Schema** -- Write SQL migration files in `backend/migrations/`, plan table relationships and indexes
3. **Run Migration** -- `make migrate` to apply schema changes
4. **Implement Backend** -- Models (with derive macros), handlers (with utoipa annotations), route registration in `main.rs`
5. **Verify Backend** -- `cargo check` (compile-time SQL verification), `cargo clippy -- -D warnings`, then curl-test endpoints
6. **Test** -- `make test`

## Port Mapping

| Port | Service |
|------|---------|
| 3000 | Backend API (Axum) |
| 5460 | PostgreSQL (Docker) |

## Testing (CURL First)
```bash
# Health check
curl -s http://localhost:3000/api/health | jq .

# List users
curl -s http://localhost:3000/api/users | jq .

# Create user
curl -s -X POST http://localhost:3000/api/users \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","name":"Test User"}' | jq .

# Swagger UI
open http://localhost:3000/swagger-ui
```

## Common Pitfalls
- **SQLx needs a live database for `cargo check`**: compile-time query verification requires `DATABASE_URL` pointing to a database with up-to-date schema. Run `make db` and `make migrate` first. For offline builds, use `cargo sqlx prepare`
- **`sqlx::query_as!` maps by column name, not position**: struct field names must match SQL column names (or use `AS` aliases). `Option<T>` fields must correspond to nullable columns
- **Never modify applied migrations**: SQLx tracks migrations by filename checksum. Modifying an applied migration causes checksum mismatch. Always create a new migration file instead
- **Forgetting to register in `#[openapi(...)]`**: adding a handler with `#[utoipa::path]` but not registering the path+schema in the `#[openapi(...)]` macro means it won't appear in Swagger
- **Axum extractor ordering**: body-consuming extractors (`Json<T>`, `String`, `Bytes`) MUST be the last parameter. `State`, `Path`, `Query`, `Extension`, `HeaderMap` can go in any order before the body extractor
- **`cargo watch` misses new files**: adding a new `.rs` file sometimes requires restarting `cargo watch`. If the new module isn't compiling, restart the watcher
- **`thiserror` v2 `#[error("...")]` format strings**: use `{0}` for tuple variants, `{field_name}` for named fields, and `{field_name:?}` for Debug formatting. `#[from]` auto-generates `From` impls

## Detailed Rules (auto-loaded by path)
Architecture details, patterns, and conventions are in `.claude/rules/`:
- `rust-backend.md` -- Axum patterns, handler structure, SQLx queries, error handling, module organization
- `docker.md` -- Docker commands, PostgreSQL container management
- `api-design.md` -- OpenAPI/utoipa documentation requirements, REST conventions
- `auth.md` -- JWT authentication, middleware usage, protecting routes, security scheme for Swagger
- `testing.md` -- Rust test patterns
- `migrations.md` -- SQLx migration rules and conventions
- `ddd.md` -- DDD layering and domain structure

<!-- MEMORY:START -->
# fullstack-rust-react

_Last updated: 2026-04-13 | 0 active memories, 0 total_

_For deeper context, use memory_search, memory_related, or memory_ask tools._
<!-- MEMORY:END -->
