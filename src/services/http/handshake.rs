use std::sync::Arc;

use rustls::ClientConfig;
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::{error, info};

/// mDNS settings advertised by the device.
#[derive(Debug, Deserialize)]
pub struct MdnsConfig {
    pub hostname: String,
    pub port: u16,
}

/// Registration payload sent by a device on the `POST /` route.
#[derive(Debug, Deserialize)]
pub struct DeviceInfo {
    pub public_key: String,
    /// `host:port` the device is reachable on (e.g. `"192.168.1.42:8883"`).
    pub address: String,
    pub mdns: MdnsConfig,
}

/// Initiate a TLS handshake to `device.address` using the system/webpki roots.
///
/// This opens the TCP connection and completes the TLS handshake so the
/// caller receives a live, authenticated [`tokio_rustls::client::TlsStream`].
/// The stream is returned so the caller can drive the secure channel; any
/// error is logged and propagated.
pub async fn initiate_tls_handshake(
    device: &DeviceInfo,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    // Build a TLS client config that trusts the standard WebPKI roots.
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(tls_config));

    // Parse "host:port" from the device address.
    let (host, _port) = device
        .address
        .rsplit_once(':')
        .ok_or_else(|| format!("Invalid device address '{}': expected host:port", device.address))?;

    let server_name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|e| format!("Invalid server name '{}': {}", host, e))?;

    info!("Opening TCP connection to {}", device.address);
    let tcp = TcpStream::connect(&device.address)
        .await
        .map_err(|e| format!("TCP connect to {} failed: {}", device.address, e))?;

    info!("Performing TLS handshake with {}", device.address);
    let tls = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| {
            error!("TLS handshake with {} failed: {}", device.address, e);
            format!("TLS handshake failed: {}", e)
        })?;

    info!("TLS handshake with {} succeeded", device.address);
    Ok(tls)
}
