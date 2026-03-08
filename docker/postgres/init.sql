-- Initialize local development roles.
-- Writer role is created by POSTGRES_USER.

DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'social_read_replica') THEN
CREATE ROLE social_read_replica LOGIN PASSWORD 'social_password';
END IF;
END $$;

GRANT CONNECT ON DATABASE social_api TO social_read_replica;
GRANT USAGE ON SCHEMA public TO social_read_replica;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO social_read_replica;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO social_read_replica;