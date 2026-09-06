# Multi-stage Dockerfile for Assistant Server
FROM rust:1.80-slim AS builder

WORKDIR /usr/src/app

# Install required build tools & SSL certs
RUN apt-get update && apt-get install -y pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy workspace source files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY apps/mobile/src-tauri ./apps/mobile/src-tauri

# Build release binary for assistant-server
RUN cargo build --release -p assistant-server

# Final runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /usr/src/app/target/release/assistant-server /app/assistant-server

# Default port configuration
EXPOSE 8787

CMD ["/app/assistant-server"]
