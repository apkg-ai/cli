use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

use crate::api::client::ApiClient;
use crate::config::keys;
use crate::error::AppError;
use crate::util::display;

pub enum KeyAction<'a> {
    Generate {
        name: &'a str,
    },
    List {
        local: bool,
    },
    Register {
        name: Option<&'a str>,
        key_id: Option<&'a str>,
    },
    Revoke {
        key_id: &'a str,
        reason: &'a str,
        message: Option<&'a str>,
    },
    Rotate {
        old_key_id: &'a str,
        name: Option<&'a str>,
    },
}

fn compute_key_id(public_key_bytes: &[u8]) -> String {
    let hash = Sha256::digest(public_key_bytes);
    let encoded = BASE64.encode(hash);
    format!("SHA256:{encoded}")
}

pub async fn run(action: KeyAction<'_>, registry: Option<&str>) -> Result<(), AppError> {
    match action {
        KeyAction::Generate { name } => generate(name),
        KeyAction::List { local } => list(registry, local).await,
        KeyAction::Register { name, key_id } => register(registry, name, key_id).await,
        KeyAction::Revoke {
            key_id,
            reason,
            message,
        } => revoke(registry, key_id, reason, message).await,
        KeyAction::Rotate { old_key_id, name } => rotate(registry, old_key_id, name).await,
    }
}

fn generate(name: &str) -> Result<(), AppError> {
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = signing_key.verifying_key();

    let public_key_b64 = BASE64.encode(public_key.as_bytes());
    let private_key_b64 = BASE64.encode(signing_key.to_bytes());
    let key_id = compute_key_id(public_key.as_bytes());

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let stored = keys::StoredKey {
        key_id: key_id.clone(),
        name: name.to_string(),
        public_key: public_key_b64.clone(),
        private_key: private_key_b64,
        created_at: now,
    };
    keys::save(&stored)?;

    display::success(&format!("Generated Ed25519 signing key: {key_id}"));
    display::label_value("Name", name);
    display::label_value("Public key", &public_key_b64);
    display::info("\nPrivate key stored in ~/.apkg/keys/");
    display::info("Register with the registry: apkg key register");

    Ok(())
}

async fn list(registry: Option<&str>, local: bool) -> Result<(), AppError> {
    if local {
        let local_keys = keys::list_local()?;
        if local_keys.is_empty() {
            display::info(
                "No local keys found. Generate one with: apkg key generate --name <name>",
            );
            return Ok(());
        }
        println!("{:<48}  {:<20}  CREATED", "KEY ID", "NAME");
        println!("{}", "-".repeat(84));
        for key in &local_keys {
            println!("{:<48}  {:<20}  {}", key.key_id, key.name, key.created_at);
        }
        display::info(&format!("\n{} local key(s)", local_keys.len()));
        return Ok(());
    }

    let client = ApiClient::new(registry)?;
    let resp = client.list_keys().await?;

    if resp.keys.is_empty() {
        display::info("No signing keys registered. Generate and register one:");
        display::info("  apkg key generate --name <name>");
        display::info("  apkg key register");
        return Ok(());
    }

    println!(
        "{:<48}  {:<20}  {:<10}  CREATED",
        "KEY ID", "NAME", "STATUS"
    );
    println!("{}", "-".repeat(96));
    for key in &resp.keys {
        println!(
            "{:<48}  {:<20}  {:<10}  {}",
            key.key_id, key.name, key.status, key.created_at
        );
    }
    display::info(&format!("\n{} registered key(s)", resp.keys.len()));

    Ok(())
}

async fn register(
    registry: Option<&str>,
    name: Option<&str>,
    key_id: Option<&str>,
) -> Result<(), AppError> {
    let local_keys = keys::list_local()?;
    if local_keys.is_empty() {
        return Err(AppError::Other(
            "No local keys found. Generate one first: apkg key generate --name <name>".into(),
        ));
    }

    let stored = if let Some(kid) = key_id {
        keys::load(kid)?.ok_or_else(|| AppError::Other(format!("Local key not found: {kid}")))?
    } else if local_keys.len() == 1 {
        local_keys
            .into_iter()
            .next()
            .expect("checked len == 1 above")
    } else {
        let items: Vec<String> = local_keys
            .iter()
            .map(|k| format!("{} ({})", k.key_id, k.name))
            .collect();
        let selection = dialoguer::Select::new()
            .with_prompt("Select a key to register")
            .items(&items)
            .interact()
            .map_err(|e| AppError::Other(format!("Input error: {e}")))?;
        local_keys
            .into_iter()
            .nth(selection)
            .expect("selection within bounds")
    };

    let key_name = name.unwrap_or(&stored.name);

    let client = ApiClient::new(registry)?;
    let resp = client.register_key(&stored.public_key, key_name).await?;

    display::success(&format!("Registered signing key: {}", resp.key_id));
    display::label_value("Name", &resp.name);
    display::label_value("Algorithm", &resp.algorithm);
    display::label_value("Created", &resp.created_at);

    Ok(())
}

async fn revoke(
    registry: Option<&str>,
    key_id: &str,
    reason: &str,
    message: Option<&str>,
) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let resp = client.revoke_key(key_id, reason, message).await?;

    display::success(&format!("Revoked signing key: {}", resp.key_id));
    display::label_value("Status", &resp.status);
    display::label_value("Revoked at", &resp.revoked_at);

    if keys::delete(key_id)? {
        display::info("Removed local key file");
    }

    Ok(())
}

async fn rotate(
    registry: Option<&str>,
    old_key_id: &str,
    name: Option<&str>,
) -> Result<(), AppError> {
    let old_key = keys::load(old_key_id)?.ok_or_else(|| {
        AppError::Other(format!(
            "Local key not found: {old_key_id}. Rotation requires the old private key to sign an attestation."
        ))
    })?;

    let old_private_bytes = BASE64
        .decode(&old_key.private_key)
        .map_err(|e| AppError::Other(format!("Failed to decode old private key: {e}")))?;
    let old_private_bytes: [u8; 32] = old_private_bytes
        .try_into()
        .map_err(|_| AppError::Other("Invalid private key length".into()))?;
    let old_signing_key = SigningKey::from_bytes(&old_private_bytes);

    let new_signing_key = SigningKey::generate(&mut OsRng);
    let new_public_key = new_signing_key.verifying_key();
    let new_public_key_b64 = BASE64.encode(new_public_key.as_bytes());
    let new_private_key_b64 = BASE64.encode(new_signing_key.to_bytes());
    let new_key_id = compute_key_id(new_public_key.as_bytes());

    // Old key signs over new public key bytes as attestation of rotation
    let attestation_sig = old_signing_key.sign(new_public_key.as_bytes());
    let attestation_b64 = BASE64.encode(attestation_sig.to_bytes());

    let client = ApiClient::new(registry)?;
    let resp = client
        .rotate_key(old_key_id, &new_public_key_b64, &attestation_b64)
        .await?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let key_name = name.unwrap_or(&old_key.name);
    let stored = keys::StoredKey {
        key_id: new_key_id,
        name: key_name.to_string(),
        public_key: new_public_key_b64,
        private_key: new_private_key_b64,
        created_at: now,
    };
    keys::save(&stored)?;
    keys::delete(old_key_id)?;

    display::success("Key rotated successfully");
    display::label_value("Old key", &resp.old_key_id);
    display::label_value("New key", &resp.new_key_id);
    display::label_value("Rotated at", &resp.rotated_at);

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use super::*;

    #[test]
    fn test_compute_key_id() {
        let key_id = compute_key_id(b"test-public-key");
        assert!(key_id.starts_with("SHA256:"));
        assert!(key_id.len() > 10);
    }

    #[test]
    fn test_generate_and_compute_key_id_deterministic() {
        let id1 = compute_key_id(b"same-bytes");
        let id2 = compute_key_id(b"same-bytes");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_compute_key_id_different_inputs() {
        let id1 = compute_key_id(b"key-a");
        let id2 = compute_key_id(b"key-b");
        assert_ne!(id1, id2);
    }

    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
    }

    #[test]
    fn test_generate_key() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        // Create the keys directory
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        let result = generate("test-key");
        assert!(result.is_ok());
        // Verify a key file was created
        let keys_dir = tmp.path().join(".apkg").join("keys");
        let entries: Vec<_> = std::fs::read_dir(&keys_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_list_local_keys() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        // Generate a key first
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        generate("local-key").unwrap();

        // Now test listing
        let local_keys = keys::list_local().unwrap();
        assert_eq!(local_keys.len(), 1);
        assert_eq!(local_keys[0].name, "local-key");
    }

    #[test]
    fn test_list_local_empty() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        let local_keys = keys::list_local().unwrap();
        assert!(local_keys.is_empty());
    }

    #[tokio::test]
    async fn test_list_remote_keys() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{
                    "keyId": "key-001",
                    "name": "mykey",
                    "algorithm": "ed25519",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;

        let result = run(KeyAction::List { local: false }, Some(&server.uri())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_remote_empty() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;

        let result = run(KeyAction::List { local: false }, Some(&server.uri())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_no_local_keys() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        let server = MockServer::start().await;

        let result = run(
            KeyAction::Register {
                name: None,
                key_id: None,
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_register_single_key() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        generate("reg-key").unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keyId": "key-reg-001",
                "name": "reg-key",
                "algorithm": "ed25519",
                "createdAt": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = run(
            KeyAction::Register {
                name: None,
                key_id: None,
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_revoke_key() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex("/auth/keys/.+/revoke"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keyId": "key-001",
                "status": "revoked",
                "revokedAt": "2026-02-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = run(
            KeyAction::Revoke {
                key_id: "key-001",
                reason: "compromised",
                message: Some("Key was exposed"),
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rotate_key() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        // Generate a key to rotate from
        generate("old-key").unwrap();
        let local_keys = keys::list_local().unwrap();
        let old_key_id = local_keys[0].key_id.clone();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/keys/rotate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "newKeyId": "key-new",
                "oldKeyId": old_key_id,
                "rotatedAt": "2026-02-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = run(
            KeyAction::Rotate {
                old_key_id: &old_key_id,
                name: Some("rotated-key"),
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rotate_key_not_found() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".apkg").join("keys")).unwrap();
        let server = MockServer::start().await;

        let result = run(
            KeyAction::Rotate {
                old_key_id: "nonexistent-key",
                name: None,
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_err());
    }
}
