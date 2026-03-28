use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// Compute `sha256-{base64}` integrity string for the given bytes.
pub fn sha256_integrity(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    let b64 = STANDARD.encode(hash);
    format!("sha256-{b64}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_integrity() {
        let data = b"hello world";
        let result = sha256_integrity(data);
        assert!(result.starts_with("sha256-"));
        assert_eq!(
            result,
            "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
        );
    }
}
