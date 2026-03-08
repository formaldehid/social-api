# syntax=docker/dockerfile:1

############################
# Builder
############################
FROM rust:1-bookworm AS builder
WORKDIR /app

# Cache deps
COPY Cargo.lock Cargo.toml rust-toolchain.toml ./
COPY crates ./crates
COPY services ./services

# Build binary
RUN cargo build --release -p mock-profile-api

############################
# Runtime base
############################
FROM debian:bookworm-slim AS runtime-base
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/* \
  && useradd -u 10001 -m -s /usr/sbin/nologin app
USER 10001:10001
WORKDIR /app

############################
# mock-profile-api
############################
FROM runtime-base AS mock-profile-api
COPY --from=builder /app/target/release/mock-profile-api /app/mock-profile-api
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --retries=3 \
  CMD ["sh", "-c", "curl -fsS \"http://127.0.0.1:8080/health/ready\" || exit 1"]
ENTRYPOINT ["/app/mock-profile-api"]