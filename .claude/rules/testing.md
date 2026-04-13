---
paths:
  - "**/*test*"
  - "**/*spec*"
  - "**/tests/**"
  - ".github/workflows/**"
---

# Testing Rules

## Rust Backend Testing

### Test Location
- Unit tests: inline in source files using `#[cfg(test)] mod tests { ... }`
- Integration tests: `backend/tests/` directory
- Test database: use `#[sqlx::test]` for automatic transaction rollback, or create a separate `voice_gate_test` database

### Unit Test Pattern
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_returns_created_status() {
        // Arrange
        let pool = setup_test_db().await;
        let state = AppState { db: pool, jwt_secret: "test".into() };

        // Act
        let result = create_user(
            State(state),
            Json(CreateUser { name: "Test".into(), email: "test@example.com".into() }),
        ).await;

        // Assert
        assert!(result.is_ok());
        let (status, Json(user)) = result.unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(user.name, "Test");
    }
}
```

### SQLx Test Macro
The `#[sqlx::test]` macro provides a clean database for each test with automatic transaction rollback:
```rust
#[sqlx::test]
async fn test_user_crud(pool: PgPool) {
    // pool is a fresh database connection with migrations applied
    // all changes are rolled back after the test
    let user = sqlx::query_as!(User, "INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *", "Test", "test@test.com")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(user.name, "Test");
}
```
Requires `DATABASE_URL` to be set and the database to be accessible.

### What to Test on Backend
- Model serialization/deserialization (serde round-trips)
- Handler responses (correct status codes, body structure)
- Auth middleware (valid token, expired token, missing token, malformed token)
- Database queries (CRUD operations, edge cases, constraint violations)
- Error cases (not found returns 404, duplicate email returns 409, invalid input returns 400)

### Running Backend Tests
```bash
make test                                    # All backend tests
cd backend && cargo test                     # Backend only
cd backend && cargo test -- --test-threads=1 # Sequential (if tests share state)
cd backend && cargo test test_name           # Single test by name
cd backend && cargo test -- --nocapture      # Show println! output
```

## CI Pipeline (GitHub Actions)
The CI pipeline in `.github/workflows/ci.yml` runs on every push/PR to `main`:

**Backend job:**
1. `cargo check` -- compile-time verification (including SQLx queries)
2. `cargo test` -- run all tests
3. `cargo clippy -- -D warnings` -- ALL warnings treated as errors

### Clippy Rules
- CI runs `cargo clippy -- -D warnings` -- any Clippy warning fails the build
- Fix ALL Clippy warnings before committing
- Common fixes:
  - Unused variables: prefix with `_` (e.g., `_unused`)
  - Redundant clone: remove `.clone()` when not needed
  - Unnecessary `unwrap()`: use `?` operator or `expect("reason")`
  - `needless_return`: remove explicit `return` when it's the last expression

## SQLx Offline Mode for CI
If CI doesn't have a database available:
```bash
# Locally (with DATABASE_URL set and database running):
cargo sqlx prepare

# This generates .sqlx/ directory with query metadata
# Check .sqlx/ into version control

# In CI (without DATABASE_URL):
# cargo check works because it reads from .sqlx/ files
```
Run `cargo sqlx prepare --check` in CI to verify `.sqlx/` is up to date.
