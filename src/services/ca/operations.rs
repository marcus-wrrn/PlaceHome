use std::path::Path;
use rcgen::{
    Certificate, CertificateParams, CertificateSigningRequestParams,
    DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use tokio::fs;
use tracing::info;

const CA_CERT_PATH: &str = "certs/ca.crt";
const CA_KEY_PATH: &str = "certs/ca.key";
const CA_CN: &str = "PlaceNet Root CA";

/// Live CA state held in memory after initialisation.
pub struct CaState {
    pub subject_cn: String,
    pub cert: Certificate,
    pub key_pair: KeyPair,
}

/// Load an existing CA from disk, or generate a new one and persist it.
pub async fn load_or_generate_ca() -> Result<CaState, String> {
    if Path::new(CA_CERT_PATH).exists() && Path::new(CA_KEY_PATH).exists() {
        info!("Loading existing CA from {}", CA_CERT_PATH);
        load_ca_from_disk().await
    } else {
        info!("No CA found — generating new root CA");
        generate_and_persist_ca().await
    }
}

async fn load_ca_from_disk() -> Result<CaState, String> {
    let cert_pem = fs::read_to_string(CA_CERT_PATH)
        .await
        .map_err(|e| format!("Failed to read {}: {}", CA_CERT_PATH, e))?;
    let key_pem = fs::read_to_string(CA_KEY_PATH)
        .await
        .map_err(|e| format!("Failed to read {}: {}", CA_KEY_PATH, e))?;

    let key_pair = KeyPair::from_pem(&key_pem)
        .map_err(|e| format!("Failed to parse CA private key: {}", e))?;
    let params = CertificateParams::from_ca_cert_pem(&cert_pem)
        .map_err(|e| format!("Failed to parse CA certificate: {}", e))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("Failed to reconstruct CA cert: {}", e))?;

    Ok(CaState { subject_cn: CA_CN.to_string(), cert, key_pair })
}

async fn generate_and_persist_ca() -> Result<CaState, String> {
    fs::create_dir_all("certs")
        .await
        .map_err(|e| format!("Failed to create certs/ directory: {}", e))?;

    let key_pair = KeyPair::generate()
        .map_err(|e| format!("Failed to generate CA key pair: {}", e))?;

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, CA_CN);
    dn.push(DnType::OrganizationName, "PlaceNet");
    params.distinguished_name = dn;

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("Failed to self-sign CA certificate: {}", e))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    fs::write(CA_CERT_PATH, &cert_pem)
        .await
        .map_err(|e| format!("Failed to write {}: {}", CA_CERT_PATH, e))?;
    fs::write(CA_KEY_PATH, &key_pem)
        .await
        .map_err(|e| format!("Failed to write {}: {}", CA_KEY_PATH, e))?;

    info!("Root CA written to {} / {}", CA_CERT_PATH, CA_KEY_PATH);

    Ok(CaState { subject_cn: CA_CN.to_string(), cert, key_pair })
}

/// Sign a PEM-encoded CSR with the loaded root CA.
/// Returns the signed certificate as PEM.
pub fn sign_csr(ca: &CaState, csr_pem: &str) -> Result<String, String> {
    let csr = CertificateSigningRequestParams::from_pem(csr_pem)
        .map_err(|e| format!("Invalid CSR: {}", e))?;

    let signed = csr
        .signed_by(&ca.cert, &ca.key_pair)
        .map_err(|e| format!("Failed to sign CSR: {}", e))?;

    Ok(signed.pem())
}
