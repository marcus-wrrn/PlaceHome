# PlaceNet

## What is PlaceNet?

PlaceNet is an open source platform for building location-scoped community internets. It is a decentralized network where content and services are only accessible to people who are physically present in a given area.

The core idea: Toronto has a different PlaceNet than Ottawa. A business in Toronto is only discoverable and accessible to people physically in Toronto (or more specifically, physically nearby). You cannot access it from the other side of the world, and you cannot be gamed out of local relevance by global ad spend or algorithmic optimization.

PlaceNet is built in opposition to the attention economy. It is designed so that the only way to be relevant on the network is to actually be there.

---

## Motivation

The internet is poorly suited for local discovery. It optimizes for global reach and engagement, which structurally disadvantages local businesses, community spaces, and in-person culture. A restaurant competes with TikTok for attention. A local bookstore has no meaningful presence against Amazon.

PlaceNet inverts this. Because access is physically bounded by the protocol (not by policy), the network cannot be aggregated or gamed by remote actors. A business's PlaceNet presence is only valuable because of their physical location. That value cannot be replicated by a competitor in another city.

Target use cases:
- Personal websites and portfolios
- Local business discovery (restaurants, shops, services)
- Third spaces (libraries, community centers, cafes)
- Game servers requiring physical co-presence
- Ephemeral local chat and social spaces
- Community infrastructure and local communications

---

## How It Works

### Network Model

Each PlaceNet node runs `placenet-home`, the node software. Nodes are tied to a physical location and a WiFi network. Anyone on the network's WiFi is considered "present" and can access the node's content.

As neighboring nodes are deployed, they federate with each other, forming a city-scale network. Each city's network is independent. Nodes within a city can share a device registry and corroborate each other's presence claims.

A cloud server handles node discovery and federation bootstrapping, similar to how Tailscale uses STUN/DERP servers. It is a coordination layer only and is never in the data path once a connection is established.

### Proof of Presence (PoP) Protocol

The core security primitive is the **Proof of Presence** protocol. It verifies that a device is physically present before granting access.

**Handshake flow:**

1. A LoRa-based Access Point (currently T-Beam hardware, targeting all Meshtastic-compatible devices) broadcasts a short-lived random beacon key
2. A nearby device receives the beacon over LoRa
3. The device initiates an IP-based ClientHello embedding the beacon key, proving it received a signal that only physical proximity could provide
4. `placenet-home` verifies the beacon key is valid and recent, coordinating with other services as needed
5. `placenet-home` issues a CA signature, client ID, and PEM certificate, establishing mTLS
6. A final HMAC is sent back over LoRa to complete the bind, proving the IP client and LoRa device are co-located
7. On each key rotation, the HMAC step is repeated over LoRa to re-prove presence

The LoRa channel is used minimally to avoid overuse of the 900 MHz spectrum. The majority of the handshake is IP-based.

### Replay Attack Mitigation

Beacon keys are time-bounded using a TOTP-style sliding window:

- The T-Beam broadcasts `beacon = HMAC(device_secret, timestamp_slot)` where `timestamp_slot` is a coarse time window (e.g. 30 seconds)
- GPS time on the T-Beam provides accurate clock synchronization without network dependency
- The final bind HMAC is `HMAC(device_secret, timestamp_slot || session_id)` where `session_id` is server-generated and single-use
- On rotation, the HMAC includes the previous session's cert fingerprint to chain rotations and prevent injection attacks

### Certificate Rotation

mTLS certificates rotate on a short cadence (target: ~5 minutes) even during persistent connections and WebSockets. This ensures that presence is continuously re-verified rather than checked once at connection time. Each rotation requires a new LoRa HMAC to re-bind the IP session to physical presence.

---

## Architecture

```
placenet-home (node software)
├── Serves local content over HTTPS (mTLS verified)
├── Runs local MQTT broker (for communication between AP's and outside servers)
├── Manages CA for its WiFi network
├── Federates with neighboring nodes
└── LoRa AP = network boundary marker + inter-node trust anchor

Cloud server
└── Node registry and federation bootstrap (coordination only, not data path)
```

### Key Components

- **CA Infrastructure:** issues and rotates client certificates, backed by SQLite
- **MQTT Broker:** handles real-time messaging and ephemeral social features
- **HTTPS Server:** serves content to verified-present devices over mTLS
- **PoP Handshake:** verifies physical presence before issuing credentials
- **LoRa AP:** separate hardware project (T-Beam / Meshtastic devices), broadcasts presence beacons

---

## Discovery and Browsing

PlaceNet uses a map-based discovery model. You don't navigate *to* a site, you encounter it by being somewhere. Nearby nodes appear on a map. Walking into a bar puts you on that bar's network. The content finds you based on where you are.

DNS-style global addressing is intentionally absent. Addressing is local:
- Each node has a human-readable local ID
- mDNS (`.local`) handles LAN-level resolution
- No global namespace, no SEO, no cross-network discoverability from outside PlaceNet

---

## Social Layer

PlaceNet includes ephemeral, location-scoped social features:

- **Local chat:** everyone in a physical space shares a chat room. Every person you interact with should be within a 5-10km range.
- **Presence feed:** see who and what is nearby
- **Node content:** businesses and individuals publish to their node; content is only visible to physically present users

The goal of the social layer is to strengthen community involvement and form stronger relationships with people who live near us.


It is a fundamental belief of the project that strong and healthy communities are built on shared environments **not** shared interest. The internet is an unnaturual means of interacting with the world that we as people did not evolve for. PlaceNet is designed with the intent of modelling how information naturally spreads through pre-internet and wide area broadcasting society. 

## Economic Model

PlaceNet aims to support businesses that offer in-person experiences, physical goods and community services. PlaceNet incentivizes local advertisement and the development of public infrastructure to support it. Businesses do not have to rely on centralized platforms such as Instagram, TikTok or YouTube to advertise to potential customers. Due to the nature of PlaceNet anybody who can see your ad is within driving distance of your business, with the benefit of not having to compete with algorithmic content and large global brands.

If a large brand wishes to advertise in an area they will need to construct supporting infrastructure

## Bot Resistance

PlaceNet is inherently bot resistant. 

---

## Hardware

- **LoRa AP:** Currently LILYGO T-Beam. Target: all Meshtastic-compatible devices.
- **Node hardware:** Any machine capable of running the `placenet-home` binary. Raspberry Pi is the target for accessible deployment.
- **Client:** Any device on the node's WiFi network. No special hardware required for end users.

---

## Design Principles

- **Location is the access control mechanism.** How password and account protection should be implemented is currently an open question 
- **Physical presence cannot be faked at scale.** The LoRa + HMAC binding provides hardware-grounded proof. Future versions of the protocol may rely on consensus algorithms
- **The protocol enforces the philosophy.** Location-scoping is structural, not a policy that can be changed by a platform owner.
- **Cloud is coordination-only.** No data passes through the cloud server once P2P is established.
- **Non-technical operators.** The long-term goal is a binary a bar owner can run on a Raspberry Pi with minimal setup.

---

## Status

Early development. Core infrastructure (CA, MQTT, mTLS, HTTP server) is in progress. PoP handshake is partially implemented. LoRa AP is a separate in-progress hardware project.

This is a multi-year open source project. There is no commercial intent.
