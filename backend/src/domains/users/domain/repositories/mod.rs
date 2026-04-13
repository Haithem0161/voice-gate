use uuid::Uuid;
use crate::domains::users::domain::entities::{CreateUser, UpdateUser, User};

/// Repository interface (port) for user persistence
/// Infrastructure layer implements this trait
#[async_trait::async_trait]
pub trait UserRepository: Send + Sync {
    async fn find_all(&self) -> Result<Vec<User>, sqlx::Error>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, sqlx::Error>;
    async fn create(&self, input: &CreateUser) -> Result<User, sqlx::Error>;
    async fn update(&self, id: Uuid, input: &UpdateUser) -> Result<Option<User>, sqlx::Error>;
    async fn delete(&self, id: Uuid) -> Result<bool, sqlx::Error>;
}
