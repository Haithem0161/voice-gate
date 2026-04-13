# Build stage
FROM rust:1.83-alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /app
COPY backend/ .

RUN cargo build --release

# Runtime stage
FROM alpine:3.21

RUN apk add --no-cache ca-certificates

COPY --from=builder /app/target/release/backend /usr/local/bin/backend

ENV HOST=0.0.0.0
ENV PORT=3000

EXPOSE 3000

CMD ["backend"]
