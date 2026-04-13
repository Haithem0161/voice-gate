---
paths:
  - "backend/src/**"
  - "backend/Cargo.toml"
  - "backend/Cargo.lock"
---

# Rust Backend Rules (Axum 0.8 + SQLx 0.8)

## Project Structure (DDD)
```
backend/src/
├── main.rs              # Entry point, OpenApi derive, AppState, migrations
├── config/
│   └── mod.rs           # Config::from_env()
├── common/
│   └── errors/
│       └── mod.rs       # AppError enum (thiserror + IntoResponse)
├── middleware/
│   └── auth.rs          # JWT auth (jsonwebtoken crate)
└── domains/             # Bounded Contexts (see ddd.md for full structure)
    ├── mod.rs
    ├── health/           # Health check domain
    └── <domain>/
        ├── domain/       # Entities, repository traits (no infra deps)
        ├── presentation/ # Axum route handlers with utoipa
        └── infrastructure/ # SQLx repository implementations
```
See `.claude/rules/ddd.md` for the complete DDD layer rules and adding new domains.

## Adding a New Domain (Checklist)
See `.claude/rules/ddd.md` for the full 9-step checklist covering:
1. Migration → 2. Entities → 3. Repository trait → 4. Repository impl → 5. Handlers → 6. AppState wiring → 7. OpenApi registration → 8. Verify

## Handler Pattern (MANDATORY)
Every handler function MUST follow this exact pattern. Based on Axum 0.8 conventions:

```rust
/// Short description for Swagger (becomes the operation summary)
///
/// Longer description becomes the operation description (optional)
#[utoipa::path(
    get,                              // HTTP method
    path = "/api/<resources>/{id}",   // Full path including /api prefix
    tag = "<resources>",              // Tag for Swagger grouping
    params(
        ("id" = Uuid, Path, description = "Resource ID")
    ),
    responses(
        (status = 200, description = "Success", body = Resource),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_resource(
    State(state): State<AppState>,   // State ALWAYS first
    Path(id): Path<Uuid>,            // Path params second
) -> Result<Json<Resource>, AppError> {
    let resource = state.resource_repo
        .find_by_id(id).await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(resource))
}
```

For POST/PUT handlers with request body:
```rust
#[utoipa::path(
    post,
    path = "/api/<resources>",
    tag = "<resources>",
    request_body = CreateResource,
    responses(
        (status = 201, description = "Created", body = Resource),
        (status = 400, description = "Invalid input"),
    )
)]
pub async fn create_resource(
    State(state): State<AppState>,
    Json(input): Json<CreateResource>,  // Body extractor MUST be LAST
) -> Result<(StatusCode, Json<Resource>), AppError> {
    let resource = state.resource_repo.create(&input).await?;
    Ok((StatusCode::CREATED, Json(resource)))
}
```

## Router Pattern
Each handler module exposes a `router()` function:
```rust
use axum::{Router, routing::{get, post, put, delete}};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/resources", get(list_resources).post(create_resource))
        .route("/resources/{id}", get(get_resource).put(update_resource).delete(delete_resource))
}
```

Note: Axum 0.8 uses `{id}` syntax for path params (not `:id`).

## Model Pattern (MANDATORY)
```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// Main resource struct -- used for database reads and API responses
#[derive(Debug, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Resource {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,  // Nullable columns -> Option<T>
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create request body -- only writable fields, no id/timestamps
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateResource {
    /// Human-readable name (required)
    pub name: String,
    /// Optional description
    pub description: Option<String>,
}

/// Update request body -- all fields optional for partial updates
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateResource {
    pub name: Option<String>,
    pub description: Option<String>,
}
```

## SQLx Query Rules
- ALWAYS use `sqlx::query_as!` for queries that return rows (compile-time type checked)
- ALWAYS use `sqlx::query!` for queries that don't map to a struct (DELETE, simple UPDATE)
- ALWAYS use `sqlx::query_scalar!` for queries returning a single column value
- ALWAYS list columns explicitly in SELECT -- never `SELECT *`
- Use `RETURNING` clause on INSERT/UPDATE to get the modified row back
- For UPDATE with optional fields: use `COALESCE($N, column_name)` pattern
- For nullable columns: the Rust struct field MUST be `Option<T>`
- Column name in SQL must match struct field name, or use `AS field_name` alias

## Error Handling Pattern
Use `thiserror` v2 with `IntoResponse` for API errors:

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("internal error: {0}")]
    Internal(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),      // #[from] auto-generates From<sqlx::Error>
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into()),
            AppError::Sqlx(_) => (StatusCode::INTERNAL_SERVER_ERROR, "database error".into()),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}
```

Key `thiserror` patterns:
- `#[error("message")]` -- sets the Display impl
- `#[from]` -- generates `From<WrappedType>` impl, enabling `?` operator
- `#[error(transparent)]` -- delegates Display and source() to the inner error
- `{0}` for tuple variant fields, `{field_name}` for named fields

## Axum Extractor Ordering (CRITICAL)
Extractors are applied in parameter order. Body-consuming extractors consume the request body and MUST be last:

```
1. State(state): State<AppState>     -- app state (always first by convention)
2. Path(id): Path<Uuid>              -- URL path parameters
3. Query(params): Query<Pagination>  -- URL query string (?page=1&limit=10)
4. headers: HeaderMap                -- all request headers
5. Extension(claims): Extension<T>   -- from middleware (e.g., auth claims)
6. Json(body): Json<CreateInput>     -- request body (MUST BE LAST)
```

If you put `Json<T>` before other extractors, Axum will consume the body before other extractors can read it, causing runtime errors.

## Async and Runtime Rules
- All handler functions are `async fn` -- required by Axum
- NEVER hold a `Mutex` or `RwLock` guard across an `.await` point -- causes deadlocks
- Use `Arc<T>` in `AppState` for shared data. The `AppState` struct must derive `Clone`
- For database operations: always use the pool reference (`&state.db`), never create new connections
- Use `tokio::spawn` for background tasks, not `std::thread::spawn`
- The `#[tokio::main]` macro in `main.rs` sets up the async runtime

## tower-http Middleware
The app uses two tower-http layers (order matters -- outermost processes first):
```rust
let app = Router::new()
    // ... routes ...
    .layer(CompressionLayer::new())  // Gzip response compression
    .layer(TraceLayer::new_for_http()) // Request/response logging via tracing
    .with_state(state);
```
- `TraceLayer::new_for_http()`: uses HTTP status code classification for span attributes
- Layers are applied bottom-up: TraceLayer runs first, then Compression
