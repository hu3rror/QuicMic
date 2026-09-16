use base64::Engine;
use ring::digest::{digest, SHA256};
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use tracing::info;
use wtransport::Identity;

/// Directory where generated certificates are stored (only used with --dump-certs).
const CERTS_DIR: &str = "certs";

/// Holds the generated TLS identity material and its SHA-256 fingerprint.
#[derive(Clone)]
pub struct TlsIdentity {
    pub cert_pem: String,
    pub key_pem: String,
    pub cert_der: Vec<u8>,
    /// Base64-encoded SHA-256 hash (used by the JS client for serverCertificateHashes).
    pub cert_hash_base64: String,
}

/// Generate a fresh ECDSA P-256 self-signed certificate for the given LAN IP.
///
/// The certificate is generated completely in memory using wtransport's
/// built-in self-signed builder. It includes both the IP address and
/// "localhost" as Subject Alternative Names.
pub fn generate_identity(lan_ip: IpAddr) -> anyhow::Result<(Identity, TlsIdentity)> {
    let san_ip = lan_ip.to_string();
    let wt_identity = Identity::self_signed(["localhost", &san_ip])
        .map_err(|e| anyhow::anyhow!("Failed to generate self-signed identity: {:?}", e))?;

    // Extract the leaf certificate from the chain
    let cert_chain = wt_identity.certificate_chain();
    let cert = cert_chain.as_slice().first().ok_or_else(|| {
        anyhow::anyhow!("Self-signed identity generated an empty certificate chain")
    })?;

    let cert_pem = cert.to_pem();
    let cert_der = cert.der().to_vec();
    let key_pem = wt_identity.private_key().to_secret_pem();

    let tls_identity = TlsIdentity {
        cert_pem,
        key_pem,
        cert_hash_base64: hash_base64(&cert_der),
        cert_der,
    };

    info!(
        hash = %tls_identity.cert_hash_base64,
        "Generated ECDSA P-256 in-memory certificate (14-day lifetime)"
    );

    Ok((wt_identity, tls_identity))
}

/// Rebuild a TLS identity from PEM strings — the persisted-identity load path
/// (ADR-0015). Parses the PEM back into a `wtransport::Identity` and validates
/// the material the same way the server would at runtime:
///
/// - the PEM must parse as a certificate and a private key,
/// - the private key must be PKCS#8 (what `Identity::self_signed` emits),
/// - a rustls `ServerConfig` must accept the pair (catches a cert/key mismatch,
///   e.g. a hand-edited `identity.json`).
///
/// The caller (the persistence layer) treats any error here as "corrupt":
/// quarantine + regenerate.
pub fn from_pem(cert_pem: &str, key_pem: &str) -> anyhow::Result<(Identity, TlsIdentity)> {
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::PrivateKeyDer;
    use x509_parser::certificate::X509Certificate;
    use x509_parser::prelude::FromDer;

    let cert_der = rustls_pki_types::CertificateDer::from_pem_slice(cert_pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate PEM: {e}"))?;
    let key_der = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to parse private key PEM: {e}"))?;
    let pkcs8 = match key_der {
        PrivateKeyDer::Pkcs8(pkcs8) => pkcs8,
        _ => anyhow::bail!("Persisted private key is not PKCS#8"),
    };
    let key_der: PrivateKeyDer = pkcs8.clone_key().into();

    // Validate the pair the same way the server would at runtime: the rustls
    // config build rejects unparseable material, and a byte-exact SPKI comparison
    // catches a cert/key that do not belong together (which the config build
    // alone would accept for two same-algorithm keys).
    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key_der)
        .map_err(|e| anyhow::anyhow!("Unsupported persisted private key: {e}"))?;
    let key_spki = signing_key
        .public_key()
        .ok_or_else(|| anyhow::anyhow!("Persisted private key has no public key"))?;
    let (_, parsed_cert) = X509Certificate::from_der(cert_der.as_ref())
        .map_err(|e| anyhow::anyhow!("Persisted certificate DER is invalid: {e}"))?;
    if parsed_cert.public_key().raw != key_spki.as_ref() {
        anyhow::bail!("Persisted certificate and key do not match");
    }
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .map_err(|e| anyhow::anyhow!("Persisted certificate cannot serve TLS: {e}"))?;

    // Rebuild the wtransport identity from the parsed DER.
    let leaf = wtransport::tls::Certificate::from_der(cert_der.as_ref().to_vec())
        .map_err(|e| anyhow::anyhow!("Persisted certificate DER is invalid: {e}"))?;
    let chain = wtransport::tls::CertificateChain::new(vec![leaf]);
    let key = wtransport::tls::PrivateKey::from_der_pkcs8(pkcs8.secret_pkcs8_der().to_vec());
    let wt_identity = Identity::new(chain, key);

    let cert_der = cert_der.as_ref().to_vec();
    let tls_identity = TlsIdentity {
        cert_pem: cert_pem.to_string(),
        key_pem: key_pem.to_string(),
        cert_der: cert_der.clone(),
        cert_hash_base64: hash_base64(&cert_der),
    };

    Ok((wt_identity, tls_identity))
}

/// Write the identity's PEM/DER files to the `certs/` directory for inspection
/// (only used with `--dump-certs`; debug aid for the pinning handshake).
pub fn dump_certs(identity: &TlsIdentity) -> anyhow::Result<()> {
    let dir = Path::new(CERTS_DIR);
    fs::create_dir_all(dir)?;
    fs::write(dir.join("cert.pem"), &identity.cert_pem)?;
    fs::write(dir.join("key.pem"), &identity.key_pem)?;
    fs::write(dir.join("cert.der"), &identity.cert_der)?;
    info!("Certificate files dumped to certs/ directory");
    Ok(())
}

/// Base64-encoded SHA-256 of the DER encoding of a certificate.
fn hash_base64(cert_der: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(digest(&SHA256, cert_der).as_ref())
}

/// Build RustlsConfig for axum-server from the TLS identity.
pub async fn build_rustls_config_async(
    identity: &TlsIdentity,
) -> anyhow::Result<axum_server::tls_rustls::RustlsConfig> {
    let config = axum_server::tls_rustls::RustlsConfig::from_pem(
        identity.cert_pem.as_bytes().to_vec(),
        identity.key_pem.as_bytes().to_vec(),
    )
    .await?;
    Ok(config)
}
