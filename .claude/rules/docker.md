---
paths:
  - "docker-compose.yml"
  - "deploy/Dockerfile"
  - "**/.env*"
---

# Docker Rules

**Docker provides the PostgreSQL database. The backend runs locally (not in Docker during development).**

## Architecture
- PostgreSQL runs in Docker via `docker-compose.yml`
- Backend runs locally with `cargo watch -x run` (via `make backend`)
- `make dev` orchestrates: start DB -> run migrations -> start backend

## Allowed Commands
- DO: `docker compose up -d postgres`, `docker compose down`, `docker compose restart`
- DO: `docker logs`, `docker exec`, `docker ps`
- DO: Update `docker-compose.yml` and `deploy/Dockerfile`

## FORBIDDEN Commands (NEVER run these)
- **NEVER** run `docker rm`, `docker compose rm`, or any command that removes/deletes containers
- **NEVER** run `docker system prune`, `docker container prune`, `docker volume prune`, `docker image prune`
- These destroy data volumes containing development database state

## PostgreSQL Container Management
```bash
# Start database
docker compose up -d postgres     # or: make db

# View logs
docker logs voice-gate-postgres --tail 100 -f

# Connect to database directly
docker exec -it voice-gate-postgres psql -U postgres -d voice_gate_dev

# Stop (preserves data in volume)
docker compose down

# Stop and DESTROY data (only when explicitly requested by user)
docker compose down -v
```

## Connection Details
| Key | Value |
|-----|-------|
| Host | localhost |
| Port | 5460 |
| User | postgres |
| Password | postgres |
| Database | voice_gate_dev |
| Container | voice-gate-postgres |
| Volume | voice_gate_pgdata |

## Decision Trees

**Database not starting:**
1. Check if port 5460 is already in use: `ss -tlnp | grep 5460`
2. Check container logs: `docker logs voice-gate-postgres`
3. If port conflict: stop whatever uses port 5460 or change port in `docker-compose.yml`

**After changing docker-compose.yml:**
- Environment changes: `docker compose down && docker compose up -d postgres`
- Volume changes: requires explicit user approval to destroy data

**Schema out of sync:**
1. Make sure PostgreSQL container is running: `docker compose up -d postgres`
2. Run migrations: `make migrate` (or `cd backend && sqlx migrate run`)
3. Verify: `cargo check` -- if queries compile, schema is correct

## Production Dockerfile (deploy/Dockerfile)
- Multi-stage build: `rust:1.83-alpine` builder -> `alpine:3.21` runtime
- Only the release binary goes into the final image, no source code
- Exposes port 3000
