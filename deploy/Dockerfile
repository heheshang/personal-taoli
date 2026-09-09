FROM rust:1.75 as builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY src-tauri/Cargo.toml src-tauri/Cargo.toml

# Create dummy source files for caching
RUN mkdir -p crates/core/src && touch crates/core/src/lib.rs
RUN mkdir -p src-tauri/src && touch src-tauri/src/main.rs

# Build dependencies
RUN cargo build --release --workspace || true

# Remove dummy source files
RUN rm -rf crates/core/src src-tauri/src

# Copy actual source
COPY crates crates
COPY src-tauri src-tauri
COPY ui ui

# Build application
RUN cargo build --release --workspace

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary
COPY --from=builder /app/target/release/taoli-core /app/taoli-core

# Copy configuration
COPY config /app/config

# Create data directories
RUN mkdir -p /app/data /app/logs

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

EXPOSE 8080

CMD ["./taoli-core"]
