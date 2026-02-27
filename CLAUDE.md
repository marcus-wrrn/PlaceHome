# PlaceNet Home

## Directory Structure

```
placenet-home/
├── Cargo.toml
├── Cargo.lock
├── .env
├── migrations/
└── src/
    ├── main.rs
    ├── lib.rs
    ├── config.rs
    ├── supervisor.rs
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