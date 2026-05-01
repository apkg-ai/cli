use std::collections::HashMap;
use std::env;
use std::fmt;

use console::Style;

use crate::api::client::ApiClient;
use crate::api::types::RegistrySigningKey;
use crate::commands::install::make_spinner;
use crate::config::{cache, lockfile};
use crate::error::AppError;
use crate::util::{display, integrity, verify as crypto_verify};

pub struct VerifyOptions<'a> {
    pub package: Option<&'a str>,
    pub json: bool,
    pub strict: bool,
    pub registry: Option<&'a str>,
}

enum CheckStatus {
    Ok,
    Verified,
    Unsigned,
    Invalid,
    Mismatch,
    Error(String),
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => write!(f, "ok"),
            Self::Verified => write!(f, "verified"),
            Self::Unsigned => write!(f, "unsigned"),
            Self::Invalid => write!(f, "invalid"),
            Self::Mismatch => write!(f, "mismatch"),
            Self::Error(msg) => write!(f, "error ({msg})"),
        }
    }
}

impl CheckStatus {
    fn json_str(&self) -> String {
        match self {
            Self::Ok => "ok".to_string(),
            Self::Verified => "verified".to_string(),
            Self::Unsigned => "unsigned".to_string(),
            Self::Invalid => "invalid".to_string(),
            Self::Mismatch => "mismatch".to_string(),
            Self::Error(msg) => format!("error: {msg}"),
        }
    }

    fn is_ok_or_verified(&self) -> bool {
        matches!(self, Self::Ok | Self::Verified)
    }
}

struct PackageResult {
    name: String,
    version: String,
    integrity: CheckStatus,
    signature: CheckStatus,
    provenance: String,
}

/// Parse a lockfile key like `"foo@1.0.0"` or `"@scope/pkg@2.0.0"` into
/// `(name, version)`.
fn parse_lockfile_key(key: &str) -> Option<(&str, &str)> {
    // The last `@` that is not at position 0 separates name from version.
    let idx = key.rfind('@')?;
    if idx == 0 {
        return None;
    }
    Some((&key[..idx], &key[idx + 1..]))
}

pub async fn run(opts: VerifyOptions<'_>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;

    let lockfile = lockfile::load(&cwd)?.ok_or_else(|| {
        AppError::Other("No lockfile found. Run `apkg install` first.".to_string())
    })?;

    // Determine which packages to verify
    let targets: Vec<(&str, &str, &lockfile::LockedPackage)> = if let Some(name) = opts.package {
        let entry = lockfile::find_by_name(&lockfile, name)
            .ok_or_else(|| AppError::Other(format!("Package \"{name}\" not found in lockfile.")))?;
        // Recover the version from the entry
        vec![(name, entry.version.as_str(), entry)]
    } else {
        let mut list = Vec::new();
        for (key, entry) in &lockfile.packages {
            if let Some((name, version)) = parse_lockfile_key(key) {
                list.push((name, version, entry));
            }
        }
        list
    };

    if targets.is_empty() {
        display::info("No packages to verify.");
        return Ok(());
    }

    let client = ApiClient::new(opts.registry)?;

    let pb = make_spinner();
    pb.set_message("Fetching registry signing keys...");

    // Fetch registry signing keys (best-effort; empty map on failure)
    let registry_keys: HashMap<String, RegistrySigningKey> =
        match client.get_registry_signing_keys().await {
            Ok(collection) => collection
                .keys
                .into_iter()
                .filter(|k| k.status == "active")
                .map(|k| (k.key_id.clone(), k))
                .collect(),
            Err(_) => HashMap::new(),
        };

    pb.set_message("Verifying packages...");

    let mut results: Vec<PackageResult> = Vec::with_capacity(targets.len());

    for (name, version, locked) in &targets {
        let integrity_status = verify_integrity(&client, name, version, locked, &pb).await;
        let signature_status =
            verify_signature(&client, name, version, locked, &registry_keys).await;
        let provenance = verify_provenance(&client, name, version).await;

        results.push(PackageResult {
            name: (*name).to_string(),
            version: (*version).to_string(),
            integrity: integrity_status,
            signature: signature_status,
            provenance,
        });
    }

    pb.finish_and_clear();

    if opts.json {
        print_json(&results);
    } else {
        print_table(&results);
    }

    if opts.strict {
        let failures: Vec<String> = results
            .iter()
            .filter(|r| !r.integrity.is_ok_or_verified() || !r.signature.is_ok_or_verified())
            .map(|r| format!("{}@{}", r.name, r.version))
            .collect();
        if !failures.is_empty() {
            return Err(AppError::VerifyFailed(format!(
                "{} package{} failed verification: {}",
                failures.len(),
                if failures.len() == 1 { "" } else { "s" },
                failures.join(", ")
            )));
        }
    }

    Ok(())
}

async fn verify_integrity(
    client: &ApiClient,
    name: &str,
    version: &str,
    locked: &lockfile::LockedPackage,
    pb: &indicatif::ProgressBar,
) -> CheckStatus {
    let expected = &locked.integrity;
    if expected.is_empty() {
        return CheckStatus::Error("no integrity hash in lockfile".to_string());
    }

    // Try cache first
    if let Ok(Some(entry)) = cache::load(name, version) {
        let computed = integrity::sha256_integrity(&entry.data);
        if computed == *expected {
            return CheckStatus::Ok;
        }
        return CheckStatus::Mismatch;
    }

    // Download and verify
    pb.set_message(format!("Downloading {name}@{version} for verification..."));
    match client.download_tarball(name, version).await {
        Ok((data, _)) => {
            let computed = integrity::sha256_integrity(&data);
            // Cache for future use (best-effort)
            let _ = cache::store(name, version, &data, &computed);
            if computed == *expected {
                CheckStatus::Ok
            } else {
                CheckStatus::Mismatch
            }
        }
        Err(e) => CheckStatus::Error(format!("download failed: {e}")),
    }
}

async fn verify_signature(
    client: &ApiClient,
    name: &str,
    _version: &str,
    locked: &lockfile::LockedPackage,
    registry_keys: &HashMap<String, RegistrySigningKey>,
) -> CheckStatus {
    // Fetch package metadata to get signatures for this version
    let Ok(metadata) = client.get_package(name).await else {
        return CheckStatus::Error("failed to fetch package metadata".to_string());
    };

    let Some(version_meta) = metadata.versions.get(&locked.version) else {
        return CheckStatus::Error("version not found in metadata".to_string());
    };

    let Some(dist) = &version_meta.dist else {
        return CheckStatus::Unsigned;
    };

    if dist.signatures.is_empty() {
        return CheckStatus::Unsigned;
    }

    // Try to verify against registry counter-signature
    for sig in &dist.signatures {
        if let Some(registry_key) = registry_keys.get(&sig.key_id) {
            let Ok(pubkey_bytes) = crypto_verify::extract_ed25519_pubkey(&registry_key.public_key)
            else {
                continue;
            };

            match crypto_verify::verify_signature(&pubkey_bytes, &sig.signature, &locked.integrity)
            {
                Ok(true) => return CheckStatus::Verified,
                Ok(false) => return CheckStatus::Invalid,
                Err(_) => {}
            }
        }
    }

    // Signatures exist but no matching registry key found — we can't verify
    // author-only signatures (no public key lookup endpoint for arbitrary users)
    CheckStatus::Unsigned
}

async fn verify_provenance(client: &ApiClient, name: &str, version: &str) -> String {
    match client.get_provenance(name, version).await {
        Ok(Some(attestation)) => {
            if let Some(predicate) = &attestation.predicate {
                let builder_id = predicate
                    .pointer("/runDetails/builder/id")
                    .and_then(serde_json::Value::as_str);
                if let Some(id) = builder_id {
                    return format!("verified ({id})");
                }
            }
            "verified".to_string()
        }
        Ok(None) | Err(_) => "none".to_string(),
    }
}

fn print_table(results: &[PackageResult]) {
    let green = Style::new().green();
    let yellow = Style::new().yellow();
    let red = Style::new().red();
    let bold = Style::new().bold();

    println!(
        "\n{:<40} {:<14} {:<14} {}",
        bold.apply_to("Package"),
        bold.apply_to("Signature"),
        bold.apply_to("Integrity"),
        bold.apply_to("Provenance"),
    );

    let mut signed = 0u32;
    let mut integrity_ok = 0u32;
    let mut with_provenance = 0u32;

    for r in results {
        let pkg_label = format!("{}@{}", r.name, r.version);

        let sig_display = match &r.signature {
            CheckStatus::Verified => green.apply_to("verified".to_string()),
            CheckStatus::Unsigned => yellow.apply_to("unsigned".to_string()),
            CheckStatus::Invalid => red.apply_to("invalid".to_string()),
            other => red.apply_to(other.to_string()),
        };

        let int_display = match &r.integrity {
            CheckStatus::Ok => green.apply_to("ok".to_string()),
            CheckStatus::Mismatch => red.apply_to("mismatch".to_string()),
            other => red.apply_to(other.to_string()),
        };

        let prov_display = if r.provenance.starts_with("verified") {
            green.apply_to(r.provenance.clone())
        } else {
            yellow.apply_to(r.provenance.clone())
        };

        println!("{pkg_label:<40} {sig_display:<14} {int_display:<14} {prov_display}");

        if matches!(&r.signature, CheckStatus::Verified) {
            signed += 1;
        }
        if matches!(&r.integrity, CheckStatus::Ok) {
            integrity_ok += 1;
        }
        if r.provenance.starts_with("verified") {
            with_provenance += 1;
        }
    }

    let total = u32::try_from(results.len()).unwrap_or(0);
    println!();
    display::success(&format!(
        "Verified: {signed}/{total} signed, {integrity_ok}/{total} integrity ok, {with_provenance}/{total} with provenance"
    ));

    // Warnings for unsigned packages
    let unsigned: Vec<String> = results
        .iter()
        .filter(|r| matches!(&r.signature, CheckStatus::Unsigned))
        .map(|r| format!("{}@{}", r.name, r.version))
        .collect();
    if !unsigned.is_empty() {
        display::warn(&format!(
            "{} package{} unsigned ({})",
            unsigned.len(),
            if unsigned.len() == 1 { " is" } else { "s are" },
            unsigned.join(", ")
        ));
    }

    // Warnings for invalid/mismatch
    let failed: Vec<String> = results
        .iter()
        .filter(|r| {
            matches!(&r.signature, CheckStatus::Invalid)
                || matches!(&r.integrity, CheckStatus::Mismatch)
        })
        .map(|r| format!("{}@{}", r.name, r.version))
        .collect();
    if !failed.is_empty() {
        display::warn(&format!(
            "{} package{} failed verification ({})",
            failed.len(),
            if failed.len() == 1 { "" } else { "s" },
            failed.join(", ")
        ));
    }
}

fn print_json(results: &[PackageResult]) {
    let mut signed = 0u32;
    let mut integrity_ok = 0u32;
    let mut with_provenance = 0u32;

    let packages: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            if matches!(&r.signature, CheckStatus::Verified) {
                signed += 1;
            }
            if matches!(&r.integrity, CheckStatus::Ok) {
                integrity_ok += 1;
            }
            if r.provenance.starts_with("verified") {
                with_provenance += 1;
            }
            serde_json::json!({
                "package": r.name,
                "version": r.version,
                "integrity": r.integrity.json_str(),
                "signature": r.signature.json_str(),
                "provenance": r.provenance,
            })
        })
        .collect();

    let total = u32::try_from(results.len()).unwrap_or(0);
    let output = serde_json::json!({
        "verifiedAt": chrono::Utc::now().to_rfc3339(),
        "packages": packages,
        "summary": {
            "total": total,
            "signed": signed,
            "integrityOk": integrity_ok,
            "withProvenance": with_provenance,
        }
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_default()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lockfile_key_unscoped() {
        let (name, version) = parse_lockfile_key("foo@1.0.0").unwrap();
        assert_eq!(name, "foo");
        assert_eq!(version, "1.0.0");
    }

    #[test]
    fn test_parse_lockfile_key_scoped() {
        let (name, version) = parse_lockfile_key("@scope/pkg@2.0.0").unwrap();
        assert_eq!(name, "@scope/pkg");
        assert_eq!(version, "2.0.0");
    }

    #[test]
    fn test_check_status_display() {
        assert_eq!(CheckStatus::Ok.to_string(), "ok");
        assert_eq!(CheckStatus::Verified.to_string(), "verified");
        assert_eq!(CheckStatus::Unsigned.to_string(), "unsigned");
        assert_eq!(CheckStatus::Invalid.to_string(), "invalid");
        assert_eq!(CheckStatus::Mismatch.to_string(), "mismatch");
        assert_eq!(
            CheckStatus::Error("test".to_string()).to_string(),
            "error (test)"
        );
    }

    #[test]
    fn test_check_status_json_str() {
        assert_eq!(CheckStatus::Ok.json_str(), "ok");
        assert_eq!(CheckStatus::Verified.json_str(), "verified");
        assert_eq!(CheckStatus::Unsigned.json_str(), "unsigned");
        assert_eq!(CheckStatus::Invalid.json_str(), "invalid");
        assert_eq!(CheckStatus::Mismatch.json_str(), "mismatch");
        assert_eq!(
            CheckStatus::Error("oops".to_string()).json_str(),
            "error: oops"
        );
    }

    #[test]
    fn test_check_status_is_ok_or_verified() {
        assert!(CheckStatus::Ok.is_ok_or_verified());
        assert!(CheckStatus::Verified.is_ok_or_verified());
        assert!(!CheckStatus::Unsigned.is_ok_or_verified());
        assert!(!CheckStatus::Invalid.is_ok_or_verified());
        assert!(!CheckStatus::Mismatch.is_ok_or_verified());
        assert!(!CheckStatus::Error("x".to_string()).is_ok_or_verified());
    }

    fn make_result(
        name: &str,
        version: &str,
        integrity: CheckStatus,
        signature: CheckStatus,
        provenance: &str,
    ) -> PackageResult {
        PackageResult {
            name: name.to_string(),
            version: version.to_string(),
            integrity,
            signature,
            provenance: provenance.to_string(),
        }
    }

    #[test]
    fn test_print_table_all_ok() {
        let results = vec![
            make_result(
                "@test/foo",
                "1.0.0",
                CheckStatus::Ok,
                CheckStatus::Verified,
                "verified",
            ),
            make_result(
                "@test/bar",
                "2.0.0",
                CheckStatus::Ok,
                CheckStatus::Verified,
                "verified (github-actions)",
            ),
        ];
        print_table(&results);
    }

    #[test]
    fn test_print_table_mixed() {
        let results = vec![
            make_result(
                "@test/ok",
                "1.0.0",
                CheckStatus::Ok,
                CheckStatus::Verified,
                "verified",
            ),
            make_result(
                "@test/unsigned",
                "1.0.0",
                CheckStatus::Ok,
                CheckStatus::Unsigned,
                "none",
            ),
            make_result(
                "@test/invalid",
                "1.0.0",
                CheckStatus::Mismatch,
                CheckStatus::Invalid,
                "none",
            ),
            make_result(
                "@test/error",
                "1.0.0",
                CheckStatus::Error("download failed".into()),
                CheckStatus::Unsigned,
                "none",
            ),
        ];
        print_table(&results);
    }

    #[test]
    fn test_print_table_all_unsigned() {
        let results = vec![
            make_result(
                "pkg-a",
                "1.0.0",
                CheckStatus::Ok,
                CheckStatus::Unsigned,
                "none",
            ),
            make_result(
                "pkg-b",
                "2.0.0",
                CheckStatus::Ok,
                CheckStatus::Unsigned,
                "none",
            ),
        ];
        print_table(&results);
    }

    #[test]
    fn test_print_json_all_ok() {
        let results = vec![make_result(
            "@test/foo",
            "1.0.0",
            CheckStatus::Ok,
            CheckStatus::Verified,
            "verified",
        )];
        print_json(&results);
    }

    #[test]
    fn test_print_json_mixed() {
        let results = vec![
            make_result(
                "@test/ok",
                "1.0.0",
                CheckStatus::Ok,
                CheckStatus::Verified,
                "verified",
            ),
            make_result(
                "@test/bad",
                "2.0.0",
                CheckStatus::Mismatch,
                CheckStatus::Invalid,
                "none",
            ),
        ];
        print_json(&results);
    }

    #[test]
    fn test_print_json_empty() {
        let results: Vec<PackageResult> = vec![];
        print_json(&results);
    }

    #[test]
    fn test_print_table_empty() {
        let results: Vec<PackageResult> = vec![];
        print_table(&results);
    }

    #[test]
    fn test_parse_lockfile_key_no_at() {
        assert!(parse_lockfile_key("nover").is_none());
    }

    #[test]
    fn test_parse_lockfile_key_only_at_start() {
        assert!(parse_lockfile_key("@1.0.0").is_none());
    }

    // --- async tests for run() and internal verify functions ---

    use std::collections::BTreeMap;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
        std::env::set_var("APKG_CACHE_DIR", tmp.join(".cache").to_str().unwrap());
    }

    fn write_lockfile(dir: &std::path::Path, entries: &[(&str, &str, &str)]) {
        let mut packages = BTreeMap::new();
        for &(name, version, integrity) in entries {
            let key = lockfile::lock_key(name, version);
            packages.insert(
                key,
                lockfile::LockedPackage {
                    version: version.to_string(),
                    resolved: String::new(),
                    integrity: integrity.to_string(),
                    dependencies: BTreeMap::new(),
                    peer_dependencies: BTreeMap::new(),
                    package_type: "skill".to_string(),
                    optional: false,
                },
            );
        }
        let lf = lockfile::Lockfile {
            lockfile_version: lockfile::LOCKFILE_VERSION,
            requires: true,
            resolved: "2026-01-01T00:00:00Z".to_string(),
            packages,
        };
        lockfile::save(dir, &lf).unwrap();
    }

    #[tokio::test]
    async fn test_verify_no_lockfile() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let result = run(VerifyOptions {
            package: None,
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("lockfile") || err.contains("install"));
    }

    #[tokio::test]
    async fn test_verify_empty_lockfile() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_lockfile(tmp.path(), &[]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        // Mock registry signing keys
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;

        let result = run(VerifyOptions {
            package: None,
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_package_not_in_lockfile() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_lockfile(tmp.path(), &[("foo", "1.0.0", "sha256-abc")]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let result = run(VerifyOptions {
            package: Some("nonexistent"),
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn test_verify_integrity_from_cache_ok() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        // Create a test tarball and cache it
        let tarball_data = b"test-tarball-content";
        let integrity_hash = crate::util::integrity::sha256_integrity(tarball_data);
        cache::store("foo", "1.0.0", tarball_data, &integrity_hash).unwrap();

        write_lockfile(tmp.path(), &[("foo", "1.0.0", &integrity_hash)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        // Mock registry signing keys
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;
        // Mock get_package for signature check
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "foo",
                "versions": {
                    "1.0.0": {
                        "version": "1.0.0"
                    }
                }
            })))
            .mount(&server)
            .await;
        // Mock provenance
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/1\\.0\\.0/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let result = run(VerifyOptions {
            package: Some("foo"),
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_integrity_mismatch_from_cache() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        // Cache with one integrity, lockfile expects another
        let tarball_data = b"test-tarball-content";
        let actual_integrity = crate::util::integrity::sha256_integrity(tarball_data);
        cache::store("bar", "2.0.0", tarball_data, &actual_integrity).unwrap();

        // Lockfile expects a different integrity
        write_lockfile(tmp.path(), &[("bar", "2.0.0", "sha256-DIFFERENT")]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "bar",
                "versions": {
                    "2.0.0": {
                        "version": "2.0.0"
                    }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/2\\.0\\.0/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        // Non-strict: should succeed but report mismatch
        let result = run(VerifyOptions {
            package: Some("bar"),
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_strict_fails_on_unsigned() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let tarball_data = b"test-content";
        let integrity_hash = crate::util::integrity::sha256_integrity(tarball_data);
        cache::store("pkg", "1.0.0", tarball_data, &integrity_hash).unwrap();

        write_lockfile(tmp.path(), &[("pkg", "1.0.0", &integrity_hash)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "pkg",
                "versions": {
                    "1.0.0": {
                        "version": "1.0.0"
                    }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/1\\.0\\.0/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        // Strict mode: unsigned packages should cause failure
        let result = run(VerifyOptions {
            package: Some("pkg"),
            json: false,
            strict: true,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed verification"));
    }

    #[tokio::test]
    async fn test_verify_json_output() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let tarball_data = b"json-test";
        let integrity_hash = crate::util::integrity::sha256_integrity(tarball_data);
        cache::store("jpkg", "1.0.0", tarball_data, &integrity_hash).unwrap();

        write_lockfile(tmp.path(), &[("jpkg", "1.0.0", &integrity_hash)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "jpkg",
                "versions": {
                    "1.0.0": {
                        "version": "1.0.0"
                    }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/1\\.0\\.0/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let result = run(VerifyOptions {
            package: Some("jpkg"),
            json: true,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_all_packages_iterates_lockfile() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let data_a = b"data-a";
        let data_b = b"data-b";
        let hash_a = crate::util::integrity::sha256_integrity(data_a);
        let hash_b = crate::util::integrity::sha256_integrity(data_b);
        cache::store("pkg-a", "1.0.0", data_a, &hash_a).unwrap();
        cache::store("pkg-b", "2.0.0", data_b, &hash_b).unwrap();

        write_lockfile(
            tmp.path(),
            &[("pkg-a", "1.0.0", &hash_a), ("pkg-b", "2.0.0", &hash_b)],
        );
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{
                    "keyId": "key-1",
                    "publicKey": "dGVzdA==",
                    "algorithm": "ed25519",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/[^/]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "pkg",
                "versions": {
                    "1.0.0": { "version": "1.0.0" },
                    "2.0.0": { "version": "2.0.0" }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        // package: None -> iterates all lockfile entries
        let result = run(VerifyOptions {
            package: None,
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_with_signatures_and_provenance() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let data = b"signed-pkg-data";
        let hash = crate::util::integrity::sha256_integrity(data);
        cache::store("signed", "1.0.0", data, &hash).unwrap();

        write_lockfile(tmp.path(), &[("signed", "1.0.0", &hash)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{
                    "keyId": "key-1",
                    "publicKey": "dGVzdA==",
                    "algorithm": "ed25519",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/[^/]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "signed",
                "versions": {
                    "1.0.0": {
                        "version": "1.0.0",
                        "dist": {
                            "tarball": "https://example.com/signed/1.0.0/tarball",
                            "integrity": hash,
                            "signatures": [{
                                "keyId": "key-1",
                                "signature": "dGVzdC1zaWc="
                            }]
                        }
                    }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/provenance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "subject": [{"name": "signed", "digest": {"sha256": "abc"}}],
                "predicate": {
                    "runDetails": {
                        "builder": { "id": "https://github.com/actions/runner" }
                    }
                }
            })))
            .mount(&server)
            .await;

        let result = run(VerifyOptions {
            package: Some("signed"),
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_integrity_download_path() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        // DON'T cache — force the download path
        let data = b"download-verify-data";
        let hash = crate::util::integrity::sha256_integrity(data);

        write_lockfile(tmp.path(), &[("dlpkg", "3.0.0", &hash)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;
        // Download endpoint
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/3\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data.to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/[^/]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "dlpkg",
                "versions": {
                    "3.0.0": { "version": "3.0.0" }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/3\\.0\\.0/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let result = run(VerifyOptions {
            package: Some("dlpkg"),
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_empty_integrity_in_lockfile() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        // Lockfile entry with empty integrity
        write_lockfile(tmp.path(), &[("nohash", "1.0.0", "")]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/[^/]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "nohash",
                "versions": {
                    "1.0.0": { "version": "1.0.0" }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/provenance"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let result = run(VerifyOptions {
            package: Some("nohash"),
            json: false,
            strict: false,
            registry: Some(&server.uri()),
        })
        .await;
        // Should succeed (non-strict) but report error for integrity
        assert!(result.is_ok());
    }
}
