.PHONY: dev backend frontend db migrate generate-api test clean

# Start everything for development
dev:
	@echo "Starting database..."
	docker compose up -d postgres
	@echo "Waiting for Postgres..."
	@sleep 2
	@$(MAKE) migrate
	@echo "Starting backend and frontend..."
	@$(MAKE) -j2 backend frontend

# Run backend
backend:
	cd backend && cargo watch -x run

# Run frontend
frontend:
	cd frontend && pnpm dev

# Start database
db:
	docker compose up -d postgres

# Run database migrations
migrate:
	cd backend && sqlx migrate run

# Generate API types from OpenAPI spec (backend must be running)
generate-api:
	cd frontend && pnpm generate-api

# Run all tests
test:
	cd backend && cargo test
	cd frontend && pnpm test

# Build for production
build:
	cd backend && cargo build --release
	cd frontend && pnpm build

# Clean build artifacts
clean:
	cd backend && cargo clean
	cd frontend && rm -rf node_modules dist

# Install dependencies
setup:
	cd frontend && pnpm install
	cargo install cargo-watch sqlx-cli
	@echo "Setup complete! Run 'make dev' to start developing."
