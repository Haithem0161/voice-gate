# Rust Backend Template

A modern, high-performance Rust API backend.

## Stack

### Backend
- **Axum** — Web framework
- **SQLx** — Database driver with compile-time checked queries
- **PostgreSQL** — Database
- **utoipa** — OpenAPI/Swagger documentation
- **tower-http** — Compression, logging middleware
- **serde** — JSON serialization
- **tracing** — Structured logging
- **jsonwebtoken** — JWT authentication
- **dotenvy** — Environment variable management

### Infrastructure
- **systemd** — Process management (production)
- **Docker** — Containerized deployment (optional)

## Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- [PostgreSQL](https://www.postgresql.org/) (v15+)

## Getting Started

### 1. Clone the template

```bash
git clone https://github.com/your-username/voice-gate.git my-app
cd my-app
```

### 2. Set up the database

```bash
# Start Postgres via Docker
docker compose up -d postgres

# Copy environment file
cp backend/.env.example backend/.env

# Edit backend/.env with your database URL
```

### 3. Run database migrations

```bash
cd backend
sqlx migrate run
```

### 4. Start the backend

```bash
cd backend
cargo run
# API available at http://localhost:3000
# Swagger UI at http://localhost:3000/swagger-ui
```

## Project Structure

```
├── backend/
│   ├── src/
│   │   ├── main.rs              # Entry point, router setup
│   │   ├── config/
│   │   │   └── mod.rs           # App configuration
│   │   ├── common/
│   │   │   └── errors/          # AppError + IntoResponse
│   │   ├── domains/             # Bounded contexts (DDD)
│   │   │   ├── health/
│   │   │   └── users/
│   │   └── middleware/
│   │       └── auth.rs          # JWT auth middleware
│   ├── migrations/
│   │   └── 001_create_users.sql # Initial migration
│   ├── Cargo.toml
│   └── .env.example
├── docker-compose.yml           # Local Postgres
├── Makefile                     # Common commands
└── README.md
```

## Common Commands

```bash
# Run everything (from root)
make dev

# Backend only
make backend

# Run migrations
make migrate

# Run tests
make test
```

## Deployment

### Backend → VPS with systemd

```bash
# Build release binary
cd backend
cargo build --release

# Copy to server and set up systemd service
# See deploy/voice-gate.service for the systemd unit file
```

## License

MIT
