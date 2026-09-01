//! TLS for the cameras' control API.
//!
//! Tapo cameras serve HTTPS with a self-signed certificate (`CN=TPRI-DEVICE`), so the
//! usual CA-based verification can't work. Instead of disabling verification, we pin the
//! certificate's SHA-256 fingerprint the first time we connect (trust on first use) and
//! refuse to talk to anything else afterwards. The handshake signature is still checked
//! against the pinned certificate's key, so a machine in the middle can't replay it.

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{
    AlgorithmIdentifier, CertificateDer, InvalidSignature, ServerName,
    SignatureVerificationAlgorithm, UnixTime, alg_id,
};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// SHA-256 fingerprint of a camera's TLS certificate.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CertFingerprint(pub [u8; 32]);

impl CertFingerprint {
    pub fn of(cert_der: &[u8]) -> Self {
        Self(Sha256::digest(cert_der).into())
    }

    /// Uppercase hex, the form shown to users and stored in settings.
    pub fn to_hex(&self) -> String {
        hex::encode_upper(self.0)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let bytes = hex::decode(s.trim()).ok()?;
        Some(Self(bytes.try_into().ok()?))
    }
}

impl fmt::Debug for CertFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CertFingerprint({})", self.to_hex())
    }
}

impl fmt::Display for CertFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Shared pin state: the expected fingerprint (if known) and the one actually seen.
#[derive(Debug, Default)]
pub(crate) struct PinState {
    expected: Option<CertFingerprint>,
    seen: Option<CertFingerprint>,
}

pub(crate) type SharedPin = Arc<Mutex<PinState>>;

pub(crate) fn new_pin(expected: Option<CertFingerprint>) -> SharedPin {
    Arc::new(Mutex::new(PinState {
        expected,
        seen: None,
    }))
}

/// The fingerprint seen in the last handshake.
pub(crate) fn seen(pin: &SharedPin) -> Option<CertFingerprint> {
    pin.lock().expect("pin lock").seen
}

/// If the last handshake failed because of a pin mismatch, the fingerprints involved.
pub(crate) fn mismatch(pin: &SharedPin) -> Option<(CertFingerprint, CertFingerprint)> {
    let state = pin.lock().expect("pin lock");
    match (state.expected, state.seen) {
        (Some(expected), Some(seen)) if expected != seen => Some((expected, seen)),
        _ => None,
    }
}

#[derive(Debug)]
struct PinningVerifier {
    pin: SharedPin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fingerprint = CertFingerprint::of(end_entity.as_ref());
        let mut state = self.pin.lock().expect("pin lock");
        state.seen = Some(fingerprint);
        match state.expected {
            Some(expected) if expected != fingerprint => Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// RSA PKCS#1 v1.5 / SHA-256 accepting keys from 1024 bits, which webpki refuses.
#[derive(Debug)]
struct LegacyRsaPkcs1Sha256;

impl SignatureVerificationAlgorithm for LegacyRsaPkcs1Sha256 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
            public_key,
        )
        .verify(message, signature)
        .map_err(|_| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::RSA_ENCRYPTION
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::RSA_PKCS1_SHA256
    }
}

/// The provider's signature algorithms, plus PKCS#1 v1.5 with 1024-bit RSA keys: some
/// camera firmware still uses 1024-bit certificates, which rustls rejects by default.
fn algorithms() -> WebPkiSupportedAlgorithms {
    static ALGORITHMS: OnceLock<WebPkiSupportedAlgorithms> = OnceLock::new();
    *ALGORITHMS.get_or_init(|| {
        let base = provider().signature_verification_algorithms;
        let legacy: &'static dyn SignatureVerificationAlgorithm = &LegacyRsaPkcs1Sha256;

        let mut all = base.all.to_vec();
        all.push(legacy);
        let mapping = base
            .mapping
            .iter()
            .map(|(scheme, algs)| {
                if *scheme == SignatureScheme::RSA_PKCS1_SHA256 {
                    let mut algs = algs.to_vec();
                    algs.push(legacy);
                    (*scheme, &*Box::leak(algs.into_boxed_slice()))
                } else {
                    (*scheme, *algs)
                }
            })
            .collect::<Vec<_>>();

        WebPkiSupportedAlgorithms {
            all: Box::leak(all.into_boxed_slice()),
            mapping: Box::leak(mapping.into_boxed_slice()),
        }
    })
}

/// A rustls client config that pins the server certificate through `pin`.
pub(crate) fn client_config(pin: SharedPin) -> Result<rustls::ClientConfig, rustls::Error> {
    let verifier = Arc::new(PinningVerifier {
        pin,
        algorithms: algorithms(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_hex_round_trip() {
        let fp = CertFingerprint::of(b"certificate bytes");
        assert_eq!(CertFingerprint::from_hex(&fp.to_hex()), Some(fp));
        assert_eq!(fp.to_hex().len(), 64);
        assert!(CertFingerprint::from_hex("abcd").is_none());
    }

    #[test]
    fn legacy_rsa_is_added_once() {
        let algs = algorithms();
        let base = provider().signature_verification_algorithms;
        assert_eq!(algs.all.len(), base.all.len() + 1);
        assert!(
            algs.supported_schemes()
                .contains(&SignatureScheme::RSA_PKCS1_SHA256)
        );
    }

    #[test]
    fn config_builds() {
        assert!(client_config(new_pin(None)).is_ok());
    }
}
