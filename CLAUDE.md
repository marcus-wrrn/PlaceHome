# PlaceNet Home

## Directory Structure

```
placenet-home/
├── Cargo.toml
├── Cargo.lock
├── .env
├── migrations/                       ← 0001_ca_keys, 0002_device_certs
├── static/                           ← served at GET /static/*
│   └── index.html
└── src/
    ├── main.rs
    ├── lib.rs
    ├── config.rs
    ├── supervisor.rs
    ├── infra/
    │   ├── mod.rs
    │   └── ca/
    │       ├── mod.rs                    ← CaService impl + init/sign_csr
    │       ├── manager.rs                ← registration only
    │       └── operations.rs             ← root CA generation, CSR signing logic
    ├── services/
    │   ├── mod.rs
    │   ├── capabilities.rs
    │   ├── http/
    │   │   ├── mod.rs                    ← HttpService impl
    │   │   ├── manager.rs                ← registration only
    │   │   ├── routes.rs                 ← route handlers (POST /)
    │   │   └── handshake.rs              ← DeviceInfo struct + TLS handshake logic
    │   ├── mqtt_brokerage/
    │   │   ├── mod.rs                    ← MosquittoBrokerageService impl
    │   │   └── registration.rs           ← registration only
    │   └── mqtt_client/
    │       ├── mod.rs                    ← MqttClientService impl
    │       └── manager.rs                ← registration only
    └── rendering/
        ├── mod.rs
        └── startup_screen.rs
```

## Developer Instructions
- Always update directory in CLAUDE.md after adding/removing files
- See PLACENET.md for full project vision, protocol design, and architecture overview