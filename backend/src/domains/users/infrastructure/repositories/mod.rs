use sqlx::PgPool;
use uuid::Uuid;
use crate::domains::users::domain::entities::{CreateUser, UpdateUser, User};
use crate::domains::users::domain::repositories::UserRepository;

/// PostgreSQL implementation of UserRepository using SQLx
pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl UserRepository for PgUserRepository {
    async fn find_all(&self) -> Result<Vec<User>, sqlx::Error> {
        sqlx::query_as!(
            User,
            "SELECT id, email, name, created_at, updated_at FROM users ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as!(
            User,
            "SELECT id, email, name, created_at, updated_at FROM users WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
    }

    async fn create(&self, input: &CreateUser) -> Result<User, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"
            INSERT INTO users (id, email, name, created_at, updated_at)
            VALUES ($1, $2, $3, NOW(), NOW())
            RETURNING id, email, name, created_at, updated_at
            "#,
            Uuid::new_v4(),
            input.email,
            input.name,
        )
        .fetch_one(&self.pool)
        .await
    }

    async fn update(&self, id: Uuid, input: &UpdateUser) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"
            UPDATE users
            SET
                email = COALESCE($2, email),
                name = COALESCE($3, name),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, email, name, created_at, updated_at
            "#,
            id,
            input.email,
            input.name,
        )
        .fetch_optional(&self.pool)
        .await
    }

    async fn delete(&self, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM users WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
