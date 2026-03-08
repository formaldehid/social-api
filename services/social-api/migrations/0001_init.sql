-- Initial schema for Social API.
-- Note: Like/unlike endpoints are scaffolded in this first commit, but schema is
-- created now to support readiness checks and future commits.

CREATE TABLE IF NOT EXISTS likes (
    user_id TEXT NOT NULL,
    content_type TEXT NOT NULL,
    content_id UUID NOT NULL,
    liked_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, content_type, content_id)
);

-- Enables fast fallback counts (and cache warming) without COUNT(*) on likes.
CREATE TABLE IF NOT EXISTS like_counts (
    content_type TEXT NOT NULL,
    content_id UUID NOT NULL,
    count BIGINT NOT NULL DEFAULT 0,
    seq BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (content_type, content_id)
);

-- Time-bucketed aggregates for scalable leaderboards.
CREATE TABLE IF NOT EXISTS like_buckets_hourly (
    bucket_start TIMESTAMPTZ NOT NULL,
    content_type TEXT NOT NULL,
    content_id UUID NOT NULL,
    count BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (bucket_start, content_type, content_id)
);

-- Query patterns:
-- 1) By content item (status checks / aggregates)
CREATE INDEX IF NOT EXISTS idx_likes_by_content ON likes (content_type, content_id);

-- 2) By user, most-recent first (cursor-based pagination)
CREATE INDEX IF NOT EXISTS idx_likes_by_user_recent ON likes (user_id, liked_at DESC, content_type, content_id);