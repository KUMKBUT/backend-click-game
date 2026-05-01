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
    id           BIGSERIAL PRIMARY KEY,
    sender_id    BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    recipient_id BIGINT REFERENCES "user"(id) ON DELETE CASCADE,
    service_uuid TEXT   REFERENCES service(uuid) ON DELETE CASCADE,
    amount       BIGINT NOT NULL,
    created_at   BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,

    CONSTRAINT chk_one_recipient CHECK (
        (recipient_id IS NOT NULL AND service_uuid IS NULL) OR
        (recipient_id IS NULL     AND service_uuid IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_transfer_sender    ON transfer_history(sender_id);
CREATE INDEX IF NOT EXISTS idx_transfer_recipient ON transfer_history(recipient_id);
CREATE INDEX IF NOT EXISTS idx_transfer_service   ON transfer_history(service_uuid);

-- Таблица сервисов
CREATE TABLE service (
    uuid TEXT PRIMARY KEY,
    creator_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    description TEXT DEFAULT '',
    balance BIGINT DEFAULT 0,
    callback_url TEXT,
    url_img TEXT,
    reg_date BIGINT NOT NULL,
    maintenance BOOLEAN NOT NULL DEFAULT TRUE
);
CREATE TABLE IF NOT EXISTS service_transfer_history (
    id           BIGSERIAL PRIMARY KEY,
    sender_user_id    BIGINT REFERENCES "user"(id) ON DELETE CASCADE,
    recipient_user_id BIGINT REFERENCES "user"(id) ON DELETE CASCADE,
    service_uuid      TEXT NOT NULL REFERENCES service(uuid) ON DELETE CASCADE,
    amount            BIGINT NOT NULL,
    direction         TEXT NOT NULL CHECK (direction IN ('to_service', 'from_service')),
    created_at        BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,

    CONSTRAINT chk_direction_matches CHECK (
        (direction = 'to_service'   AND sender_user_id IS NOT NULL AND recipient_user_id IS NULL) OR
        (direction = 'from_service' AND sender_user_id IS NULL     AND recipient_user_id IS NOT NULL)
    )
);

CREATE INDEX ON service_transfer_history(sender_user_id);
CREATE INDEX ON service_transfer_history(recipient_user_id);
CREATE INDEX ON service_transfer_history(service_uuid);
