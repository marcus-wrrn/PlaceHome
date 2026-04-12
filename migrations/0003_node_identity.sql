CREATE TABLE IF NOT EXISTS node_identity (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    cert_pem   TEXT    NOT NULL,
    key_pem    TEXT    NOT NULL,
    issued_at  INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
