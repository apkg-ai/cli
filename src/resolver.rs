use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use indicatif::ProgressBar;
use semver::VersionReq;

use crate::api::client::ApiClient;
use crate::api::types::{PackageMetadata, VersionMetadata};
use crate::config::lockfile::{self, Lockfile};
use crate::config::{cache, manifest};
use crate::error::AppError;
use crate::util::{integrity, tarball};

pub struct ResolvedPackage {
    pub version: String,
    pub tarball_url: String,
    pub integrity: String,
    pub package_type: String,
    pub dependencies: BTreeMap<String, String>,
    pub peer_dependencies: BTreeMap<String, String>,
}

pub struct ResolutionResult {
    pub packages: BTreeMap<String, ResolvedPackage>,
}

/// Greedy BFS dependency resolver. Conflicts are hard errors (no backtracking).
///
/// `install_root` is the project root where `apkg_packages/` lives. When the
/// registry does not include dependency information in version metadata, the
/// resolver downloads and extracts each package so it can read the manifest
/// (`apkg.json`) to discover transitive dependencies.
pub async fn resolve(
    client: &ApiClient,
    dependencies: &BTreeMap<String, String>,
    existing_lockfile: Option<&Lockfile>,
    pb: &ProgressBar,
    install_root: &Path,
) -> Result<ResolutionResult, AppError> {
    let mut resolved: BTreeMap<String, ResolvedPackage> = BTreeMap::new();
    let mut queue: VecDeque<(String, String)> = dependencies
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    while let Some((name, range_str)) = queue.pop_front() {
        // Already resolved — check compatibility
        if let Some(existing) = resolved.get(&name) {
            let normalized = normalize_range(&range_str);
            let req = VersionReq::parse(&normalized).map_err(|e| {
                AppError::Other(format!(
                    "Invalid version range '{range_str}' for {name}: {e}"
                ))
            })?;
            let existing_ver = semver::Version::parse(&existing.version).map_err(|e| {
                AppError::Other(format!(
                    "Invalid resolved version '{}' for {name}: {e}",
                    existing.version
                ))
            })?;
            if req.matches(&existing_ver) {
                continue;
            }
            return Err(AppError::DependencyConflict(format!(
                "{name} — resolved {}, but {} also required",
                existing.version, range_str
            )));
        }

        let normalized = normalize_range(&range_str);
        let req = VersionReq::parse(&normalized).map_err(|e| {
            AppError::Other(format!(
                "Invalid version range '{range_str}' for {name}: {e}"
            ))
        })?;

        // Try lockfile seed
        if let Some(lf) = existing_lockfile {
            if let Some(locked) = lockfile::find_by_name(lf, &name) {
                if let Ok(locked_version) = semver::Version::parse(&locked.version) {
                    if req.matches(&locked_version) {
                        pb.set_message(format!("Using locked {name}@{}", locked.version));

                        // If the locked entry has deps, use them directly.
                        // Otherwise, check the installed manifest for deps.
                        let deps = if locked.dependencies.is_empty() {
                            read_installed_deps(install_root, &name)
                        } else {
                            locked.dependencies.clone()
                        };

                        let pkg = ResolvedPackage {
                            version: locked.version.clone(),
                            tarball_url: locked.resolved.clone(),
                            integrity: locked.integrity.clone(),
                            package_type: locked.package_type.clone(),
                            dependencies: deps.clone(),
                            peer_dependencies: locked.peer_dependencies.clone(),
                        };
                        for (dep_name, dep_version) in &deps {
                            queue.push_back((dep_name.clone(), dep_version.clone()));
                        }
                        resolved.insert(name, pkg);
                        continue;
                    }
                }
            }
        }

        // Fetch from registry
        pb.set_message(format!("Resolving {name}..."));
        let metadata = client.get_package(&name).await?;
        let version_meta = resolve_best_version(&metadata, &req)?;

        let dist = version_meta.dist.as_ref().ok_or_else(|| {
            AppError::Other(format!(
                "No dist info for {name}@{} — cannot determine tarball URL",
                version_meta.version
            ))
        })?;

        // Use registry deps if available; otherwise download the package
        // and read its manifest to discover dependencies.
        let mut deps = version_meta.dependencies.clone().unwrap_or_default();
        let computed_integrity;

        if deps.is_empty() {
            // Download and extract to discover dependencies from apkg.json
            let install_dir = install_root.join("apkg_packages").join(&name);
            computed_integrity = download_and_extract(
                client,
                &name,
                &version_meta.version,
                &dist.integrity,
                &install_dir,
                pb,
            )
            .await?;
            deps = read_installed_deps(install_root, &name);
        } else {
            computed_integrity = dist.integrity.clone();
        }

        let pkg = ResolvedPackage {
            version: version_meta.version.clone(),
            tarball_url: dist.tarball.clone(),
            integrity: computed_integrity,
            package_type: version_meta
                .package_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            dependencies: deps.clone(),
            peer_dependencies: BTreeMap::new(),
        };

        // Push transitive deps
        for (dep_name, dep_range) in &deps {
            queue.push_back((dep_name.clone(), dep_range.clone()));
        }

        resolved.insert(name, pkg);
    }

    Ok(ResolutionResult { packages: resolved })
}

/// Read dependencies from an already-installed package's apkg.json.
/// Uses lenient parsing (only extracts the `dependencies` field) so that
/// packages missing other required fields (e.g. `platform`) still work.
fn read_installed_deps(install_root: &Path, name: &str) -> BTreeMap<String, String> {
    let manifest_path = install_root
        .join("apkg_packages")
        .join(name)
        .join(manifest::MANIFEST_FILE);
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(_) => return BTreeMap::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return BTreeMap::new(),
    };
    parsed
        .get("dependencies")
        .and_then(|v| serde_json::from_value::<BTreeMap<String, String>>(v.clone()).ok())
        .unwrap_or_default()
}

/// Download a package tarball, verify integrity, cache it, and extract.
/// Returns the computed integrity hash.
async fn download_and_extract(
    client: &ApiClient,
    name: &str,
    version: &str,
    expected_integrity: &str,
    install_dir: &Path,
    pb: &ProgressBar,
) -> Result<String, AppError> {
    // Try cache first
    if let Ok(Some(entry)) = cache::load(name, version) {
        if entry.integrity == expected_integrity {
            pb.set_message(format!("Extracting {name}@{version} (cached)..."));
            if install_dir.exists() {
                std::fs::remove_dir_all(install_dir)?;
            }
            tarball::extract_tarball(&entry.data, install_dir)?;
            return Ok(entry.integrity);
        }
    }

    // Download
    pb.set_message(format!("Downloading {name}@{version}..."));
    let (data, _server_integrity) = client.download_tarball(name, version).await?;

    let computed = integrity::sha256_integrity(&data);
    if computed != expected_integrity {
        return Err(AppError::IntegrityMismatch {
            expected: expected_integrity.to_string(),
            actual: computed,
        });
    }

    // Cache (best-effort)
    let _ = cache::store(name, version, &data, &computed);

    // Extract
    pb.set_message(format!("Extracting {name}@{version}..."));
    if install_dir.exists() {
        std::fs::remove_dir_all(install_dir)?;
    }
    tarball::extract_tarball(&data, install_dir)?;

    Ok(computed)
}

/// Find the highest non-yanked version that satisfies the requirement.
fn resolve_best_version<'a>(
    metadata: &'a PackageMetadata,
    req: &VersionReq,
) -> Result<&'a VersionMetadata, AppError> {
    metadata
        .versions
        .values()
        .filter(|v| !v.yanked.unwrap_or(false))
        .filter(|v| {
            semver::Version::parse(&v.version)
                .map(|sv| req.matches(&sv))
                .unwrap_or(false)
        })
        .max_by(|a, b| {
            let va = semver::Version::parse(&a.version)
                .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
            let vb = semver::Version::parse(&b.version)
                .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
            va.cmp(&vb)
        })
        .ok_or_else(|| {
            AppError::PackageNotFound(format!(
                "No version matching {req} found for {}",
                metadata.name
            ))
        })
}

/// Normalize a version range string for the `semver` crate.
///
/// Bare versions like `"1.2.3"` are treated as exact (`"=1.2.3"`) because the
/// spec says bare version means exact. The `semver` crate would interpret
/// `"1.2.3"` as `"^1.2.3"`.
///
/// Caret ranges for 0.x versions are expanded so that `^0.1.0` means
/// `>=0.1.0, <1.0.0` (same semantics as `^1.0.0` → `>=1.0.0, <2.0.0`).
/// The Rust `semver` crate follows strict semver where `^0.1.0` = `>=0.1.0, <0.2.0`,
/// which is overly restrictive for a package manager.
fn normalize_range(range: &str) -> String {
    let trimmed = range.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return trimmed.to_string();
    }
    // Caret ranges for 0.x: expand to >=0.x.y, <1.0.0
    if let Some(version_part) = trimmed.strip_prefix('^') {
        if let Ok(v) = semver::Version::parse(version_part) {
            if v.major == 0 {
                return format!(">={version_part}, <1.0.0");
            }
        }
        return trimmed.to_string();
    }
    if trimmed.starts_with('~')
        || trimmed.starts_with('>')
        || trimmed.starts_with('<')
        || trimmed.starts_with('=')
    {
        return trimmed.to_string();
    }
    // Bare version — prepend = for exact match
    format!("={trimmed}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::api::types::{DistInfo, PackageMetadata, VersionMetadata};

    fn make_version(version: &str, yanked: bool) -> VersionMetadata {
        VersionMetadata {
            version: version.to_string(),
            package_type: Some("skill".to_string()),
            description: None,
            dist: Some(DistInfo {
                tarball: format!("https://example.com/{version}/tarball"),
                integrity: format!("sha256-{version}"),
                signatures: vec![],
            }),
            published_at: None,
            yanked: Some(yanked),
            dependencies: None,
            license: None,
            keywords: None,
            platform: None,
            deprecated: None,
        }
    }

    fn make_metadata(name: &str, versions: &[(&str, bool)]) -> PackageMetadata {
        let mut version_map = BTreeMap::new();
        for &(v, yanked) in versions {
            version_map.insert(v.to_string(), make_version(v, yanked));
        }
        PackageMetadata {
            name: name.to_string(),
            description: None,
            dist_tags: BTreeMap::new(),
            versions: version_map,
            maintainers: vec![],
            created_at: None,
            updated_at: None,
            readme: None,
            deprecated: None,
        }
    }

    #[test]
    fn test_resolve_best_version_picks_highest() {
        let meta = make_metadata(
            "pkg",
            &[("1.0.0", false), ("1.1.0", false), ("1.2.0", false)],
        );
        let req = VersionReq::parse("^1.0.0").unwrap();
        let best = resolve_best_version(&meta, &req).unwrap();
        assert_eq!(best.version, "1.2.0");
    }

    #[test]
    fn test_resolve_best_version_caret_boundary() {
        let meta = make_metadata("pkg", &[("1.9.9", false), ("2.0.0", false)]);
        let req = VersionReq::parse("^1.0.0").unwrap();
        let best = resolve_best_version(&meta, &req).unwrap();
        assert_eq!(best.version, "1.9.9");
    }

    #[test]
    fn test_resolve_best_version_tilde() {
        let meta = make_metadata("pkg", &[("1.2.5", false), ("1.3.0", false)]);
        let req = VersionReq::parse("~1.2.0").unwrap();
        let best = resolve_best_version(&meta, &req).unwrap();
        assert_eq!(best.version, "1.2.5");
    }

    #[test]
    fn test_resolve_best_version_exact() {
        let meta = make_metadata("pkg", &[("1.2.3", false), ("1.2.4", false)]);
        let req = VersionReq::parse("=1.2.3").unwrap();
        let best = resolve_best_version(&meta, &req).unwrap();
        assert_eq!(best.version, "1.2.3");
    }

    #[test]
    fn test_resolve_best_version_no_match() {
        let meta = make_metadata("pkg", &[("2.0.0", false)]);
        let req = VersionReq::parse("^1.0.0").unwrap();
        let result = resolve_best_version(&meta, &req);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_best_version_skips_yanked() {
        let meta = make_metadata("pkg", &[("1.0.0", false), ("1.1.0", true), ("1.2.0", true)]);
        let req = VersionReq::parse("^1.0.0").unwrap();
        let best = resolve_best_version(&meta, &req).unwrap();
        assert_eq!(best.version, "1.0.0");
    }

    #[test]
    fn test_normalize_range_bare_version() {
        assert_eq!(normalize_range("1.2.3"), "=1.2.3");
    }

    #[test]
    fn test_normalize_range_caret() {
        assert_eq!(normalize_range("^1.0.0"), "^1.0.0");
    }

    #[test]
    fn test_normalize_range_caret_zero_major() {
        assert_eq!(normalize_range("^0.1.0"), ">=0.1.0, <1.0.0");
    }

    #[test]
    fn test_normalize_range_caret_zero_zero() {
        assert_eq!(normalize_range("^0.0.3"), ">=0.0.3, <1.0.0");
    }

    #[test]
    fn test_normalize_range_tilde() {
        assert_eq!(normalize_range("~1.2.0"), "~1.2.0");
    }

    #[test]
    fn test_normalize_range_gte() {
        assert_eq!(normalize_range(">=1.0.0"), ">=1.0.0");
    }

    #[test]
    fn test_normalize_range_exact() {
        assert_eq!(normalize_range("=1.2.3"), "=1.2.3");
    }

    #[test]
    fn test_normalize_range_star() {
        assert_eq!(normalize_range("*"), "*");
    }

    #[test]
    fn test_normalize_range_empty() {
        assert_eq!(normalize_range(""), "");
    }

    #[test]
    fn test_normalize_range_whitespace() {
        assert_eq!(normalize_range("  ^1.0.0  "), "^1.0.0");
        assert_eq!(normalize_range("  1.2.3  "), "=1.2.3");
    }

    #[test]
    fn test_normalize_range_less_than() {
        assert_eq!(normalize_range("<2.0.0"), "<2.0.0");
    }

    #[test]
    fn test_resolve_best_version_all_yanked() {
        let meta = make_metadata("pkg", &[("1.0.0", true), ("1.1.0", true)]);
        let req = VersionReq::parse("^1.0.0").unwrap();
        let result = resolve_best_version(&meta, &req);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_installed_deps_with_deps() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("apkg_packages").join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("apkg.json"),
            r#"{"name":"mypkg","version":"1.0.0","dependencies":{"foo":"^1.0.0","bar":"^2.0.0"}}"#,
        )
        .unwrap();
        let deps = read_installed_deps(tmp.path(), "mypkg");
        assert_eq!(deps.len(), 2);
        assert_eq!(deps["foo"], "^1.0.0");
        assert_eq!(deps["bar"], "^2.0.0");
    }

    #[test]
    fn test_read_installed_deps_missing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let deps = read_installed_deps(tmp.path(), "nonexistent");
        assert!(deps.is_empty());
    }

    #[test]
    fn test_read_installed_deps_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("apkg_packages").join("badpkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("apkg.json"), "not valid json").unwrap();
        let deps = read_installed_deps(tmp.path(), "badpkg");
        assert!(deps.is_empty());
    }

    #[test]
    fn test_read_installed_deps_no_dependencies_field() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("apkg_packages").join("nodeps");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("apkg.json"),
            r#"{"name":"nodeps","version":"1.0.0"}"#,
        )
        .unwrap();
        let deps = read_installed_deps(tmp.path(), "nodeps");
        assert!(deps.is_empty());
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
    }

    #[tokio::test]
    async fn test_resolve_from_lockfile() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;

        let client = crate::api::client::ApiClient::new(Some(&server.uri())).unwrap();
        let mut deps = BTreeMap::new();
        deps.insert("lockedpkg".to_string(), "^1.0.0".to_string());

        let mut lock_packages = BTreeMap::new();
        lock_packages.insert(
            "lockedpkg@1.0.5".to_string(),
            lockfile::LockedPackage {
                version: "1.0.5".to_string(),
                resolved: "https://example.com/lockedpkg/1.0.5/tarball".to_string(),
                integrity: "sha256-locked".to_string(),
                package_type: "skill".to_string(),
                dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        let lockfile = Lockfile {
            lockfile_version: 1,
            requires: true,
            resolved: String::new(),
            packages: lock_packages,
        };

        let pb = ProgressBar::hidden();
        let result = resolve(&client, &deps, Some(&lockfile), &pb, tmp.path())
            .await
            .unwrap();
        assert_eq!(result.packages.len(), 1);
        assert_eq!(result.packages["lockedpkg"].version, "1.0.5");
    }

    #[tokio::test]
    async fn test_resolve_conflict_with_already_resolved() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;

        // pkgA is resolved from lockfile at 1.0.0, then the deps also
        // request pkgA@^2.0.0 which conflicts with the already-resolved 1.0.0
        let client = crate::api::client::ApiClient::new(Some(&server.uri())).unwrap();
        let mut deps = BTreeMap::new();
        deps.insert("pkgA".to_string(), "^1.0.0".to_string());
        // Second request for incompatible range:
        deps.insert("pkgA".to_string(), "^1.0.0".to_string());

        // Use lockfile to seed pkgA at 1.0.0 with a transitive dep that
        // requests pkgA@^2.0.0 (creating a conflict in the second iteration)
        let mut lock_packages = BTreeMap::new();
        lock_packages.insert(
            "pkgA@1.0.0".to_string(),
            lockfile::LockedPackage {
                version: "1.0.0".to_string(),
                resolved: "https://x.com/a/tarball".to_string(),
                integrity: "sha256-a".to_string(),
                package_type: "skill".to_string(),
                dependencies: {
                    let mut d = BTreeMap::new();
                    d.insert("pkgB".to_string(), "^1.0.0".to_string());
                    d
                },
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        lock_packages.insert(
            "pkgB@1.0.0".to_string(),
            lockfile::LockedPackage {
                version: "1.0.0".to_string(),
                resolved: "https://x.com/b/tarball".to_string(),
                integrity: "sha256-b".to_string(),
                package_type: "skill".to_string(),
                dependencies: {
                    let mut d = BTreeMap::new();
                    d.insert("pkgA".to_string(), "^2.0.0".to_string());
                    d
                },
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        let lockfile = Lockfile {
            lockfile_version: 1,
            requires: true,
            resolved: String::new(),
            packages: lock_packages,
        };
        let pb = ProgressBar::hidden();

        let result = resolve(&client, &deps, Some(&lockfile), &pb, tmp.path()).await;
        assert!(matches!(result, Err(AppError::DependencyConflict(_))));
    }

    #[tokio::test]
    async fn test_resolve_lockfile_with_transitive_deps() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;

        let client = crate::api::client::ApiClient::new(Some(&server.uri())).unwrap();
        let mut deps = BTreeMap::new();
        deps.insert("pkgA".to_string(), "^1.0.0".to_string());

        // pkgA depends on pkgB, both in lockfile
        let mut lock_packages = BTreeMap::new();
        lock_packages.insert(
            "pkgA@1.0.0".to_string(),
            lockfile::LockedPackage {
                version: "1.0.0".to_string(),
                resolved: "https://x.com/a/tarball".to_string(),
                integrity: "sha256-a".to_string(),
                package_type: "skill".to_string(),
                dependencies: {
                    let mut d = BTreeMap::new();
                    d.insert("pkgB".to_string(), "^1.0.0".to_string());
                    d
                },
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        lock_packages.insert(
            "pkgB@1.2.0".to_string(),
            lockfile::LockedPackage {
                version: "1.2.0".to_string(),
                resolved: "https://x.com/b/tarball".to_string(),
                integrity: "sha256-b".to_string(),
                package_type: "skill".to_string(),
                dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        let lockfile = Lockfile {
            lockfile_version: 1,
            requires: true,
            resolved: String::new(),
            packages: lock_packages,
        };
        let pb = ProgressBar::hidden();

        let result = resolve(&client, &deps, Some(&lockfile), &pb, tmp.path())
            .await
            .unwrap();
        assert_eq!(result.packages.len(), 2);
        assert_eq!(result.packages["pkgA"].version, "1.0.0");
        assert_eq!(result.packages["pkgB"].version, "1.2.0");
    }

    #[tokio::test]
    async fn test_resolve_package_not_found() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/nonexistent"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": { "code": "NOT_FOUND", "message": "Not found" }
            })))
            .mount(&server)
            .await;

        let client = crate::api::client::ApiClient::new(Some(&server.uri())).unwrap();
        let mut deps = BTreeMap::new();
        deps.insert("nonexistent".to_string(), "^1.0.0".to_string());
        let pb = ProgressBar::hidden();

        let result = resolve(&client, &deps, None, &pb, tmp.path()).await;
        assert!(matches!(result, Err(AppError::PackageNotFound(_))));
    }

    #[tokio::test]
    async fn test_resolve_compatible_duplicate_request() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;

        // pkgA (from lockfile at 1.0.0) is also requested as ^1.0.0 by pkgB
        // This should succeed (compatible)
        let client = crate::api::client::ApiClient::new(Some(&server.uri())).unwrap();
        let mut deps = BTreeMap::new();
        deps.insert("pkgA".to_string(), "^1.0.0".to_string());

        let mut lock_packages = BTreeMap::new();
        lock_packages.insert(
            "pkgA@1.5.0".to_string(),
            lockfile::LockedPackage {
                version: "1.5.0".to_string(),
                resolved: "https://x.com/a/tarball".to_string(),
                integrity: "sha256-a".to_string(),
                package_type: "skill".to_string(),
                dependencies: {
                    let mut d = BTreeMap::new();
                    d.insert("pkgB".to_string(), "^1.0.0".to_string());
                    d
                },
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        lock_packages.insert(
            "pkgB@1.0.0".to_string(),
            lockfile::LockedPackage {
                version: "1.0.0".to_string(),
                resolved: "https://x.com/b/tarball".to_string(),
                integrity: "sha256-b".to_string(),
                package_type: "skill".to_string(),
                dependencies: {
                    let mut d = BTreeMap::new();
                    // pkgB requests pkgA@^1.0.0 — compatible with already-resolved 1.5.0
                    d.insert("pkgA".to_string(), "^1.0.0".to_string());
                    d
                },
                peer_dependencies: BTreeMap::new(),
                optional: false,
            },
        );
        let lockfile = Lockfile {
            lockfile_version: 1,
            requires: true,
            resolved: String::new(),
            packages: lock_packages,
        };
        let pb = ProgressBar::hidden();

        let result = resolve(&client, &deps, Some(&lockfile), &pb, tmp.path())
            .await
            .unwrap();
        assert_eq!(result.packages.len(), 2);
        assert_eq!(result.packages["pkgA"].version, "1.5.0");
    }
}
