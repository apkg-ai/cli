use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// SPKI/PKIX DER prefix for Ed25519 public keys (12 bytes).
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Extract 32 raw Ed25519 public key bytes from a base64-encoded key.
///
/// Accepts either:
/// - Raw 32-byte key (base64 decodes to 32 bytes)
/// - SPKI-wrapped 44-byte key (12-byte prefix + 32-byte key)
pub fn extract_ed25519_pubkey(base64_key: &str) -> Result<[u8; 32], String> {
    let bytes = STANDARD
        .decode(base64_key)
        .map_err(|e| format!("Invalid base64 key: {e}"))?;

    match bytes.len() {
        32 => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        44 => {
            if bytes[..12] != ED25519_SPKI_PREFIX {
                return Err("44-byte key does not have expected SPKI prefix".to_string());
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes[12..]);
            Ok(key)
        }
        n => Err(format!(
            "Unexpected key length: {n} bytes (expected 32 or 44)"
        )),
    }
}

/// Verify an Ed25519 signature.
///
/// - `public_key_bytes` — 32-byte raw Ed25519 public key
/// - `signature_b64` — base64-encoded 64-byte signature
/// - `integrity` — the signed message (the integrity string)
///
/// Returns `Ok(true)` if the signature is valid, `Ok(false)` if the signature
/// is structurally valid but does not match, or `Err` if inputs are malformed.
pub fn verify_signature(
    public_key_bytes: &[u8; 32],
    signature_b64: &str,
    integrity: &str,
) -> Result<bool, String> {
    let sig_bytes = STANDARD
        .decode(signature_b64)
        .map_err(|e| format!("Invalid base64 signature: {e}"))?;

    let sig =
        Signature::from_slice(&sig_bytes).map_err(|e| format!("Invalid signature length: {e}"))?;

    let verifying_key = VerifyingKey::from_bytes(public_key_bytes)
        .map_err(|e| format!("Invalid public key: {e}"))?;

    match verifying_key.verify(integrity.as_bytes(), &sig) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn test_keypair() -> (SigningKey, VerifyingKey) {
        let secret_bytes: [u8; 32] = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        (signing_key, verifying_key)
    }

    #[test]
    fn test_extract_raw_ed25519_key() {
        let (_, vk) = test_keypair();
        let raw_b64 = STANDARD.encode(vk.as_bytes());
        let extracted = extract_ed25519_pubkey(&raw_b64).unwrap();
        assert_eq!(extracted, *vk.as_bytes());
    }

    #[test]
    fn test_extract_spki_ed25519_key() {
        let (_, vk) = test_keypair();
        let mut spki = Vec::with_capacity(44);
        spki.extend_from_slice(&ED25519_SPKI_PREFIX);
        spki.extend_from_slice(vk.as_bytes());
        let spki_b64 = STANDARD.encode(&spki);
        let extracted = extract_ed25519_pubkey(&spki_b64).unwrap();
        assert_eq!(extracted, *vk.as_bytes());
    }

    #[test]
    fn test_extract_invalid_key() {
        let bad_b64 = STANDARD.encode([0u8; 16]);
        let result = extract_ed25519_pubkey(&bad_b64);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unexpected key length"));
    }

    #[test]
    fn test_verify_valid_signature() {
        let (sk, vk) = test_keypair();
        let message = "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek=";
        let sig = sk.sign(message.as_bytes());
        let sig_b64 = STANDARD.encode(sig.to_bytes());

        let result = verify_signature(vk.as_bytes(), &sig_b64, message).unwrap();
        assert!(result);
    }

    #[test]
    fn test_verify_invalid_signature() {
        let (sk, vk) = test_keypair();
        let message = "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek=";
        let sig = sk.sign(message.as_bytes());
        let mut sig_bytes = sig.to_bytes();
        sig_bytes[0] ^= 0xff; // tamper
        let sig_b64 = STANDARD.encode(sig_bytes);

        let result = verify_signature(vk.as_bytes(), &sig_b64, message).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_verify_wrong_key() {
        let (sk, _) = test_keypair();
        let message = "sha256-abc";
        let sig = sk.sign(message.as_bytes());
        let sig_b64 = STANDARD.encode(sig.to_bytes());

        // Use a different key
        let other_secret: [u8; 32] = [
            99, 98, 97, 96, 95, 94, 93, 92, 91, 90, 89, 88, 87, 86, 85, 84, 83, 82, 81, 80, 79, 78,
            77, 76, 75, 74, 73, 72, 71, 70, 69, 68,
        ];
        let other_vk = SigningKey::from_bytes(&other_secret).verifying_key();

        let result = verify_signature(other_vk.as_bytes(), &sig_b64, message).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_verify_wrong_message() {
        let (sk, vk) = test_keypair();
        let message = "sha256-correct";
        let sig = sk.sign(message.as_bytes());
        let sig_b64 = STANDARD.encode(sig.to_bytes());

        let result = verify_signature(vk.as_bytes(), &sig_b64, "sha256-wrong").unwrap();
        assert!(!result);
    }
}
