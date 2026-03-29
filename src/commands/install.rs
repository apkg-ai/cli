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

    let result = resolver::resolve(&client, &deps, existing_lockfile.as_ref(), &pb).await?;

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
        for (name, _pkg) in &result.packages {
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
    pb.set_message(format!("Resolving {name}..."));

    let metadata = client.get_package(&name).await?;
    let version = resolve_version(&metadata, version_spec)?;

    // Get dist info for cache and lockfile
    let version_meta = metadata.versions.get(&version);
    let (tarball_url, expected_integrity, pkg_type) = extract_dist_info(version_meta);

    let install_dir = cwd.join("apkg_packages").join(safe_dir_name(&name));

    let (data_len, computed) = if expected_integrity.is_empty() {
        // No dist info — download directly
        pb.set_message(format!("Downloading {name}@{version}..."));
        let (data, server_integrity) = client.download_tarball(&name, &version).await?;
        let computed = integrity::sha256_integrity(&data);
        if let Some(expected) = &server_integrity {
            if expected != &computed {
                pb.finish_and_clear();
                return Err(AppError::IntegrityMismatch {
                    expected: expected.clone(),
                    actual: computed,
                });
            }
        }
        pb.set_message(format!("Extracting {name}@{version}..."));
        if install_dir.exists() {
            std::fs::remove_dir_all(&install_dir)?;
        }
        tarball::extract_tarball(&data, &install_dir)?;
        let _ = cache::store(&name, &version, &data, &computed);
        (data.len(), computed)
    } else {
        // Try cache-aware download
        download_or_cache_with_info(
            &client,
            &name,
            &version,
            &expected_integrity,
            &install_dir,
            &pb,
        )
        .await?
    };

    // Update lockfile
    let mut lf = lockfile::load(&cwd)?.unwrap_or_else(|| Lockfile {
        lockfile_version: LOCKFILE_VERSION,
        requires: true,
        resolved: chrono::Utc::now().to_rfc3339(),
        packages: BTreeMap::new(),
    });
    let key = lockfile::lock_key(&name, &version);
    let version_deps = version_meta
        .and_then(|vm| vm.dependencies.clone())
        .unwrap_or_default();
    lf.packages.insert(
        key,
        LockedPackage {
            version: version.clone(),
            resolved: tarball_url,
            integrity: computed.clone(),
            dependencies: version_deps,
            peer_dependencies: BTreeMap::new(),
            package_type: pkg_type,
            optional: false,
        },
    );
    lf.resolved = chrono::Utc::now().to_rfc3339();
    lockfile::save(&cwd, &lf)?;

    pb.finish_and_clear();

    display::success(&format!("Installed {name}@{version}"));
    display::label_value("Location", &install_dir.display().to_string());
    display::label_value("Integrity", &computed);
    display::label_value("Size", &display::format_size(data_len));

    if let Some(target) = opts.setup_target.clone() {
        let report = setup::run_setup(&setup::SetupContext {
            project_root: cwd,
            install_dir,
            target,
        });
        setup::display_report(&report);
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

fn resolve_version(
    metadata: &crate::api::types::PackageMetadata,
    version_spec: Option<&str>,
) -> Result<String, AppError> {
    match version_spec {
        Some(v) => {
            if let Some(resolved) = metadata.dist_tags.get(v) {
                return Ok(resolved.clone());
            }
            if metadata.versions.contains_key(v) {
                return Ok(v.to_string());
            }
            Err(AppError::PackageNotFound(format!(
                "{}@{v} — version not found",
                metadata.name
            )))
        }
        None => metadata
            .dist_tags
            .get("latest")
            .cloned()
            .or_else(|| {
                metadata
                    .versions
                    .keys()
                    .filter_map(|v| semver::Version::parse(v).ok().map(|sv| (v.clone(), sv)))
                    .max_by(|a, b| a.1.cmp(&b.1))
                    .map(|(v, _)| v)
            })
            .ok_or_else(|| {
                AppError::PackageNotFound(format!("{} — no versions published", metadata.name))
            }),
    }
}

fn extract_dist_info(
    version_meta: Option<&crate::api::types::VersionMetadata>,
) -> (String, String, String) {
    if let Some(vm) = version_meta {
        let dist = vm.dist.as_ref();
        (
            dist.map_or_else(String::new, |d| d.tarball.clone()),
            dist.map_or_else(String::new, |d| d.integrity.clone()),
            vm.package_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        )
    } else {
        (String::new(), String::new(), "unknown".to_string())
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
