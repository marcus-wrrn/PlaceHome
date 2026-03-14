CREATE TABLE IF NOT EXISTS ca_keys (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    cert_pem TEXT    NOT NULL,
    key_pem  TEXT    NOT NULL
);
