CREATE TABLE subscriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    branch TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('builds', 'news', 'both')),
    news_app_id INTEGER NOT NULL,
    channel_id TEXT NOT NULL,
    role_id TEXT,
    build_id TEXT,
    news_initialized INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 0,
    UNIQUE (app_id, branch, channel_id)
);

CREATE TABLE seen_articles (
    subscription_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    article_id TEXT NOT NULL,
    PRIMARY KEY (subscription_id, article_id)
);

CREATE TABLE outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subscription_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    event_key TEXT NOT NULL UNIQUE,
    channel_id TEXT NOT NULL,
    role_id TEXT,
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    delivered_at INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);
CREATE INDEX outbox_pending ON outbox(delivered_at, next_attempt);

CREATE TABLE state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
