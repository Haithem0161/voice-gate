---
paths:
  - "**/auth*"
  - "**/middleware/**"
  - "**/jwt*"
  - "**/claims*"
---

# Authentication Rules (JWT with jsonwebtoken crate)

**Uses HS256 JWT** with a shared secret (`JWT_SECRET` env var). The `require_auth` middleware validates tokens and injects `Claims` into request extensions.

## JWT Token Structure (Claims)
| Field | Type | Description |
|-------|------|-------------|
| `sub` | `Uuid` | User ID |
| `email` | `String` | User email address |
| `exp` | `usize` | Expiry timestamp (Unix seconds) |
| `iat` | `usize` | Issued-at timestamp (Unix seconds) |

Default token lifetime: 24 hours.

## Key Functions (`middleware/auth.rs`)
| Function | Purpose |
|----------|---------|
| `create_token(user_id, email, secret)` | Creates a signed JWT string |
| `validate_token(token, secret)` | Decodes and validates a JWT, returns `Claims` |
| `require_auth` | Axum middleware that extracts Bearer token, validates it, and injects `Claims` into request extensions |

## Protecting Routes
Apply `require_auth` as a middleware layer on routes or routers:

```rust
use axum::{middleware, Router, routing::get};

// Protect individual routes
let protected_router = Router::new()
    .route("/api/profile", get(get_profile))
    .route("/api/settings", get(get_settings).put(update_settings))
    .layer(middleware::from_fn_with_state(state.clone(), require_auth));

// Or protect an entire nested router
let app = Router::new()
    .nest("/api/public", public_router)        // No auth
    .nest("/api/protected", protected_router)   // Auth required
    .with_state(state);
```

## Accessing Claims in Handlers
After `require_auth` middleware runs, `Claims` are available via `Extension`:

```rust
use axum::Extension;
use crate::middleware::auth::Claims;

pub async fn get_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,  // Injected by require_auth middleware
) -> Result<Json<User>, AppError> {
    let user = sqlx::query_as!(
        User,
        "SELECT id, name, email, created_at, updated_at FROM users WHERE id = $1",
        claims.sub  // The authenticated user's ID
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(user))
}
```

Note: `Extension(claims)` must come BEFORE `Json<T>` in the parameter list (body extractor must be last).

## Adding the utoipa Security Scheme
To document authenticated endpoints in Swagger:

1. Add security scheme to `#[openapi(...)]` in `main.rs`:
   ```rust
   #[derive(OpenApi)]
   #[openapi(
       // ... paths, components ...
       modifiers(&SecurityAddon),
   )]
   struct ApiDoc;

   struct SecurityAddon;
   impl utoipa::Modify for SecurityAddon {
       fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
           let components = openapi.components.as_mut().unwrap();
           components.add_security_scheme(
               "bearer_auth",
               utoipa::openapi::security::SecurityScheme::Http(
                   utoipa::openapi::security::HttpBuilder::new()
                       .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                       .bearer_format("JWT")
                       .build(),
               ),
           );
       }
   }
   ```

2. Add `security` to protected handler annotations:
   ```rust
   #[utoipa::path(
       get,
       path = "/api/profile",
       tag = "auth",
       security(("bearer_auth" = [])),  // <-- marks as authenticated
       responses(
           (status = 200, description = "User profile", body = User),
           (status = 401, description = "Unauthorized"),
       )
   )]
   pub async fn get_profile(...) { ... }
   ```

## Security Best Practices
- NEVER log JWT secrets or full tokens -- log only token prefix or claims
- NEVER hardcode `JWT_SECRET` -- always load from environment variable
- NEVER commit `.env` files containing secrets
- Token expiry should be kept short (24h for access tokens)
- For production: consider RS256 (asymmetric) instead of HS256 (symmetric)
- Always validate token expiry (the `jsonwebtoken` crate does this by default via `Validation::default()`)
- Return `401 Unauthorized` for missing/invalid tokens, `403 Forbidden` for valid token with insufficient permissions

