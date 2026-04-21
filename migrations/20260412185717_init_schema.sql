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

CREATE TABLE user_upgrades (
    user_id BIGINT REFERENCES "user"(id) ON DELETE CASCADE,
    upgrade_id TEXT NOT NULL,
    level INTEGER DEFAULT 0,
    PRIMARY KEY (user_id, upgrade_id)
);