---
paths:
  - "**/presentation/routes/**"
  - "**/domain/entities/**"
  - "backend/src/main.rs"
---

# API Design & OpenAPI Documentation Rules

## utoipa v5 Requirements

### Every Handler MUST Have
A `#[utoipa::path(...)]` attribute with ALL required fields:
```rust
#[utoipa::path(
    get,                                    // HTTP method (get, post, put, delete, patch)
    path = "/api/resources/{id}",           // Full path with /api prefix
    tag = "resources",                      // Swagger UI grouping tag
    params(
        ("id" = Uuid, Path, description = "Resource ID")
    ),
    responses(
        (status = 200, description = "Success", body = Resource),
        (status = 404, description = "Not found"),
    ),
    // security(("bearer_auth" = [])),      // Uncomment for authenticated endpoints
)]
```

For POST/PUT with request body:
```rust
#[utoipa::path(
    post,
    path = "/api/resources",
    tag = "resources",
    request_body = CreateResource,          // The ToSchema type for the body
    responses(
        (status = 201, description = "Created", body = Resource),
        (status = 400, description = "Invalid input"),
    )
)]
```

For query parameters:
```rust
#[utoipa::path(
    get,
    path = "/api/resources",
    tag = "resources",
    params(
        ("page" = Option<i64>, Query, description = "Page number"),
        ("per_page" = Option<i64>, Query, description = "Items per page"),
    ),
    responses(
        (status = 200, description = "List of resources", body = Vec<Resource>),
    )
)]
```

### Every Model Used in API MUST Have
- `#[derive(ToSchema)]` from utoipa -- required for OpenAPI schema generation
- `#[derive(Serialize)]` for response models
- `#[derive(Deserialize)]` for request models (Create/Update structs)
- Doc comments (`///`) on struct and fields become OpenAPI descriptions

### Registration in main.rs (MANDATORY -- MOST COMMON MISTAKE)
After creating a new handler or model, you MUST add it to the `#[openapi(...)]` macro in `main.rs`:

```rust
#[derive(OpenApi)]
#[openapi(
    paths(
        domains::health::presentation::routes::health_check,
        domains::users::presentation::routes::list_users,       // <-- ADD new handler paths here
        domains::users::presentation::routes::get_user,
        domains::users::presentation::routes::create_user,
        domains::users::presentation::routes::update_user,
        domains::users::presentation::routes::delete_user,
    ),
    components(schemas(
        domains::health::presentation::routes::HealthResponse,
        domains::users::domain::entities::User,                 // <-- ADD new schema types here
        domains::users::domain::entities::CreateUser,
        domains::users::domain::entities::UpdateUser,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "users", description = "User management endpoints"),
        // <-- ADD new tags here
    )
)]
struct ApiDoc;
```

**Forgetting this step means:**
- The endpoint won't appear in Swagger UI at `/swagger-ui`
- The OpenAPI spec won't include the endpoint

### Alternative: utoipa-axum OpenApiRouter
For larger projects, consider using `utoipa-axum` which auto-collects routes:
```rust
use utoipa_axum::{routes, router::OpenApiRouter};

let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
    .routes(routes!(list_resources, create_resource))
    .routes(routes!(get_resource, update_resource, delete_resource))
    .with_state(state)
    .split_for_parts();
```
This eliminates the need to manually register paths in `#[openapi(...)]`.

## REST Conventions

### URL Patterns
| Action | Method | Path | Handler Name | Returns |
|--------|--------|------|--------------|---------|
| List | GET | `/api/<resources>` | `list_<resources>` | `Json<Vec<Resource>>` |
| Get one | GET | `/api/<resources>/{id}` | `get_<resource>` | `Json<Resource>` |
| Create | POST | `/api/<resources>` | `create_<resource>` | `(StatusCode::CREATED, Json<Resource>)` |
| Update | PUT | `/api/<resources>/{id}` | `update_<resource>` | `Json<Resource>` |
| Delete | DELETE | `/api/<resources>/{id}` | `delete_<resource>` | `StatusCode::NO_CONTENT` |

### Response Conventions
- **List**: return `Json<Vec<T>>` (or a pagination wrapper struct)
- **Create**: return `(StatusCode::CREATED, Json<T>)` -- status 201, NOT 200
- **Delete**: return `StatusCode::NO_CONTENT` -- status 204 with no body
- **Errors**: return `AppError` which serializes to `{"error": "message"}`
- **IDs**: always UUIDs (`uuid::Uuid`) -- never sequential integers

### Naming
- URLs: plural nouns, kebab-case for multi-word (`/api/user-profiles`)
- Rust handlers: snake_case matching CRUD verb (`list_users`, `get_user`, `create_user`)
- Tags: match the resource name (plural, lowercase)
- Axum 0.8 path params use `{id}` syntax, not `:id`

## Swagger UI
- Available at: `http://localhost:3000/swagger-ui`
- OpenAPI JSON spec: `http://localhost:3000/api-docs/openapi.json`
- Always verify new endpoints appear in Swagger
- Swagger UI is configured in `main.rs`:
  ```rust
  .merge(SwaggerUi::new("/swagger-ui")
      .url("/api-docs/openapi.json", ApiDoc::openapi()))
  ```
