# Social API (Rust)

A production-style Rust microservice implementing the BeInCrypto Social API assignment.

This submission aims to cover all required endpoints and the high-signal operational pieces reviewers care about most: intentional data design, clear architecture boundaries, correct idempotent write behavior, cache-aware hot paths, graceful degradation, observability, and deployment ergonomics.

## Requirement coverage

### Required endpoints

- `POST /v1/likes` — like content
- `DELETE /v1/likes/{content_type}/{content_id}` — unlike content
- `GET /v1/likes/{content_type}/{content_id}/count` — public count
- `GET /v1/likes/{content_type}/{content_id}/status` — auth status
- `GET /v1/likes/user` — auth user likes with cursor pagination
- `POST /v1/likes/batch/counts` — public batch counts
- `POST /v1/likes/batch/statuses` — auth batch statuses
- `GET /v1/likes/top` — leaderboard for `24h | 7d | 30d | all`
- `GET /v1/likes/stream` — SSE stream for one content item
- `GET /health/live`
- `GET /health/ready`
- `GET /metrics`

### Required technical behavior

- PostgreSQL source of truth + versioned migrations
- Redis used for like counts, content validation cache, and rate limiting
- Rate limiting headers on all responses, plus `Retry-After` on `429`
- Circuit breaker on external calls
- Graceful shutdown with bounded drain timeout and final SSE `shutdown` event
- Structured JSON logging via `tracing`
- Prometheus metrics at `/metrics`
- Docker multi-stage builds + docker-compose stack + mock services

### Bonus artifacts included

- OpenAPI spec: `openapi/social-api.openapi.yaml`
- Proto definitions: `proto/`
- k6 load test scripts: `k6/`
- Submission checklist: `docs/submission-checklist.md`

## Repo layout

- `crates/social-core` — domain types, ports/traits, use-cases, circuit breaker
- `crates/mock-common` — shared mock helpers
- `services/social-api` — HTTP adapter, storage adapters, external clients, middleware, infra
- `services/mock-content-api` — content validation mocks
- `services/mock-profile-api` — token validation mock
- `openapi/` — API contract bonus artifact
- `proto/` — gRPC/protobuf bonus artifact
- `k6/` — load testing bonus artifact
- `docs/` — submission notes

## Architecture

The service is intentionally organized in a ports-and-adapters style.

- **Domain / use-cases** live in `social-core`
- **Ports** describe what the domain needs: auth provider, content catalog, repositories, caches
- **Adapters** implement those ports using HTTP, Postgres, and Redis
- **Axum** is only the transport adapter; business logic is not coupled to HTTP-specific types

This makes the “HTTP today, gRPC tomorrow” requirement realistic rather than decorative. The included proto definitions are meant to show that transport evolution can happen without rewriting core behavior.

## Data model and indexing

### Tables

`likes`
- source of truth for who liked what and when
- unique constraint on `(user_id, content_type, content_id)` enforces one like per user per item

`like_counts`
- materialized all-time counts for fast reads and fast DB fallback
- stores a monotonic `seq` version to prevent stale cache overwrites

`like_buckets_hourly`
- hourly aggregates for scalable leaderboard windows (`24h`, `7d`, `30d`)
- avoids naive leaderboard scans over the full `likes` table

### Indexing rationale

Indexes are intentionally limited to the access patterns the service actually uses:

- uniqueness / idempotency on `(user_id, content_type, content_id)`
- by-content lookups for counts/status paths
- by-user recent-order traversal for cursor pagination

Every index has write cost, so the schema tries to stay minimal but sufficient.

## Caching and consistency model

### Like counts

Redis stores hot-path like counts under keys shaped like:

- `likes:count:{content_type}:{content_id}`

Cache values include both `count` and `seq`.
Writes update Postgres first, then update Redis via a compare-and-set Lua script that only overwrites when the incoming `seq` is newer or equal. This prevents stale cache writes under concurrent requests.

### Content validation

Content validation responses are cached in Redis under:

- `content:exists:{content_type}:{content_id}`

This reduces dependency pressure on the content mocks / external content APIs and keeps the write path fast.

### Leaderboard

Leaderboard payloads are cached separately and refreshed by a background task. Reads prefer Redis and fall back to Postgres aggregation when needed.

### Redis degraded mode

If Redis is unavailable, the service continues operating with degraded performance:

- count reads fall back to Postgres
- leaderboard reads fall back to Postgres
- content validation falls back to direct HTTP calls
- rate limiting becomes the one feature that depends on Redis state by design

### Staleness contract

- like counts are written through quickly after successful DB commits
- leaderboard freshness is bounded by `LEADERBOARD_REFRESH_INTERVAL_SECS`
- content validation freshness is bounded by `CACHE_TTL_CONTENT_VALIDATION_SECS`

## Endpoint behavior highlights

### Like / Unlike

The write path is transactional and idempotent:

- **Like** inserts into `likes` with conflict protection
- if inserted, it increments `like_counts` and the correct hourly bucket
- if already present, it returns the current count without double-counting

Unlike mirrors this logic:

- delete from `likes` with `RETURNING liked_at`
- if a row existed, decrement all-time count and the matching hourly bucket
- if not, return idempotent success without changing count

### Count endpoint

The public count endpoint is cache-first and does not call content validation. That keeps the hottest path focused on Redis / DB only.

### Batch counts

Batch counts are optimized for the “content listing page” path:

- input limit is capped at 100
- Redis is queried in bulk
- misses are fetched in one DB query
- results preserve request order

### User likes pagination

`/v1/likes/user` uses cursor pagination rather than offset pagination.

Why:
- offset pagination gets slower as offsets grow
- offsets can shift under concurrent writes
- a `(liked_at, content_id)` cursor is stable and index-friendly

### Leaderboard

The leaderboard uses `like_buckets_hourly` for bounded windows and `like_counts` for all-time reads.
That keeps the query shape scalable and aligned with the assignment’s “naive leaderboard won’t scale” warning.

### SSE stream

SSE is implemented per content item and backed by Redis Pub/Sub.

- channel shape: `likes:events:{content_type}:{content_id}`
- immediate heartbeat on connect
- periodic heartbeat every `SSE_HEARTBEAT_INTERVAL_SECS`
- bounded in-memory channel for backpressure protection
- slow clients drop buffered events instead of growing unbounded memory
- on graceful shutdown, clients receive a final `shutdown` event and the stream closes

## External dependencies and circuit breaker

External calls include:

- profile API token validation
- content API existence validation

Those calls are wrapped by a circuit breaker with the assignment’s intended state transitions:

- closed → open after enough failures
- open → half-open after recovery timeout
- half-open → closed after enough successes

When the content validation breaker is open, writes fail fast with `DEPENDENCY_UNAVAILABLE`, while read endpoints that can serve from cache / DB continue working.

## Observability

### Logging

Structured logs are emitted through `tracing` with request-oriented fields such as:

- `request_id`
- `service`
- `method`
- `path`
- `status`
- `latency_ms`
- `user_id` when authenticated

External calls and circuit breaker transitions are logged as well.

### Metrics

The service exposes the metric families requested in the assignment, including:

- `social_api_http_requests_total`
- `social_api_http_request_duration_seconds`
- `social_api_cache_operations_total`
- `social_api_external_calls_total`
- `social_api_external_call_duration_seconds`
- `social_api_circuit_breaker_state`
- `social_api_db_pool_connections`
- `social_api_sse_connections_active`
- `social_api_likes_total`

### Health checks

- `/health/live` returns `200` if the process is alive
- `/health/ready` checks Postgres, Redis, and at least one reachable content API
- readiness fails during graceful shutdown so upstreams can drain traffic cleanly

## Graceful shutdown

On `SIGTERM`, the service:

1. stops accepting new connections
2. flips readiness to failing
3. signals background tasks and SSE streams
4. emits a final SSE `shutdown` event
5. drains in-flight requests with a bounded timeout
6. closes pools and exits cleanly

## Run with Docker Compose

```bash
docker compose up --build
```

Endpoints:

- Social API: `http://localhost:8080`
- Mock Post API: `http://localhost:8081`
- Mock Bonus Hunter API: `http://localhost:8082`
- Mock Top Picks API: `http://localhost:8083`
- Mock Profile API: `http://localhost:8084`

## Quick smoke test

```bash
curl -s http://localhost:8080/health/ready | jq .
curl -s http://localhost:8080/v1/likes/post/731b0395-4888-4822-b516-05b4b7bf2089/count | jq .
curl -X POST http://localhost:8080/v1/likes \
  -H "Authorization: Bearer tok_user_1" \
  -H "Content-Type: application/json" \
  -d '{"content_type":"post","content_id":"731b0395-4888-4822-b516-05b4b7bf2089"}' | jq .
curl -s "http://localhost:8080/v1/likes/top?window=24h&limit=5" | jq .
curl -N "http://localhost:8080/v1/likes/stream?content_type=post&content_id=731b0395-4888-4822-b516-05b4b7bf2089"
```

## Local development

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings -A dead_code
cargo test
```

Bring up dependencies only:

```bash
docker compose up -d postgres redis mock-profile-api mock-post-api mock-bonus-hunter-api mock-top-picks-api
```

Run the API locally:

```bash
cp .env.example .env
cargo run -p social-api
```

## Integration tests

The contract tests run against the docker-compose stack.

```bash
RUN_INTEGRATION=1 SOCIAL_API_BASE_URL=http://localhost:8080 \
  cargo test -p social-api --test e2e_http -- --nocapture --test-threads=1
```

The `--test-threads=1` flag is recommended because the integration suite exercises shared mutable state in the same compose stack.

## Bonus artifacts

### OpenAPI

- `openapi/social-api.openapi.yaml`

This describes the public REST surface, schemas, auth, common errors, and SSE endpoint shape.

### Proto definitions

- `proto/social_api/v1/social_api.proto`
- `proto/content/v1/content_catalog.proto`
- `proto/profile/v1/profile_auth.proto`

These are transport-ready bonus artifacts to show how API-to-API communication could move behind protobuf/gRPC without changing the domain layer.

### k6 scripts

- `k6/count_read_10k_rps.js`
- `k6/batch_counts_1k_rps.js`
- `k6/mixed_traffic.js`

See `k6/README.md` for usage.