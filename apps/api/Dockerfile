# =============================================================================
# Builder Stage (using cargo-chef for excellent dependency caching)
# =============================================================================
FROM lukemathwalker/cargo-chef:latest-rust-1.95 AS chef
WORKDIR /app

# --- Planner stage: Analyze dependencies ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Builder stage: Build dependencies first (cached layer), then the app ---
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

# Build only the dependencies (this layer is cached unless Cargo.toml changes)
RUN cargo chef cook --release --recipe-path recipe.json

# Copy the actual source code and build the application
COPY . .
RUN cargo build --release --locked --bin virivu-api

# =============================================================================
# Runtime Stage (minimal image)
# =============================================================================
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# Install only the minimal runtime dependencies needed
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Copy only the compiled binary from the builder stage
COPY --from=builder /app/target/release/virivu-api /usr/local/bin/virivu-api

EXPOSE 8080

# Run the binary
CMD ["virivu-api"]
