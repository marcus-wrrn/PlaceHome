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