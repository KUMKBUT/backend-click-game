CREATE TABLE IF NOT EXISTS "user" (
    id         BIGSERIAL PRIMARY KEY,
    first_name TEXT NOT NULL DEFAULT 'Player',
    balance    BIGINT NOT NULL DEFAULT 0,
    per_click  BIGINT NOT NULL DEFAULT 1,
    auto_click BIGINT NOT NULL DEFAULT 0,
    last_sync  BIGINT NOT NULL DEFAULT 0,
    token      TEXT NOT NULL UNIQUE
);

CREATE INDEX IF NOT EXISTS idx_user_token ON "user"(token);