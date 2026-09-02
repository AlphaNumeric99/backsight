//! Camera-side hashing and encryption (mirrors what real cameras do).

use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use md5::Md5;
use sha2::{Digest, Sha256};

pub type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
pub type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

pub fn sha256_upper(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex::encode_upper(hasher.finalize())
}

pub fn sha256_hex_upper(text: &str) -> String {
    hex::encode_upper(Sha256::digest(text.as_bytes()))
}

pub fn md5_hex(text: &str) -> String {
    hex::encode(Md5::digest(text.as_bytes()))
}

pub fn md5_bytes(text: &str) -> [u8; 16] {
    Md5::digest(text.as_bytes()).into()
}

pub fn random_hex(bytes: usize, upper: bool) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator");
    if upper {
        hex::encode_upper(buf)
    } else {
        hex::encode(buf)
    }
}

/// Session key or IV for the secure control channel.
pub fn derive_key(label: &str, cnonce: &str, hashed: &str, nonce: &str) -> [u8; 16] {
    let hashed_key = sha256_upper(&[cnonce.as_bytes(), hashed.as_bytes(), nonce.as_bytes()]);
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(cnonce.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(hashed_key.as_bytes());
    hasher.finalize()[..16].try_into().expect("16 bytes")
}

pub fn encrypt(key: [u8; 16], iv: [u8; 16], data: &[u8]) -> Vec<u8> {
    Aes128CbcEnc::new(&key.into(), &iv.into()).encrypt_padded_vec::<Pkcs7>(data)
}

pub fn decrypt(key: [u8; 16], iv: [u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    Aes128CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_vec::<Pkcs7>(data)
        .ok()
}
