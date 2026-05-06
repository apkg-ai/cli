use std::collections::BTreeMap;
use std::env;
use std::path::Path;

use futures_util::stream::{self, StreamExt, TryStreamExt};
use indicatif::{ProgressBar, ProgressStyle};

use crate::api::client::ApiClient;
use crate::config::lockfile::{LockedPackage, Lockfile, LOCKFILE_VERSION};
use crate::config::{cache, lockfile, manifest};
use crate::error::AppError;
use crate::resolver;
use crate::setup;
use crate::util::{display, integrity, package::parse_package_spec, tarball};

pub struct InstallOptions<'a> {
    pub package: Option<&'a str>,
    pub registry: Option<&'a str>,
    pub setup_target: Option<setup::SetupTarget>,
    pub frozen_lockfile: bool,
}

pub async fn run(opts: InstallOptions<'_>) -> Result<(), AppError> {
    match opts.package {
        Some(pkg) => install_single(&opts, pkg).await,
        None => install_all(&opts).await,
    }
}

async fn install_all(opts: &InstallOptions<'_>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let m = manifest::load(&cwd)?;

    let deps = m.dependencies.unwrap_or_default();
    if deps.is_empty() {
        display::info("No dependencies to install.");
        return Ok(());
    }

    let existing_lockfile = lockfile::load(&cwd)?;

    if opts.frozen_lockfile && existing_lockfile.is_none() {
        return Err(AppError::LockfileNotFound);
    }

    // Nicer-than-guard UX: explain offline install needs a lockfile before
    // anything ApiClient-shaped can surface the same error further in.
    if crate::util::offline::is_offline() && existing_lockfile.is_none() {
        return Err(AppError::OfflineModeBlocked {
            operation: "resolve without a lockfile".into(),
        });
    }

    let client = ApiClient::new(opts.registry)?;
    let start = std::time::Instant::now();

    let pb = make_spinner();
    pb.set_message("Resolving dependencies...");

    let result = resolver::resolve(&client, &deps, existing_lockfile.as_ref(), &pb, &cwd).await?;

    // --frozen-lockfile: verify the lockfile would not change
    if opts.frozen_lockfile {
        if let Some(existing) = &existing_lockfile {
            let new_lf = build_lockfile(&result);
            if new_lf.packages != existing.packages {
                pb.finish_and_clear();
                return Err(AppError::LockfileStale(
                    "resolved packages differ from lockfile".to_string(),
                ));
            }
        }
    }

    // Install each resolved package
    let pkg_count = result.packages.len();
    download_resolved(&client, &result, &cwd, &pb).await?;

    // Write lockfile
    let lf = build_lockfile(&result);
    lockfile::save(&cwd, &lf)?;

    pb.finish_and_clear();

    // Run setup for all installed packages
    run_setup_for_result(&result, &cwd, opts.setup_target.as_ref())?;

    let elapsed = start.elapsed();
    display::success(&format!(
        "Installed {pkg_count} package{} in {:.1}s",
        if pkg_count == 1 { "" } else { "s" },
        elapsed.as_secs_f64()
    ));

    Ok(())
}

async fn install_single(opts: &InstallOptions<'_>, pkg: &str) -> Result<(), AppError> {
    let (name, version_spec) = parse_package_spec(pkg);
    crate::util::package::validate_package_name(&name)?;
    let cwd = env::current_dir()?;

    let client = ApiClient::new(opts.registry)?;

    let pb = make_spinner();

    let range = resolve_dist_tag_to_range(&client, &name, version_spec, &pb).await?;

    let mut deps_map = BTreeMap::new();
    deps_map.insert(name.clone(), range);

    let existing_lockfile = lockfile::load(&cwd)?;

    pb.set_message("Resolving dependencies...".to_string());
    let result =
        resolver::resolve(&client, &deps_map, existing_lockfile.as_ref(), &pb, &cwd).await?;

    // Download all resolved packages (direct + transitive)
    let start = std::time::Instant::now();
    let pkg_count = result.packages.len();
    download_resolved(&client, &result, &cwd, &pb).await?;

    // Merge into existing lockfile
    let mut lf = existing_lockfile.unwrap_or_else(|| Lockfile {
        lockfile_version: LOCKFILE_VERSION,
        requires: true,
        resolved: chrono::Utc::now().to_rfc3339(),
        packages: BTreeMap::new(),
    });
    merge_into_lockfile(&mut lf, &result);
    lockfile::save(&cwd, &lf)?;

    pb.finish_and_clear();

    // Display info for the direct package
    if let Some(direct) = result.packages.get(&name) {
        display::success(&format!("Installed {name}@{}", direct.version));
        let install_dir = cwd.join("apkg_packages").join(validated_dir_name(&name)?);
        display::label_value("Location", &install_dir.display().to_string());
        display::label_value("Integrity", &direct.integrity);
    }
    if pkg_count > 1 {
        display::info(&format!(
            "Also installed {} transitive dependenc{}",
            pkg_count - 1,
            if pkg_count == 2 { "y" } else { "ies" }
        ));
    }

    let elapsed = start.elapsed();
    display::success(&format!(
        "Installed {pkg_count} package{} in {:.1}s",
        if pkg_count == 1 { "" } else { "s" },
        elapsed.as_secs_f64()
    ));

    // Run setup for ALL resolved packages
    run_setup_for_result(&result, &cwd, opts.setup_target.as_ref())?;

    Ok(())
}

/// Download a package using the cache, or fetch from the registry.
/// Extracts the tarball into `install_dir`.
pub(crate) async fn download_or_cache(
    client: &ApiClient,
    name: &str,
    version: &str,
    expected_integrity: &str,
    install_dir: &Path,
    pb: &ProgressBar,
) -> Result<(), AppError> {
    download_or_cache_with_info(client, name, version, expected_integrity, install_dir, pb)
        .await
        .map(|_| ())
}

/// Like `download_or_cache` but returns `(data_len, computed_integrity)`.
async fn download_or_cache_with_info(
    client: &ApiClient,
    name: &str,
    version: &str,
    expected_integrity: &str,
    install_dir: &Path,
    pb: &ProgressBar,
) -> Result<(usize, String), AppError> {
    // Try cache
    if let Ok(Some(entry)) = cache::load(name, version) {
        if entry.integrity == expected_integrity {
            pb.set_message(format!("Extracting {name}@{version} (cached)..."));
            if install_dir.exists() {
                std::fs::remove_dir_all(install_dir)?;
            }
            tarball::extract_tarball(&entry.data, install_dir)?;
            return Ok((entry.data.len(), entry.integrity));
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

    // Store in cache (best-effort)
    let _ = cache::store(name, version, &data, &computed);

    // Extract
    pb.set_message(format!("Extracting {name}@{version}..."));
    if install_dir.exists() {
        std::fs::remove_dir_all(install_dir)?;
    }
    tarball::extract_tarball(&data, install_dir)?;

    Ok((data.len(), computed))
}

/// Merge resolution results into an existing lockfile, preserving entries
/// not part of the current resolution. Used by `install_single` and `add`.
pub(crate) fn merge_into_lockfile(existing: &mut Lockfile, result: &resolver::ResolutionResult) {
    for (name, pkg) in &result.packages {
        let key = lockfile::lock_key(name, &pkg.version);
        let entry = LockedPackage {
            version: pkg.version.clone(),
            resolved: pkg.tarball_url.clone(),
            integrity: pkg.integrity.clone(),
            dependencies: pkg.dependencies.clone(),
            peer_dependencies: pkg.peer_dependencies.clone(),
            package_type: pkg.package_type.clone(),
            optional: false,
        };
        existing.packages.insert(key, entry);
    }
    existing.resolved = chrono::Utc::now().to_rfc3339();
}

/// Returns `true` if a version spec looks like a dist-tag (e.g. "latest")
/// rather than a semver version or range.
pub(crate) fn is_dist_tag(spec: &str) -> bool {
    let s = spec.trim();
    !s.is_empty()
        && !s.contains('.')
        && !s.starts_with('^')
        && !s.starts_with('~')
        && !s.starts_with('>')
        && !s.starts_with('<')
        && !s.starts_with('=')
        && !s.starts_with('*')
}

pub(crate) fn build_lockfile(result: &resolver::ResolutionResult) -> Lockfile {
    let packages = result
        .packages
        .iter()
        .map(|(name, pkg)| {
            let key = lockfile::lock_key(name, &pkg.version);
            let entry = LockedPackage {
                version: pkg.version.clone(),
                resolved: pkg.tarball_url.clone(),
                integrity: pkg.integrity.clone(),
                dependencies: pkg.dependencies.clone(),
                peer_dependencies: pkg.peer_dependencies.clone(),
                package_type: pkg.package_type.clone(),
                optional: false,
            };
            (key, entry)
        })
        .collect();
    Lockfile {
        lockfile_version: LOCKFILE_VERSION,
        requires: true,
        resolved: chrono::Utc::now().to_rfc3339(),
        packages,
    }
}

/// Validate a package name for use as a directory segment under
/// `apkg_packages/`. Returns the name unchanged on success; errors if the
/// name contains anything that could escape the intended install root
/// (e.g. `..`, absolute paths, backslashes).
///
/// Thin wrapper around `util::package::validate_package_name` — the name
/// lives here for discoverability alongside other install-path helpers.
pub(crate) fn validated_dir_name(name: &str) -> Result<&str, AppError> {
    crate::util::package::validate_package_name(name)
}

/// Parsed from `APKG_MAX_CONCURRENT_DOWNLOADS`; default 4. Zero is normalized
/// to 1 to avoid a stalled stream.
fn max_concurrent_downloads() -> usize {
    std::env::var("APKG_MAX_CONCURRENT_DOWNLOADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.max(1))
        .unwrap_or(4)
}

pub(crate) fn make_spinner() -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

/// Pre-resolve a version spec to something the resolver can consume.
/// Dist-tags (e.g. "latest", "beta") require a `GET /packages/{name}` to look
/// up the tag-to-version mapping; exact versions and ranges pass through.
/// Returns `"*"` when `spec` is `None`.
pub(crate) async fn resolve_dist_tag_to_range(
    client: &ApiClient,
    name: &str,
    spec: Option<&str>,
    pb: &ProgressBar,
) -> Result<String, AppError> {
    match spec {
        Some(s) if is_dist_tag(s) => {
            pb.set_message(format!("Resolving {name}@{s}..."));
            let metadata = client.get_package(name).await?;
            let version = metadata
                .dist_tags
                .get(s)
                .ok_or_else(|| AppError::PackageNotFound(format!("{name}@{s} — tag not found")))?;
            Ok(format!("={version}"))
        }
        Some(s) => Ok(s.to_string()),
        None => Ok("*".to_string()),
    }
}

/// Download (or fetch-from-cache) every package in the resolution result and
/// extract it into `<cwd>/apkg_packages/<name>`. Used by the install/add/update
/// commands as a shared "apply resolution to disk" step.
pub(crate) async fn download_resolved(
    client: &ApiClient,
    result: &resolver::ResolutionResult,
    cwd: &Path,
    pb: &ProgressBar,
) -> Result<(), AppError> {
    let concurrency = max_concurrent_downloads();
    let items: Vec<(&String, &resolver::ResolvedPackage)> = result.packages.iter().collect();

    stream::iter(items)
        .map(|(name, pkg)| async move {
            let install_dir = cwd.join("apkg_packages").join(validated_dir_name(name)?);
            download_or_cache(client, name, &pkg.version, &pkg.integrity, &install_dir, pb).await
        })
        .buffer_unordered(concurrency)
        .try_collect::<Vec<()>>()
        .await?;

    Ok(())
}

/// Like `download_resolved`, but restricted to the named subset. `update.rs`
/// uses this to re-install only the packages whose versions actually changed.
/// Names not present in `result.packages` are silently skipped.
pub(crate) async fn download_resolved_subset(
    client: &ApiClient,
    result: &resolver::ResolutionResult,
    names: &[&str],
    cwd: &Path,
    pb: &ProgressBar,
) -> Result<(), AppError> {
    let concurrency = max_concurrent_downloads();
    let items: Vec<(&str, &resolver::ResolvedPackage)> = names
        .iter()
        .filter_map(|n| result.packages.get(*n).map(|p| (*n, p)))
        .collect();

    stream::iter(items)
        .map(|(name, pkg)| async move {
            let install_dir = cwd.join("apkg_packages").join(validated_dir_name(name)?);
            download_or_cache(client, name, &pkg.version, &pkg.integrity, &install_dir, pb).await
        })
        .buffer_unordered(concurrency)
        .try_collect::<Vec<()>>()
        .await?;

    Ok(())
}

/// Run tool-specific setup (claude / cursor / codex) for every package in the
/// resolution result. No-op when `target` is `None`. Errors if any package
/// name is unsafe to use as a directory segment.
pub(crate) fn run_setup_for_result(
    result: &resolver::ResolutionResult,
    cwd: &Path,
    target: Option<&setup::SetupTarget>,
) -> Result<(), AppError> {
    let Some(target) = target else { return Ok(()) };
    for name in result.packages.keys() {
        let install_dir = cwd.join("apkg_packages").join(validated_dir_name(name)?);
        let report = setup::run_setup(&setup::SetupContext {
            project_root: cwd.to_path_buf(),
            install_dir,
            target: target.clone(),
        });
        setup::display_report(&report);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use std::collections::BTreeMap;

    use super::*;
    use crate::config::lockfile::{LockedPackage, LOCKFILE_VERSION};
    use crate::resolver::{ResolutionResult, ResolvedPackage};
    use crate::test_utils::env_lock;

    /// Restores CWD to the crate root on drop. Must be declared **after** the
    /// tempdir in a test so it drops *before* the tempdir is deleted —
    /// otherwise CWD is left pointing at a path that no longer exists, which
    /// poisons ENV_LOCK for the next test that calls `env::current_dir()`.
    struct CwdGuard;
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
        }
    }

    fn make_resolved(name: &str, version: &str) -> (String, ResolvedPackage) {
        (
            name.to_string(),
            ResolvedPackage {
                version: version.to_string(),
                tarball_url: format!("https://example.com/{name}/{version}/tarball"),
                integrity: format!("sha256-{name}-{version}"),
                package_type: "skill".to_string(),
                dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
            },
        )
    }

    fn make_empty_lockfile() -> Lockfile {
        Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: String::new(),
            packages: BTreeMap::new(),
        }
    }

    fn make_lockfile_with(entries: &[(&str, &str)]) -> Lockfile {
        let mut lf = make_empty_lockfile();
        for &(name, version) in entries {
            let key = lockfile::lock_key(name, version);
            lf.packages.insert(
                key,
                LockedPackage {
                    version: version.to_string(),
                    resolved: String::new(),
                    integrity: String::new(),
                    dependencies: BTreeMap::new(),
                    peer_dependencies: BTreeMap::new(),
                    package_type: "skill".to_string(),
                    optional: false,
                },
            );
        }
        lf
    }

    #[test]
    fn test_merge_into_lockfile_empty() {
        let mut lf = make_empty_lockfile();
        let result = ResolutionResult {
            packages: BTreeMap::from([make_resolved("foo", "1.0.0")]),
        };
        merge_into_lockfile(&mut lf, &result);
        assert_eq!(lf.packages.len(), 1);
        assert!(lf.packages.contains_key("foo@1.0.0"));
    }

    #[test]
    fn test_merge_into_lockfile_preserves_existing() {
        let mut lf = make_lockfile_with(&[("bar", "2.0.0")]);
        let result = ResolutionResult {
            packages: BTreeMap::from([make_resolved("foo", "1.0.0")]),
        };
        merge_into_lockfile(&mut lf, &result);
        assert_eq!(lf.packages.len(), 2);
        assert!(lf.packages.contains_key("foo@1.0.0"));
        assert!(lf.packages.contains_key("bar@2.0.0"));
    }

    #[test]
    fn test_merge_into_lockfile_upserts() {
        let mut lf = make_lockfile_with(&[("foo", "1.0.0")]);
        let result = ResolutionResult {
            packages: BTreeMap::from([make_resolved("foo", "1.1.0")]),
        };
        merge_into_lockfile(&mut lf, &result);
        // Old entry remains (different key), new entry added
        assert!(lf.packages.contains_key("foo@1.1.0"));
        assert_eq!(
            lf.packages.get("foo@1.1.0").unwrap().integrity,
            "sha256-foo-1.1.0"
        );
    }

    #[test]
    fn test_is_dist_tag_latest() {
        assert!(is_dist_tag("latest"));
    }

    #[test]
    fn test_is_dist_tag_beta() {
        assert!(is_dist_tag("beta"));
    }

    #[test]
    fn test_is_dist_tag_semver_version() {
        assert!(!is_dist_tag("1.0.0"));
    }

    #[test]
    fn test_is_dist_tag_caret() {
        assert!(!is_dist_tag("^1.0.0"));
    }

    #[test]
    fn test_is_dist_tag_tilde() {
        assert!(!is_dist_tag("~1.0.0"));
    }

    #[test]
    fn test_is_dist_tag_star() {
        assert!(!is_dist_tag("*"));
    }

    #[test]
    fn test_is_dist_tag_empty() {
        assert!(!is_dist_tag(""));
    }

    #[test]
    fn test_is_dist_tag_gte() {
        assert!(!is_dist_tag(">=1.0.0"));
    }

    // --- build_lockfile ---

    #[test]
    fn test_build_lockfile_empty() {
        let result = ResolutionResult {
            packages: BTreeMap::new(),
        };
        let lf = build_lockfile(&result);
        assert_eq!(lf.lockfile_version, LOCKFILE_VERSION);
        assert!(lf.requires);
        assert!(lf.packages.is_empty());
    }

    #[test]
    fn test_build_lockfile_single_package() {
        let result = ResolutionResult {
            packages: BTreeMap::from([make_resolved("foo", "1.0.0")]),
        };
        let lf = build_lockfile(&result);
        assert_eq!(lf.packages.len(), 1);
        let entry = lf.packages.get("foo@1.0.0").unwrap();
        assert_eq!(entry.version, "1.0.0");
        assert_eq!(entry.integrity, "sha256-foo-1.0.0");
        assert_eq!(entry.package_type, "skill");
        assert!(!entry.optional);
    }

    #[test]
    fn test_build_lockfile_multiple_packages() {
        let result = ResolutionResult {
            packages: BTreeMap::from([
                make_resolved("foo", "1.0.0"),
                make_resolved("bar", "2.0.0"),
            ]),
        };
        let lf = build_lockfile(&result);
        assert_eq!(lf.packages.len(), 2);
        assert!(lf.packages.contains_key("foo@1.0.0"));
        assert!(lf.packages.contains_key("bar@2.0.0"));
    }

    #[test]
    fn test_build_lockfile_scoped_package() {
        let result = ResolutionResult {
            packages: BTreeMap::from([make_resolved("@acme/tool", "3.0.0")]),
        };
        let lf = build_lockfile(&result);
        assert_eq!(lf.packages.len(), 1);
        assert!(lf.packages.contains_key("@acme/tool@3.0.0"));
    }

    // --- validated_dir_name ---

    #[test]
    fn test_validated_dir_name_simple() {
        assert_eq!(validated_dir_name("foo").unwrap(), "foo");
    }

    #[test]
    fn test_validated_dir_name_scoped() {
        assert_eq!(validated_dir_name("@org/pkg").unwrap(), "@org/pkg");
    }

    #[test]
    fn test_validated_dir_name_rejects_path_traversal() {
        assert!(validated_dir_name("../evil").is_err());
        assert!(validated_dir_name("@evil/../foo").is_err());
        assert!(validated_dir_name("/etc/passwd").is_err());
    }

    // --- make_spinner ---

    #[test]
    fn test_make_spinner_creates() {
        let pb = make_spinner();
        pb.finish_and_clear();
    }

    // --- async tests for download_or_cache and run ---

    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
        std::env::set_var("APKG_CACHE_DIR", tmp.join(".cache").to_str().unwrap());
    }

    fn write_project_manifest(dir: &std::path::Path, deps: &[(&str, &str)]) {
        let mut dep_map = serde_json::Map::new();
        for &(name, range) in deps {
            dep_map.insert(
                name.to_string(),
                serde_json::Value::String(range.to_string()),
            );
        }
        let manifest = serde_json::json!({
            "name": "@test/project",
            "version": "1.0.0",
            "type": "project",
            "description": "Test project",
            "license": "MIT",
            "platform": ["claude"],
            "dependencies": dep_map,
        });
        std::fs::write(
            dir.join("apkg.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    /// Create a minimal valid tarball for testing.
    fn make_test_tarball() -> Vec<u8> {
        let buf = Vec::new();
        let enc = zstd::Encoder::new(buf, 1).unwrap();
        let mut archive = tar::Builder::new(enc);
        let content = b"test content";
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "index.ts", &content[..])
            .unwrap();
        let enc = archive.into_inner().unwrap();
        enc.finish().unwrap()
    }

    #[tokio::test]
    async fn test_install_all_empty_deps() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_project_manifest(tmp.path(), &[]);
        let _cwd = CwdGuard;
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run(InstallOptions {
            package: None,
            registry: Some("http://localhost:1"),
            setup_target: None,
            frozen_lockfile: false,
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_install_all_frozen_no_lockfile() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        let _cwd = CwdGuard;
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let result = run(InstallOptions {
            package: None,
            registry: Some(&server.uri()),
            setup_target: None,
            frozen_lockfile: true,
        })
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("lockfile"));
    }

    #[tokio::test]
    async fn test_download_or_cache_from_network() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/1\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let client = ApiClient::new(Some(&server.uri())).unwrap();
        let install_dir = tmp.path().join("install_target");
        let pb = make_spinner();

        let result = download_or_cache(
            &client,
            "testpkg",
            "1.0.0",
            &expected_integrity,
            &install_dir,
            &pb,
        )
        .await;
        pb.finish_and_clear();
        assert!(result.is_ok());
        // Verify extraction happened
        assert!(install_dir.join("index.ts").exists());
    }

    #[tokio::test]
    async fn test_download_or_cache_integrity_mismatch() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let tarball = make_test_tarball();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/.+/1\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball))
            .mount(&server)
            .await;

        let client = ApiClient::new(Some(&server.uri())).unwrap();
        let install_dir = tmp.path().join("install_target");
        let pb = make_spinner();

        let result = download_or_cache(
            &client,
            "testpkg",
            "1.0.0",
            "sha256-WRONG",
            &install_dir,
            &pb,
        )
        .await;
        pb.finish_and_clear();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("integrity"));
    }

    #[test]
    fn test_max_concurrent_downloads_env_override() {
        let _lock = env_lock();
        std::env::set_var("APKG_MAX_CONCURRENT_DOWNLOADS", "8");
        assert_eq!(max_concurrent_downloads(), 8);
        std::env::set_var("APKG_MAX_CONCURRENT_DOWNLOADS", "0");
        assert_eq!(max_concurrent_downloads(), 1);
        std::env::remove_var("APKG_MAX_CONCURRENT_DOWNLOADS");
        assert_eq!(max_concurrent_downloads(), 4);
    }

    /// Prove the install loop runs downloads concurrently: 4 mock endpoints
    /// each delay 200ms. Serial would take ≥800ms; concurrent should be close
    /// to a single RTT. We allow generous slack for CI variance.
    #[tokio::test]
    async fn test_download_resolved_is_concurrent() {
        use std::time::Duration;
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        std::env::remove_var("APKG_MAX_CONCURRENT_DOWNLOADS");

        let server = MockServer::start().await;
        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);

        for i in 0..4 {
            Mock::given(method("GET"))
                .and(wiremock::matchers::path(format!(
                    "/packages/pkg{i}/1.0.0/tarball"
                )))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_millis(200))
                        .set_body_bytes(tarball.clone()),
                )
                .mount(&server)
                .await;
        }

        let cwd = tmp.path().join("cwd");
        std::fs::create_dir_all(&cwd).unwrap();
        let _guard = CwdGuard;
        std::env::set_current_dir(&cwd).unwrap();

        let mut packages = BTreeMap::new();
        for i in 0..4 {
            packages.insert(
                format!("pkg{i}"),
                ResolvedPackage {
                    version: "1.0.0".into(),
                    tarball_url: format!("{}/packages/pkg{i}/1.0.0/tarball", server.uri()),
                    integrity: expected_integrity.clone(),
                    package_type: "skill".into(),
                    dependencies: BTreeMap::new(),
                    peer_dependencies: BTreeMap::new(),
                },
            );
        }
        let result = ResolutionResult { packages };

        let client = ApiClient::new(Some(&server.uri())).unwrap();
        let pb = make_spinner();
        let start = std::time::Instant::now();
        download_resolved(&client, &result, &cwd, &pb)
            .await
            .unwrap();
        let elapsed = start.elapsed();
        pb.finish_and_clear();

        assert!(
            elapsed < Duration::from_millis(600),
            "expected concurrent downloads (<600ms), got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_download_or_cache_uses_cache() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());

        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);

        // Pre-populate cache
        cache::store("cached-pkg", "2.0.0", &tarball, &expected_integrity).unwrap();

        // No mock server needed — should use cache
        let client = ApiClient::new(Some("http://localhost:1")).unwrap();
        let install_dir = tmp.path().join("install_target");
        let pb = make_spinner();

        let result = download_or_cache(
            &client,
            "cached-pkg",
            "2.0.0",
            &expected_integrity,
            &install_dir,
            &pb,
        )
        .await;
        pb.finish_and_clear();
        assert!(result.is_ok());
        assert!(install_dir.join("index.ts").exists());
    }

    /// Helper: write an `apkg-lock.json` that maps `name@version` entries,
    /// so resolver skips network lookups and uses the lockfile-seeded path.
    fn write_lockfile(dir: &std::path::Path, entries: &[(&str, &str, &str)]) {
        let mut packages = BTreeMap::new();
        for &(name, version, integrity) in entries {
            let key = lockfile::lock_key(name, version);
            packages.insert(
                key,
                LockedPackage {
                    version: version.to_string(),
                    resolved: format!("https://example.test/{name}/{version}/tarball"),
                    integrity: integrity.to_string(),
                    dependencies: BTreeMap::new(),
                    peer_dependencies: BTreeMap::new(),
                    package_type: "skill".to_string(),
                    optional: false,
                },
            );
        }
        let lf = Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: String::new(),
            packages,
        };
        lockfile::save(dir, &lf).unwrap();
    }

    #[tokio::test]
    async fn test_install_all_happy_path_via_lockfile() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        // Resolver sees dep "foo@^1.0.0" + lockfile entry foo@1.0.0 → uses lockfile.
        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);

        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        write_lockfile(tmp.path(), &[("foo", "1.0.0", &expected_integrity)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/foo/1\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let result = run(InstallOptions {
            package: None,
            registry: Some(&server.uri()),
            setup_target: None,
            frozen_lockfile: false,
        })
        .await;

        assert!(result.is_ok(), "{:?}", result.err());
        assert!(tmp.path().join("apkg_packages/foo/index.ts").exists());
        // Lockfile still present + still mentions foo.
        let lf = lockfile::load(tmp.path()).unwrap().unwrap();
        assert!(lf.packages.contains_key("foo@1.0.0"));
    }

    #[tokio::test]
    async fn test_install_all_offline_succeeds_with_lockfile_and_cache() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        // Prime the tarball cache so nothing needs the network.
        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);
        crate::config::cache::store("foo", "1.0.0", &tarball, &expected_integrity).unwrap();

        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        write_lockfile(tmp.path(), &[("foo", "1.0.0", &expected_integrity)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        std::env::set_var("APKG_OFFLINE", "1");
        // No MockServer mounted — offline install must not hit the network.
        let result = run(InstallOptions {
            package: None,
            registry: None,
            setup_target: None,
            frozen_lockfile: false,
        })
        .await;
        std::env::remove_var("APKG_OFFLINE");

        assert!(result.is_ok(), "offline install failed: {:?}", result.err());
        assert!(tmp.path().join("apkg_packages/foo/index.ts").exists());
    }

    #[tokio::test]
    async fn test_install_all_offline_fails_without_lockfile() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        // Manifest but no lockfile.
        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        std::env::set_current_dir(tmp.path()).unwrap();

        std::env::set_var("APKG_OFFLINE", "1");
        let result = run(InstallOptions {
            package: None,
            registry: None,
            setup_target: None,
            frozen_lockfile: false,
        })
        .await;
        std::env::remove_var("APKG_OFFLINE");

        let err = result.unwrap_err();
        assert!(
            matches!(err, AppError::OfflineModeBlocked { .. }),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_install_all_frozen_lockfile_matches() {
        // `--frozen-lockfile` should pass (not error) when resolution exactly
        // matches the existing lockfile.
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);

        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        write_lockfile(tmp.path(), &[("foo", "1.0.0", &expected_integrity)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/foo/1\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let result = run(InstallOptions {
            package: None,
            registry: Some(&server.uri()),
            setup_target: None,
            frozen_lockfile: true,
        })
        .await;

        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[tokio::test]
    async fn test_install_single_happy_path_via_lockfile() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        let tarball = make_test_tarball();
        let expected_integrity = crate::util::integrity::sha256_integrity(&tarball);

        // install_single does NOT need a project manifest, but does need the
        // lockfile to short-circuit resolver's network lookup.
        write_lockfile(tmp.path(), &[("bar", "1.0.0", &expected_integrity)]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/bar/1\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let result = run(InstallOptions {
            package: Some("bar@^1.0.0"),
            registry: Some(&server.uri()),
            setup_target: None,
            frozen_lockfile: false,
        })
        .await;

        assert!(result.is_ok(), "{:?}", result.err());
        assert!(tmp.path().join("apkg_packages/bar/index.ts").exists());
    }

    /// Attack-path test: a package name with `..` traversal must be refused
    /// before any filesystem write. Proves SEC-1 is closed.
    #[tokio::test]
    async fn test_install_rejects_malicious_package_name() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;
        std::env::set_current_dir(tmp.path()).unwrap();

        // No network mock needed — validation fires before any HTTP call.
        let result = run(InstallOptions {
            package: Some("../../etc/passwd"),
            registry: Some("http://unused.test"),
            setup_target: None,
            frozen_lockfile: false,
        })
        .await;

        let err = result.expect_err("malicious name must be rejected");
        assert!(
            err.to_string().contains("Invalid package name"),
            "expected validator error, got: {err}"
        );
        // No files were written outside the tempdir.
        assert!(!tmp.path().join("apkg_packages").exists());
    }
}
