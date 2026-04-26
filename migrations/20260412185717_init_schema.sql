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

CREATE TABLE IF NOT EXISTS transfer_history (
    id          BIGSERIAL PRIMARY KEY,
    sender_id   BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    recipient_id BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    amount      BIGINT NOT NULL,
    created_at  BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT
);

CREATE INDEX IF NOT EXISTS idx_transfer_sender    ON transfer_history(sender_id);
CREATE INDEX IF NOT EXISTS idx_transfer_recipient ON transfer_history(recipient_id);

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE TABLE service (
    uuid UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    creator_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    description TEXT DEFAULT '',
    balance BIGINT DEFAULT 0,
    url_img TEXT,
    reg_date BIGINT NOT NULL
);