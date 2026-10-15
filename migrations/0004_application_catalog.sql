CREATE TABLE applications (
    app_id INTEGER PRIMARY KEY CHECK (app_id BETWEEN 1 AND 4294967295),
    name TEXT NOT NULL,
    metadata_json TEXT,
    metadata_updated_at INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO applications(app_id, name)
SELECT app_id, name FROM subscriptions
WHERE app_id BETWEEN 1 AND 4294967295 AND name <> ''
ORDER BY id DESC;
