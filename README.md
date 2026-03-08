# Social API (Rust) — foundation commit

This repo is the **first commit** of the Social API assignment: an enterprise-grade Rust microservice foundation that compiles and runs locally and in Docker Compose.

## Run (Docker Compose)

```bash
docker compose up --build
```

Endpoints:

* Social API: [http://localhost:8080](http://localhost:8080)
* Mock Post API: [http://localhost:8081](http://localhost:8081)
* Mock Bonus Hunter API: [http://localhost:8082](http://localhost:8082)
* Mock Top Picks API: [http://localhost:8083](http://localhost:8083)
* Mock Profile API: [http://localhost:8084](http://localhost:8084)

## Smoke tests

```bash
# Liveness
curl -i http://localhost:8080/health/live

# Readiness (expects ready=true when postgres+redis+mocks are up)
curl -s http://localhost:8080/health/ready | jq .

# Like count (public)
curl -s http://localhost:8080/v1/likes/post/731b0395-4888-4822-b516-05b4b7bf2089/count | jq .

# Like status (requires auth token)
curl -s http://localhost:8080/v1/likes/post/731b0395-4888-4822-b516-05b4b7bf2089/status \
  -H "Authorization: Bearer tok_user_1" | jq .

# Metrics
curl -s http://localhost:8080/metrics | head
```

## Architecture (Ports & Adapters / Hexagonal)

The goal is to keep business logic independent of transport so we can swap HTTP → gRPC later without rewriting domain code.

* `crates/mock-common`: common mocks for external services
* `crates/social-core`: domain types + **ports (traits)** + use-cases (business logic)
* `services/social-api`: HTTP adapter (Axum), infra (config/logging/metrics), storage + external client adapters
* `services/mock-*`: minimal external service mocks required by the spec

### Why Axum?

Axum is built on Hyper/Tower, giving a clean middleware story (request IDs, tracing, metrics) and a production-friendly ecosystem.

### Config-driven content types

The Social API builds a content registry by scanning environment variables of the form:

`CONTENT_API_<TYPE>_URL`

Adding a new content type in the future is intended to be **configuration-only**.

## Cursor-based pagination (why)

The `/v1/likes/user` endpoint is scaffolded with cursor-based pagination because offset-based pagination becomes slower and inconsistent under concurrent writes. Cursor pagination stays stable and index-friendly as the dataset grows.

## What’s included in this first commit

* Rust workspace + service skeleton
* Fail-fast env config + `.env.example`
* JSON structured logs via `tracing`
* Request ID propagation + Prometheus metrics at `/metrics`
* Health checks:

  * `/health/live` always 200
  * `/health/ready` checks Postgres (writer+reader), Redis, and that at least one Content API is reachable
* DB migrations (initial schema + indexes)
* Dockerfile (multi-stage, non-root runtime) + docker-compose bringing up Postgres, Redis, Social API, and mocks
* Routes from the spec are registered; write-heavy endpoints are intentionally `501 NOT_IMPLEMENTED` but already return the spec-shaped error envelope
* End-to-end contract tests (Rust) that run against the docker-compose stack (GitHub Actions + local)

## What comes next (planned)

* Like/unlike write path with idempotency, atomic cache updates, and SSE event publishing
* Rate limiting (Redis-based) and circuit breaker for external calls
* Hot-path caching hardening (stampede control, warmup, bounded staleness decisions)
* Leaderboard implementation (hourly buckets + periodic refresh)
* Deeper integration tests (failure injection: Redis down, circuit breaker open)
* k6 load testing scripts

## Development

```bash
# format/lint/test
cargo fmt
cargo clippy --all-targets --all-features
cargo test

# run dependencies+mocks
docker compose up -d postgres redis mock-profile-api mock-post-api mock-bonus-hunter-api mock-top-picks-api

# run API locally
cp .env.example .env
cargo run -p social-api

# E2E contract tests (requires the docker-compose stack running)
RUN_INTEGRATION=1 SOCIAL_API_BASE_URL=http://localhost:8080 \
  cargo test -p social-api --test e2e_http
```

## Known warnings
* `cargo` may print future-incompatibility warnings originating from upstream crates (`redis`, `sqlx-postgres`). Our code compiles cleanly; we’ll bump/adjust dependencies as upstream releases fixes.