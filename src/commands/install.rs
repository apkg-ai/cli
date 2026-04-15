use std::collections::BTreeMap;
use std::env;
use std::path::Path;

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
    for (name, pkg) in &result.packages {
        let install_dir = cwd.join("apkg_packages").join(safe_dir_name(name));
        download_or_cache(
            &client,
            name,
            &pkg.version,
            &pkg.integrity,
            &install_dir,
            &pb,
        )
        .await?;
    }

    // Write lockfile
    let lf = build_lockfile(&result);
    lockfile::save(&cwd, &lf)?;

    pb.finish_and_clear();

    // Run setup for all installed packages
    if let Some(ref target) = opts.setup_target {
        for name in result.packages.keys() {
            let install_dir = cwd.join("apkg_packages").join(safe_dir_name(name));
            let report = setup::run_setup(&setup::SetupContext {
                project_root: cwd.clone(),
                install_dir,
                target: target.clone(),
            });
            setup::display_report(&report);
        }
    }

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
    let cwd = env::current_dir()?;

    let client = ApiClient::new(opts.registry)?;

    let pb = make_spinner();

    // Pre-resolve dist-tags to a version range the resolver can handle
    let range = match version_spec {
        Some(spec) if is_dist_tag(spec) => {
            pb.set_message(format!("Resolving {name}@{spec}..."));
            let metadata = client.get_package(&name).await?;
            let version = metadata.dist_tags.get(spec).ok_or_else(|| {
                AppError::PackageNotFound(format!("{}@{spec} — tag not found", name))
            })?;
            format!("={version}")
        }
        Some(spec) => spec.to_string(),
        None => "*".to_string(),
    };

    let mut deps_map = BTreeMap::new();
    deps_map.insert(name.clone(), range);

    let existing_lockfile = lockfile::load(&cwd)?;

    pb.set_message("Resolving dependencies...".to_string());
    let result =
        resolver::resolve(&client, &deps_map, existing_lockfile.as_ref(), &pb, &cwd).await?;

    // Download all resolved packages (direct + transitive)
    let start = std::time::Instant::now();
    let pkg_count = result.packages.len();
    for (pkg_name, resolved) in &result.packages {
        let install_dir = cwd.join("apkg_packages").join(safe_dir_name(pkg_name));
        download_or_cache(
            &client,
            pkg_name,
            &resolved.version,
            &resolved.integrity,
            &install_dir,
            &pb,
        )
        .await?;
    }

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
        let install_dir = cwd.join("apkg_packages").join(safe_dir_name(&name));
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
    if let Some(ref target) = opts.setup_target {
        for pkg_name in result.packages.keys() {
            let install_dir = cwd.join("apkg_packages").join(safe_dir_name(pkg_name));
            let report = setup::run_setup(&setup::SetupContext {
                project_root: cwd.clone(),
                install_dir,
                target: target.clone(),
            });
            setup::display_report(&report);
        }
    }

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
pub(crate) fn merge_into_lockfile(
    existing: &mut Lockfile,
    result: &resolver::ResolutionResult,
) {
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

pub(crate) fn safe_dir_name(name: &str) -> String {
    name.to_string()
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::lockfile::{LockedPackage, LOCKFILE_VERSION};
    use crate::resolver::{ResolvedPackage, ResolutionResult};

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
}
