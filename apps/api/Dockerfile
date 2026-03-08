FROM rust:1.85 as builder
WORKDIR /app
COPY Cargo.toml Cargo.toml
COPY src src
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/virivu-api /usr/local/bin/virivu-api
EXPOSE 8080
CMD ["virivu-api"]
