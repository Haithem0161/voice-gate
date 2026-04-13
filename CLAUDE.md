# Fullstack Rust + React Template

**Axum backend + React 19 frontend** with compile-time SQL, OpenAPI codegen, and type safety end-to-end.
Tech Stack: Rust/Axum 0.8 | SQLx 0.8 | PostgreSQL | React 19 | TanStack Router/Query | Orval | Tailwind CSS v4

## Core Principles
1. **Compile-Time Safety**: Use `sqlx::query_as!` / `sqlx::query!` macros for compile-time verified SQL -- never raw string queries that skip validation
2. **API-First**: Every endpoint MUST have `#[utoipa::path]` annotations AND be registered in the `#[openapi(...)]` macro in `main.rs`. The OpenAPI spec is the contract between backend and frontend
3. **Generated Frontend Types**: Frontend API types come from Orval codegen -- NEVER manually write API types, fetch functions, or query hooks. Files in `frontend/src/api/generated/` and `frontend/src/api/models/` are auto-generated and must NOT be edited
4. **Context7 First**: NEVER write code without first querying Context7 for up-to-date docs on every library/crate being used. This is mandatory, not optional
5. **No Emojis**: Never use emojis in code, comments, docs, or commit messages
6. **Makefile Orchestration**: Use `make` commands for all workflow tasks -- never run raw commands when a make target exists

## Critical Rules

### Git Commits
**NEVER commit with Claude authorship or co-authorship.** No `Co-Authored-By: Claude`, no Claude/Anthropic emails, no modifying git config. All commits must appear as solely human-made.

### Docker Safety
- DO: `docker compose up/restart/down`, `docker logs`, `docker exec`, `docker ps`
- **NEVER**: `docker rm`, `docker compose rm`, `docker system prune`, `docker container prune`, `docker volume prune` or any destructive cleanup

### Context7 Documentation Lookup (MANDATORY)
Before writing ANY implementation code using a crate or npm package:
1. Call `resolve-library-id` to find the library
2. Call `query-docs` with your specific use case
3. Use the returned docs/examples as the basis for implementation

This applies to: axum, sqlx, utoipa, tower-http, tokio, serde, thiserror, TanStack Router, TanStack Query, Orval, Zod, any crate or npm package.

### SQLx Compile-Time Checks
- ALWAYS use `sqlx::query_as!` (returns typed struct) or `sqlx::query!` (returns anonymous record) macros
- NEVER use `sqlx::query_as::<_, T>(...)` runtime string queries -- they skip compile-time validation
- `DATABASE_URL` must be set (in `.env` or environment) for `cargo check` to verify queries at compile time
- For CI without a database: use `cargo sqlx prepare` to generate offline query data in `.sqlx/`, then check it into version control
- After schema changes: run `make migrate` then `cargo check` to verify all queries still compile

### Package/Crate Management
- **Backend**: NEVER manually edit `Cargo.toml` dependency versions. Use `cargo add <crate>`.
- **Frontend**: NEVER manually edit `package.json`. Use `pnpm add <package>`.

## Development Workflow
1. **Research & Documentation (MANDATORY)** -- Query Context7 MCP for every library/crate. Study existing codebase patterns.
2. **Design Schema** -- Write SQL migration files in `backend/migrations/`, plan table relationships and indexes
3. **Run Migration** -- `make migrate` to apply schema changes
4. **Implement Backend** -- Models (with derive macros), handlers (with utoipa annotations), route registration in `main.rs`
5. **Verify Backend** -- `cargo check` (compile-time SQL verification), `cargo clippy -- -D warnings`, then curl-test endpoints
6. **Regenerate Frontend Types** -- Start backend, then `make generate-api` to update Orval-generated code
7. **Implement Frontend** -- Use generated hooks/types, build UI with TanStack Router + Tailwind CSS v4
8. **Test End-to-End** -- `make test`, then manual verification in browser

## Port Mapping

| Port | Service |
|------|---------|
| 3000 | Backend API (Axum) |
| 5173 | Frontend dev server (Vite) |
| 5432 | PostgreSQL (Docker) |

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
- **Orval codegen requires a running backend**: it reads the OpenAPI spec from `http://localhost:3000/api-docs/openapi.json`. Start the backend before running `make generate-api`
- **Forgetting to register in `#[openapi(...)]`**: adding a handler with `#[utoipa::path]` but not registering the path+schema in the `#[openapi(...)]` macro means it won't appear in Swagger and Orval won't generate types for it
- **Axum extractor ordering**: body-consuming extractors (`Json<T>`, `String`, `Bytes`) MUST be the last parameter. `State`, `Path`, `Query`, `Extension`, `HeaderMap` can go in any order before the body extractor
- **`cargo watch` misses new files**: adding a new `.rs` file sometimes requires restarting `cargo watch`. If the new module isn't compiling, restart the watcher
- **Tailwind CSS v4 has no config file**: v4 uses CSS-first configuration via `@theme` in CSS, not `tailwind.config.js`. The Vite plugin (`@tailwindcss/vite`) must come before the React plugin in `vite.config.ts`
- **TanStack Router auto-generates `routeTree.gen.ts`**: never edit this file. The Vite plugin regenerates it when route files change. The plugin must come before `react()` in `vite.config.ts`
- **`thiserror` v2 `#[error("...")]` format strings**: use `{0}` for tuple variants, `{field_name}` for named fields, and `{field_name:?}` for Debug formatting. `#[from]` auto-generates `From` impls

## Detailed Rules (auto-loaded by path)
Architecture details, patterns, and conventions are in `.claude/rules/`:
- `rust-backend.md` -- Axum patterns, handler structure, SQLx queries, error handling, module organization
- `docker.md` -- Docker commands, PostgreSQL container management
- `frontend.md` -- React 19/TanStack/Orval/Tailwind v4 patterns
- `api-design.md` -- OpenAPI/utoipa documentation requirements, REST conventions
- `auth.md` -- JWT authentication, middleware usage, protecting routes, security scheme for Swagger
- `testing.md` -- Rust test patterns, frontend test patterns
- `migrations.md` -- SQLx migration rules and conventions

<!-- MEMORY:START -->
# fullstack-rust-react

_Last updated: 2026-03-07 | 0 active memories, 0 total_

_For deeper context, use memory_search, memory_related, or memory_ask tools._
<!-- MEMORY:END -->
