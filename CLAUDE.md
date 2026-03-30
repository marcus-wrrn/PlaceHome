# PlaceNet Home

## Directory Structure

```
placenet-home/
├── Cargo.toml
├── Cargo.lock
├── .env
├── migrations/                       ← 0001_ca_keys, 0002_device_certs
├── static/                           ← (upstream app content, not served by placenet-home)
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
    │   ├── gateway/
    │   │   ├── mod.rs                    ← GatewayService impl (TLS proxy + PlaceNet protocol dispatch)
    │   │   ├── manager.rs                ← registration only
    │   │   ├── tls.rs                    ← rustls ServerConfig builder
    │   │   └── handshake.rs              ← MqttBrokerageInfo + build_brokerage_info
    │   ├── mqtt_brokerage/
    │   │   ├── mod.rs                    ← MosquittoBrokerageService impl
    │   │   └── registration.rs           ← registration only
    │   ├── mqtt_client/
    │   │   ├── mod.rs                    ← MqttClientService impl
    │   │   └── manager.rs                ← registration only
    │   └── peer/
    │       └── mod.rs                    ← send_message() plain HTTP client to peer node
    └── rendering/
        ├── mod.rs
        └── startup_screen.rs
```

## Developer Instructions
- Always update directory in CLAUDE.md after adding/removing files
- See PLACENET.md for full project vision, protocol design, and architecture overview