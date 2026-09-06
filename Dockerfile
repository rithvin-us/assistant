# Multi-stage Dockerfile for Assistant Server.
#
# The builder image must satisfy the workspace's `rust-version` (1.94, set by
# sqlx 0.9) and `edition = "2024"` / `resolver = "3"` in the root Cargo.toml.
# An older toolchain fails to even parse the manifest.
FROM rust:1.96-slim-bookworm AS builder

WORKDIR /usr/src/app

# Install required build tools & SSL certs
RUN apt-get update && apt-get install -y pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*

# Release tuning for a CI builder rather than a shipped mobile binary: fat LTO
# with a single codegen unit is what the workspace profile asks for, and it is
# the usual cause of an out-of-memory build on a small hosted builder. The
# server binary does not need it.
ENV CARGO_PROFILE_RELEASE_LTO=false \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_TERM_COLOR=never

# Copy workspace source files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY apps/mobile/src-tauri ./apps/mobile/src-tauri

# Build release binary for assistant-server
RUN cargo build --release --locked -p assistant-server

# Final runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /usr/src/app/target/release/assistant-server /app/assistant-server

# Default port configuration
EXPOSE 8787

CMD ["/app/assistant-server"]
