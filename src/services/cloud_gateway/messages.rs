use serde::{Deserialize, Serialize};

/// Frames exchanged with the PlaceNet cloud gateway over the WebSocket
/// connection. Mirrors the `GatewayMessage` type defined in the gateway crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayMessage {
    /// Sent by this server on connect to announce its identity.
    Register { server_url: String },

    /// Request a relay session with another registered server.
    Connect { target: String },

    /// Received when another server has requested a session with us.
    ConnectRequest { from: String },

    /// A relay frame, either outbound (us → gateway) or inbound (gateway → us).
    Relay {
        from: String,
        to: String,
        payload: serde_json::Value,
    },

    /// Acknowledgement or error response from the gateway.
    Ack {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}
