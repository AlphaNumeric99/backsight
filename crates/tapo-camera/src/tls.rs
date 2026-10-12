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

/// A reader for just enough DER to find a certificate's public key.
struct Der<'a>(&'a [u8]);

impl<'a> Der<'a> {
    /// Reads one tag-length-value, returning the tag and the contents.
    fn tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let (&tag, rest) = self.0.split_first()?;
        let (&first, rest) = rest.split_first()?;
        let (len, rest) = if first < 0x80 {
            (first as usize, rest)
        } else {
            // Long form: the low bits say how many bytes hold the length.
            let count = (first & 0x7f) as usize;
            if count == 0 || count > 4 || rest.len() < count {
                return None;
            }
            let (bytes, rest) = rest.split_at(count);
            (
                bytes.iter().fold(0usize, |n, &b| (n << 8) | b as usize),
                rest,
            )
        };
        if rest.len() < len {
            return None;
        }
        let (value, remainder) = rest.split_at(len);
        self.0 = remainder;
        Some((tag, value))
    }

    fn sequence(&mut self) -> Option<Der<'a>> {
        match self.tlv()? {
            (0x30, value) => Some(Der(value)),
            _ => None,
        }
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        for _ in 0..count {
            self.tlv()?;
        }
        Some(())
    }
}

/// The `tbsCertificate` of an X.509 certificate, positioned at its first field.
fn tbs_certificate(cert_der: &[u8]) -> Option<Der<'_>> {
    Der(cert_der).sequence()?.sequence()
}

/// Whether this is an X.509 v1 certificate — the `version` field is `[0] EXPLICIT` and
/// absent on v1. webpki refuses to parse these at all.
fn is_x509_v1(cert_der: &[u8]) -> bool {
    match tbs_certificate(cert_der) {
        // A leading context-specific [0] means the version is present, so v2 or v3.
        Some(mut tbs) => !matches!(tbs.tlv(), Some((0xA0, _))),
        None => false,
    }
}

/// The subjectPublicKeyInfo of a certificate, as (algorithm identifier contents, key bits).
///
/// The algorithm identifier is returned as the *contents* of its SEQUENCE, which is how
/// [`SignatureVerificationAlgorithm::public_key_alg_id`] encodes it.
fn subject_public_key(cert_der: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut tbs = tbs_certificate(cert_der)?;
    // version [0] (optional), serialNumber, signature, issuer, validity, subject, then the key.
    let mut peek = Der(tbs.0);
    if let Some((0xA0, _)) = peek.tlv() {
        tbs.tlv()?;
    }
    tbs.skip(5)?;
    let mut spki = tbs.sequence()?;

    let (0x30, algorithm) = spki.tlv()? else {
        return None;
    };
    let (0x03, key) = spki.tlv()? else {
        return None;
    };
    // A BIT STRING starts with the count of unused bits in the final byte; keys use none.
    match key.split_first() {
        Some((0, bits)) => Some((algorithm, bits)),
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
        if is_x509_v1(cert.as_ref()) {
            return self.verify_against_pinned_key(message, cert, dss);
        }
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        if is_x509_v1(cert.as_ref()) {
            return self.verify_against_pinned_key(message, cert, dss);
        }
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

impl PinningVerifier {
    /// Checks the handshake signature using the key inside the certificate itself.
    ///
    /// Only used for X.509 v1 certificates, which webpki rejects outright
    /// (`UnsupportedCertVersion`) — some Tapo firmware issues them, so the standard path
    /// can't even read the key. This changes nothing about what is trusted: the pinned
    /// fingerprint covers these exact DER bytes and was already checked in
    /// `verify_server_cert`, and the signature is verified against the public key carried in
    /// that same certificate, using the provider's own vetted algorithms.
    fn verify_against_pinned_key(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let bad_encoding =
            || rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding);
        let (algorithm, key) = subject_public_key(cert.as_ref()).ok_or_else(bad_encoding)?;

        let candidates = self
            .algorithms
            .mapping
            .iter()
            .find(|(scheme, _)| *scheme == dss.scheme)
            .map(|(_, algs)| *algs)
            .ok_or(rustls::Error::PeerMisbehaved(
                rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme,
            ))?;

        let mut tried = 0;
        for alg in candidates {
            if alg.public_key_alg_id().as_ref() != algorithm {
                continue;
            }
            tried += 1;
            if alg.verify_signature(key, message, dss.signature()).is_ok() {
                return Ok(HandshakeSignatureValid::assertion());
            }
        }
        tracing::debug!(
            scheme = ?dss.scheme,
            candidates = candidates.len(),
            tried,
            key_len = key.len(),
            "no algorithm verified the handshake signature for a v1 certificate"
        );
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadSignature,
        ))
    }
}

fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// RSA PKCS#1 v1.5 accepting keys from 1024 bits, which webpki refuses.
#[derive(Debug)]
struct LegacyRsaPkcs1 {
    ring: &'static ring::signature::RsaParameters,
    signature_alg_id: AlgorithmIdentifier,
}

impl SignatureVerificationAlgorithm for LegacyRsaPkcs1 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        ring::signature::UnparsedPublicKey::new(self.ring, public_key)
            .verify(message, signature)
            .map_err(|_| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::RSA_ENCRYPTION
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        self.signature_alg_id
    }
}

/// Cameras have been seen signing the handshake with either digest; ring has no 1024-bit
/// SHA-384 variant, and SHA-1 is left out on purpose.
static LEGACY_RSA_SHA256: LegacyRsaPkcs1 = LegacyRsaPkcs1 {
    ring: &ring::signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
    signature_alg_id: alg_id::RSA_PKCS1_SHA256,
};
static LEGACY_RSA_SHA512: LegacyRsaPkcs1 = LegacyRsaPkcs1 {
    ring: &ring::signature::RSA_PKCS1_1024_8192_SHA512_FOR_LEGACY_USE_ONLY,
    signature_alg_id: alg_id::RSA_PKCS1_SHA512,
};

/// The provider's signature algorithms, plus PKCS#1 v1.5 with 1024-bit RSA keys: some
/// camera firmware still uses 1024-bit certificates, which rustls rejects by default.
fn algorithms() -> WebPkiSupportedAlgorithms {
    static ALGORITHMS: OnceLock<WebPkiSupportedAlgorithms> = OnceLock::new();
    *ALGORITHMS.get_or_init(|| {
        let base = provider().signature_verification_algorithms;
        let legacy: [(SignatureScheme, &'static dyn SignatureVerificationAlgorithm); 2] = [
            (SignatureScheme::RSA_PKCS1_SHA256, &LEGACY_RSA_SHA256),
            (SignatureScheme::RSA_PKCS1_SHA512, &LEGACY_RSA_SHA512),
        ];

        let mut all = base.all.to_vec();
        all.extend(legacy.iter().map(|(_, alg)| *alg));
        let mapping = base
            .mapping
            .iter()
            .map(
                |(scheme, algs)| match legacy.iter().find(|(s, _)| s == scheme) {
                    Some((_, alg)) => {
                        let mut algs = algs.to_vec();
                        algs.push(*alg);
                        (*scheme, &*Box::leak(algs.into_boxed_slice()))
                    }
                    None => (*scheme, *algs),
                },
            )
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
    fn legacy_rsa_is_added_for_both_digests() {
        let algs = algorithms();
        let base = provider().signature_verification_algorithms;
        assert_eq!(algs.all.len(), base.all.len() + 2);

        // Cameras have been seen signing with either; SHA-512 is what firmware 1.3.3 uses.
        for scheme in [
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA512,
        ] {
            assert!(algs.supported_schemes().contains(&scheme), "{scheme:?}");
            let candidates = algs
                .mapping
                .iter()
                .find(|(s, _)| *s == scheme)
                .map(|(_, a)| *a)
                .unwrap_or_default();
            assert!(
                candidates
                    .iter()
                    .any(|a| a.public_key_alg_id() == alg_id::RSA_ENCRYPTION
                        && a.signature_alg_id() == scheme_alg_id(scheme)),
                "no legacy RSA algorithm registered for {scheme:?}"
            );
        }
    }

    fn scheme_alg_id(scheme: SignatureScheme) -> AlgorithmIdentifier {
        match scheme {
            SignatureScheme::RSA_PKCS1_SHA256 => alg_id::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA512 => alg_id::RSA_PKCS1_SHA512,
            other => panic!("unexpected scheme {other:?}"),
        }
    }

    #[test]
    fn config_builds() {
        assert!(client_config(new_pin(None)).is_ok());
    }

    // Certificates taken from two C325WB cameras. The v1 one appeared after a firmware
    // update and is what webpki refuses to parse.
    const CERT_V1: &[u8] = include_bytes!("../tests/fixtures/tapo-cert-v1.der");
    const CERT_V3: &[u8] = include_bytes!("../tests/fixtures/tapo-cert-v3.der");

    /// 1.2.840.113549.1.1.1 (rsaEncryption) with NULL parameters.
    const RSA_ENCRYPTION_ALG: &[u8] = &[
        0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
    ];

    #[test]
    fn tells_x509_versions_apart() {
        assert!(is_x509_v1(CERT_V1), "the firmware-updated camera serves v1");
        assert!(!is_x509_v1(CERT_V3));
        // Nothing here may panic on rubbish.
        assert!(!is_x509_v1(b""));
        assert!(!is_x509_v1(b"\x30\x82"));
        assert!(!is_x509_v1(&[0x30, 0x03, 0x02, 0x01, 0x00]));
    }

    #[test]
    fn reads_the_public_key_from_a_v1_certificate() {
        let (algorithm, key) = subject_public_key(CERT_V1).expect("an RSA key");
        assert_eq!(algorithm, RSA_ENCRYPTION_ALG);
        assert_eq!(algorithm, alg_id::RSA_ENCRYPTION.as_ref());
        // An RSA-1024 SubjectPublicKey is a SEQUENCE of two INTEGERs, about 140 bytes.
        assert!(
            key.len() > 130 && key.len() < 180,
            "key was {} bytes",
            key.len()
        );
        assert_eq!(key[0], 0x30, "the key itself is a DER SEQUENCE");
    }

    #[test]
    fn reads_the_public_key_from_a_v3_certificate() {
        let (algorithm, key) = subject_public_key(CERT_V3).expect("an EC key");
        assert_ne!(algorithm, RSA_ENCRYPTION_ALG, "this camera uses ECDSA");
        // An uncompressed P-256 point: 0x04 then two 32-byte coordinates.
        assert_eq!(key.first(), Some(&0x04));
        assert_eq!(key.len(), 65);
    }

    #[test]
    fn malformed_certificates_return_none_rather_than_panicking() {
        assert!(subject_public_key(b"").is_none());
        assert!(subject_public_key(&[0x30, 0x80]).is_none());
        // A valid prefix that is then cut short.
        for cut in [10, 40, 100, 200, 400] {
            assert!(
                subject_public_key(&CERT_V1[..cut]).is_none(),
                "truncating to {cut} bytes should not parse"
            );
        }
    }

    #[test]
    fn fingerprints_match_what_the_cameras_present() {
        // Guards against the fixtures being replaced by something else by accident.
        assert_eq!(
            CertFingerprint::of(CERT_V1).to_hex(),
            "32D649F8F39B0ABFBACE857AD6DC05A5922C81ADC0C28E8D4BFD87D3109E4FAD"
        );
        assert_eq!(
            CertFingerprint::of(CERT_V3).to_hex(),
            "66508425B413907901762DF282ABD3B9CC02B38884CE6EC85E631352851691F9"
        );
    }
}
