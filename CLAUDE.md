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
    │   ├── mosquitto.rs
    │   └── mosquitto_client.rs
    └── rendering/
        ├── mod.rs
        └── startup_screen.rs
```

## Developer Instructions
- Always update directory in CLAUDE.md after adding/removing files