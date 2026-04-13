use axum::Router;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod common;
mod config;
mod domains;
mod middleware;

use domains::users::infrastructure::repositories::PgUserRepository;

#[derive(OpenApi)]
#[openapi(
    paths(
        domains::health::presentation::routes::health_check,
        domains::users::presentation::routes::list_users,
        domains::users::presentation::routes::get_user,
        domains::users::presentation::routes::create_user,
        domains::users::presentation::routes::update_user,
        domains::users::presentation::routes::delete_user,
    ),
    components(schemas(
        domains::health::presentation::routes::HealthResponse,
        domains::users::domain::entities::User,
        domains::users::domain::entities::CreateUser,
        domains::users::domain::entities::UpdateUser,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "users", description = "User management endpoints"),
    )
)]
struct ApiDoc;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub jwt_secret: String,
    pub user_repo: Arc<PgUserRepository>,
}

#[tokio::main]
async fn main() {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "backend=debug,tower_http=debug".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load config
    let config = config::Config::from_env();

    // Set up database connection pool
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    tracing::info!("Database connected and migrations applied");

    // Build repositories (infrastructure layer)
    let user_repo = Arc::new(PgUserRepository::new(pool.clone()));

    // Build application state
    let state = AppState {
        db: pool,
        jwt_secret: config.jwt_secret,
        user_repo,
    };

    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(config.frontend_url.parse::<http::HeaderValue>().unwrap())
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router from domain routers
    let api_router = Router::new()
        .merge(domains::health::presentation::routes::router())
        .merge(domains::users::presentation::routes::router());

    let swagger: Router<()> = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi())
        .into();

    let app = Router::new()
        .nest("/api", api_router)
        .with_state(state)
        .merge(swagger)
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Server starting on {}", addr);
    tracing::info!("Swagger UI available at http://{}:{}/swagger-ui", config.host, config.port);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
