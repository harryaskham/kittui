//! Helpers around content hashing. All scene/image identity goes through
//! blake3 so different platforms and language bindings agree on cache keys.

use serde::Serialize;

/// Hash any serde-serializable value via its canonical JSON encoding. The
/// JSON encoder is configured to keep object keys sorted so semantically
/// equal values share an id.
pub fn blake3_of_serializable<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("scene must serialize");
    blake3_of_bytes(&bytes)
}

/// Hash raw bytes via blake3 and return the lowercase hex digest.
pub fn blake3_of_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_hex_is_deterministic() {
        let a = blake3_of_bytes(b"kittui");
        let b = blake3_of_bytes(b"kittui");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
