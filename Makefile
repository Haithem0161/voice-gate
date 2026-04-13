.PHONY: dev backend db migrate test build clean setup

# Start everything for development
dev:
	@echo "Starting database..."
	docker compose up -d postgres
	@echo "Waiting for Postgres..."
	@sleep 2
	@$(MAKE) migrate
	@echo "Starting backend..."
	@$(MAKE) backend

# Run backend
backend:
	cd backend && cargo watch -x run

# Start database
db:
	docker compose up -d postgres

# Run database migrations
migrate:
	cd backend && sqlx migrate run

# Run all tests
test:
	cd backend && cargo test

# Build for production
build:
	cd backend && cargo build --release

# Clean build artifacts
clean:
	cd backend && cargo clean

# Install dependencies
setup:
	cargo install cargo-watch sqlx-cli
	@echo "Setup complete! Run 'make dev' to start developing."
