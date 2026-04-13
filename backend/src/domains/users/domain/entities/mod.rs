use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// User entity -- core domain object
#[derive(Debug, Serialize, Deserialize, FromRow, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for creating a user
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUser {
    /// User's email address
    pub email: String,
    /// User's display name
    pub name: String,
}

/// Request body for updating a user
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUser {
    /// Updated email address
    pub email: Option<String>,
    /// Updated display name
    pub name: Option<String>,
}
