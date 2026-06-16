FROM rust:1.89-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim
RUN useradd -m -u 10001 virival
WORKDIR /app
COPY --from=builder /app/target/release/virival-api /usr/local/bin/virival-api
COPY --from=builder /app/migrations /app/migrations
RUN mkdir -p /app/data/media && chown -R virival:virival /app
USER virival
EXPOSE 8090
ENV APP_NAME=virival-api
ENV BIND_ADDRESS=0.0.0.0:8090
ENV DATABASE_URL=postgres://postgres:postgres@postgres:5432/virival
ENV ALLOW_DEV_AUTH_BYPASS=true
ENV RUN_MODE=api
ENV MEDIA_STORAGE_ROOT=/app/data/media
ENV MEDIA_SIGNED_URL_TTL_SECONDS=900
CMD ["/usr/local/bin/virival-api"]
