use rcgen::{
    Certificate, CertificateParams, CertificateSigningRequestParams,
    DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use sqlx::SqlitePool;
use tracing::info;

const CA_CN: &str = "PlaceNet Root CA";

/// Live CA state held in memory after initialisation.
pub struct CaState {
    pub subject_cn: String,
    pub cert: Certificate,
    pub key_pair: KeyPair,
}

/// Load an existing CA from the database, or generate a new one and persist it.
pub async fn load_or_generate_ca(pool: &SqlitePool) -> Result<CaState, String> {
    if let Some(state) = load_ca_from_db(pool).await? {
        info!("Loading existing CA from database");
        Ok(state)
    } else {
        info!("No CA found — generating new root CA");
        generate_and_persist_ca(pool).await
    }
}

async fn load_ca_from_db(pool: &SqlitePool) -> Result<Option<CaState>, String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT cert_pem, key_pem FROM ca_keys WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query CA from database: {}", e))?;

    let Some((cert_pem, key_pem)) = row else {
        return Ok(None);
    };

    let key_pair = KeyPair::from_pem(&key_pem)
        .map_err(|e| format!("Failed to parse CA private key: {}", e))?;
    let params = CertificateParams::from_ca_cert_pem(&cert_pem)
        .map_err(|e| format!("Failed to parse CA certificate: {}", e))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("Failed to reconstruct CA cert: {}", e))?;

    Ok(Some(CaState { subject_cn: CA_CN.to_string(), cert, key_pair }))
}

async fn generate_and_persist_ca(pool: &SqlitePool) -> Result<CaState, String> {
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

    sqlx::query("INSERT INTO ca_keys (id, cert_pem, key_pem) VALUES (1, ?, ?)")
        .bind(&cert_pem)
        .bind(&key_pem)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to persist CA to database: {}", e))?;

    info!("Root CA stored in database");

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
