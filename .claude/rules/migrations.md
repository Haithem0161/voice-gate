---
paths:
  - "backend/migrations/**"
  - "**/migrate*"
---

# SQLx Migration Rules

## Migration File Format
- Location: `backend/migrations/`
- Naming: `NNN_short_description.sql` (e.g., `001_create_users.sql`, `002_add_user_roles.sql`)
- Sequential numbering with zero-padding (001, 002, 003...)
- SQLx runs migrations in filename order
- Create new migration files with: `cd backend && sqlx migrate add <description>`

## Writing Migrations

### MUST Follow
- Use `IF NOT EXISTS` on CREATE TABLE and CREATE INDEX for idempotency
- Every table gets `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()` and `updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`
- Primary keys: `UUID PRIMARY KEY DEFAULT gen_random_uuid()` (PostgreSQL 13+ built-in, no extension needed)
- If targeting PostgreSQL 12 or earlier: use `CREATE EXTENSION IF NOT EXISTS "uuid-ossp";` and `DEFAULT uuid_generate_v4()`
- Always use `TIMESTAMPTZ` (timezone-aware), never `TIMESTAMP` (timezone-naive)
- Add indexes on foreign key columns and frequently queried columns
- Add `UNIQUE` constraints for natural keys (email, username, slug)

### NEVER Do
- NEVER modify an already-applied migration file -- SQLx tracks applied migrations by filename + checksum
- NEVER delete migration files -- this breaks the migration history
- NEVER use `DROP TABLE` or `DROP COLUMN` without explicit user approval
- NEVER use `CASCADE` on drops/deletes without explicit user approval

## Templates

### New Table
```sql
CREATE TABLE IF NOT EXISTS resources (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    description TEXT,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_resources_status ON resources(status);
CREATE INDEX IF NOT EXISTS idx_resources_created_at ON resources(created_at);
```

### Adding Columns
```sql
ALTER TABLE resources
ADD COLUMN IF NOT EXISTS category_id UUID REFERENCES categories(id);

CREATE INDEX IF NOT EXISTS idx_resources_category_id ON resources(category_id);
```

### Adding a Junction Table (Many-to-Many)
```sql
CREATE TABLE IF NOT EXISTS resource_tags (
    resource_id UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    tag_id UUID NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (resource_id, tag_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_resource_tags_tag_id ON resource_tags(tag_id);
```

### Adding an Enum Type
```sql
DO $$ BEGIN
    CREATE TYPE user_role AS ENUM ('admin', 'editor', 'viewer');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

ALTER TABLE users ADD COLUMN IF NOT EXISTS role user_role NOT NULL DEFAULT 'viewer';
```

## Running Migrations
```bash
make migrate                              # Apply all pending migrations
cd backend && sqlx migrate run            # Same thing, manual
cd backend && sqlx migrate add <name>     # Create new empty migration file
cd backend && sqlx migrate info           # Show migration status
```

## After Migration Changes
1. Run `make migrate` to apply the new migration
2. Run `cargo check` to verify all `sqlx::query!` / `sqlx::query_as!` macros still compile
3. If compile-time queries fail: fix the SQL in handler code to match the new schema
4. Update the corresponding Rust model struct if columns were added/removed/renamed
5. If using offline mode: run `cargo sqlx prepare` to update `.sqlx/` metadata

## Rollback Strategy
SQLx does not support automatic rollbacks. To undo a migration:
1. Write a new migration that reverses the changes (e.g., `003_revert_add_column.sql`)
2. Apply it: `make migrate`
3. NEVER delete or modify the original migration file -- the history must be preserved

## Mapping SQL Types to Rust Types
| PostgreSQL Type | Rust Type | Notes |
|----------------|-----------|-------|
| `UUID` | `uuid::Uuid` | Requires `uuid` feature on sqlx |
| `VARCHAR(N)` / `TEXT` | `String` | |
| `INTEGER` / `INT4` | `i32` | |
| `BIGINT` / `INT8` | `i64` | |
| `BOOLEAN` | `bool` | |
| `TIMESTAMPTZ` | `chrono::DateTime<Utc>` | Requires `chrono` feature on sqlx |
| `JSONB` | `serde_json::Value` | Requires `json` feature on sqlx |
| `NUMERIC` / `DECIMAL` | `rust_decimal::Decimal` | Requires `rust_decimal` crate |
| Nullable column | `Option<T>` | MUST be `Option` in Rust struct |
| Custom ENUM | `String` or custom type | Use `#[sqlx(type_name = "enum_name")]` |
