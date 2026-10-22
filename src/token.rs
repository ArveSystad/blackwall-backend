use axum::http::HeaderValue;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// `SHA-256` over the write token's header bytes (contract §3.3).
pub fn hash(token: &[u8]) -> [u8; 32] {
    Sha256::digest(token).into()
}

/// Whether `provided` hashes to `stored`, compared in constant time. A missing
/// token never matches.
pub fn matches(provided: Option<&HeaderValue>, stored: &[u8]) -> bool {
    match provided {
        Some(token) => hash(token.as_bytes()).ct_eq(stored).into(),
        None => false,
    }
}
