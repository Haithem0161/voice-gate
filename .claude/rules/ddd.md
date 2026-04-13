---
paths:
  - "**/domains/**"
  - "**/domain/**"
  - "**/infrastructure/**"
  - "**/presentation/**"
---

# Domain-Driven Design Rules (Rust/Axum)

We use **DDD with Hexagonal Architecture** — organizing code by business domains with clear layer separation.

## Backend Structure
```
backend/src/
├── main.rs                          # Entry point, OpenApi derive, AppState, migrations
├── config/
│   └── mod.rs                       # Config::from_env()
├── common/
│   ├── mod.rs
│   └── errors/
│       └── mod.rs                   # AppError enum (thiserror + IntoResponse)
├── middleware/
│   ├── mod.rs
│   └── auth.rs                      # JWT auth (jsonwebtoken crate)
└── domains/                         # BUSINESS DOMAINS (Bounded Contexts)
    ├── mod.rs                       # pub mod health; pub mod users; ...
    ├── health/
    │   ├── mod.rs
    │   └── presentation/
    │       ├── mod.rs
    │       └── routes/
    │           └── mod.rs           # GET /api/health
    └── <domain>/
        ├── mod.rs                   # pub mod domain; pub mod infrastructure; pub mod presentation;
        ├── domain/                  # Core business logic (no external dependencies)
        │   ├── mod.rs
        │   ├── entities/
        │   │   └── mod.rs           # Domain structs: Resource, CreateResource, UpdateResource
        │   └── repositories/
        │       └── mod.rs           # Repository trait (port) using async_trait
        ├── presentation/            # API layer (inbound adapter)
        │   ├── mod.rs
        │   └── routes/
        │       └── mod.rs           # Axum handlers with utoipa annotations
        └── infrastructure/          # External integrations (outbound adapter)
            ├── mod.rs
            └── repositories/
                └── mod.rs           # Repository impl with SQLx queries (PgXxxRepository)
```

## DDD Layer Rules

| Layer | Purpose | Dependencies | Rust Traits |
|-------|---------|--------------|-------------|
| **Domain** | Entities, repository traits, business rules | None (isolated core) | `async_trait`, `serde`, `utoipa::ToSchema`, `sqlx::FromRow` |
| **Presentation** | HTTP handlers, request/response mapping | → Domain | `axum`, `utoipa` |
| **Infrastructure** | Database queries, external API clients | → Domain | `sqlx`, `async_trait` |

**Key Principles:**
- Domain layer has **zero infrastructure dependencies** — no sqlx queries, no axum types
- Infrastructure implements domain repository traits (Dependency Inversion)
- Presentation uses domain types for request/response, calls repository through `AppState`
- Each domain is a **Bounded Context** — self-contained and independently testable
- Repository traits live in `domain/repositories/`, implementations in `infrastructure/repositories/`

## Adding a New Domain (Checklist)

```bash
# 1. Create domain directory structure
mkdir -p backend/src/domains/<domain>/{domain/{entities,repositories},presentation/routes,infrastructure/repositories}

# 2. Create each mod.rs file in the tree
# 3. Register in backend/src/domains/mod.rs: pub mod <domain>;
```

Then implement in this order:

1. **Migration**: `backend/migrations/NNN_create_<resources>.sql`
2. **Entities** (`domain/entities/mod.rs`):
   - `Resource` — derives `Debug, Serialize, Deserialize, FromRow, ToSchema`
   - `CreateResource` — derives `Debug, Deserialize, ToSchema`
   - `UpdateResource` — derives `Debug, Deserialize, ToSchema` (all fields `Option<T>`)
3. **Repository trait** (`domain/repositories/mod.rs`):
   ```rust
   #[async_trait::async_trait]
   pub trait ResourceRepository: Send + Sync {
       async fn find_all(&self) -> Result<Vec<Resource>, sqlx::Error>;
       async fn find_by_id(&self, id: Uuid) -> Result<Option<Resource>, sqlx::Error>;
       async fn create(&self, input: &CreateResource) -> Result<Resource, sqlx::Error>;
       async fn update(&self, id: Uuid, input: &UpdateResource) -> Result<Option<Resource>, sqlx::Error>;
       async fn delete(&self, id: Uuid) -> Result<bool, sqlx::Error>;
   }
   ```
4. **Repository impl** (`infrastructure/repositories/mod.rs`):
   ```rust
   #[derive(Clone)]
   pub struct PgResourceRepository {
       pool: sqlx::PgPool,
   }

   impl PgResourceRepository {
       pub fn new(pool: sqlx::PgPool) -> Self { Self { pool } }
   }

   #[async_trait::async_trait]
   impl ResourceRepository for PgResourceRepository {
       // SQLx query_as! implementations
   }
   ```
5. **Handlers** (`presentation/routes/mod.rs`):
   - All 5 CRUD handlers with `#[utoipa::path]` annotations
   - `pub fn router() -> Router<AppState>`
6. **Wire into AppState** (`main.rs`):
   - Add `pub resource_repo: Arc<PgResourceRepository>` to `AppState`
   - Build repository: `Arc::new(PgResourceRepository::new(pool.clone()))`
   - Merge router: `.merge(domains::<domain>::presentation::routes::router())`
7. **Register in OpenApi** (`main.rs`):
   - Add handler paths to `paths(...)`
   - Add entity schemas to `components(schemas(...))`
   - Add tag to `tags(...)`
8. **Verify**: `cargo check`, `cargo clippy -- -D warnings`
9. **Regenerate frontend types**: `make generate-api`

## Layer Dependency Violations (NEVER DO)

- **NEVER** import `axum` types in `domain/` — domain must be framework-agnostic
- **NEVER** import `sqlx::query!` macros in `domain/` — that's infrastructure
- **NEVER** import from one domain into another — domains are independent bounded contexts
- **NEVER** put business logic in `presentation/routes/` — handlers should only map HTTP ↔ domain
- **NEVER** put HTTP concerns (StatusCode, Json) in `infrastructure/` — that's presentation

## Cross-Domain Communication
If two domains need to interact:
1. Define a shared trait or event in `common/`
2. Wire them together in `main.rs` through `AppState`
3. Do NOT create direct imports between `domains/<a>/` and `domains/<b>/`
