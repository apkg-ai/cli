use std::collections::BTreeMap;
use std::env;

use indicatif::{ProgressBar, ProgressStyle};

use crate::api::client::ApiClient;
use crate::api::types::PackageMetadata;
use crate::config::lockfile::{LockedPackage, Lockfile, LOCKFILE_VERSION};
use crate::config::{cache, lockfile, manifest};
use crate::error::AppError;
use crate::setup;
use crate::util::package::DepCategory;
use crate::util::{display, integrity, package::parse_package_spec, tarball};

pub struct AddOptions<'a> {
    pub package: &'a str,
    pub registry: Option<&'a str>,
    pub category: DepCategory,
    pub setup_target: Option<setup::SetupTarget>,
}

#[allow(clippy::too_many_lines)]
pub async fn run(opts: AddOptions<'_>) -> Result<(), AppError> {
    let (name, version_spec) = parse_package_spec(opts.package);
    let cwd = env::current_dir()?;

    // Manifest must exist for `add`
    let mut m = manifest::load(&cwd)?;

    let client = ApiClient::new(opts.registry)?;

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb.set_message(format!("Resolving {name}..."));

    let metadata = client.get_package(&name).await?;
    let version = resolve_version(&metadata, version_spec)?;

    // Extract dist info for cache and lockfile before downloading
    let version_meta = metadata.versions.get(&version);
    let (tarball_url, expected_integrity, pkg_type) = extract_dist_info(version_meta);

    let install_dir = cwd.join("apkg_packages").join(&name);

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
    } else if let Ok(Some(entry)) = cache::load(&name, &version) {
        if entry.integrity == expected_integrity {
            pb.set_message(format!("Extracting {name}@{version} (cached)..."));
            if install_dir.exists() {
                std::fs::remove_dir_all(&install_dir)?;
            }
            tarball::extract_tarball(&entry.data, &install_dir)?;
            (entry.data.len(), entry.integrity)
        } else {
            download_and_extract(
                &client,
                &name,
                &version,
                &expected_integrity,
                &install_dir,
                &pb,
            )
            .await?
        }
    } else {
        download_and_extract(
            &client,
            &name,
            &version,
            &expected_integrity,
            &install_dir,
            &pb,
        )
        .await?
    };

    pb.finish_and_clear();

    // Update manifest
    let range = format!("^{version}");
    let deps = match opts.category {
        DepCategory::Dependencies => m.dependencies.get_or_insert_with(BTreeMap::new),
        DepCategory::DevDependencies => m.dev_dependencies.get_or_insert_with(BTreeMap::new),
        DepCategory::PeerDependencies => m.peer_dependencies.get_or_insert_with(BTreeMap::new),
    };
    deps.insert(name.clone(), range.clone());
    manifest::save(&cwd, &m)?;

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

    display::success(&format!("Added {name}@{version}"));
    display::label_value("Range", &range);
    display::label_value("Saved to", opts.category.label());
    display::label_value("Location", &install_dir.display().to_string());
    display::label_value("Integrity", &computed);
    display::label_value("Size", &display::format_size(data_len));

    if let Some(target) = opts.setup_target {
        let report = setup::run_setup(&setup::SetupContext {
            project_root: cwd,
            install_dir,
            target,
        });
        setup::display_report(&report);
    }

    Ok(())
}

async fn download_and_extract(
    client: &ApiClient,
    name: &str,
    version: &str,
    expected_integrity: &str,
    install_dir: &std::path::Path,
    pb: &ProgressBar,
) -> Result<(usize, String), AppError> {
    pb.set_message(format!("Downloading {name}@{version}..."));
    let (data, _server_integrity) = client.download_tarball(name, version).await?;
    let computed = integrity::sha256_integrity(&data);
    if computed != expected_integrity {
        return Err(AppError::IntegrityMismatch {
            expected: expected_integrity.to_string(),
            actual: computed,
        });
    }
    let _ = cache::store(name, version, &data, &computed);
    pb.set_message(format!("Extracting {name}@{version}..."));
    if install_dir.exists() {
        std::fs::remove_dir_all(install_dir)?;
    }
    tarball::extract_tarball(&data, install_dir)?;
    Ok((data.len(), computed))
}

fn resolve_version(
    metadata: &PackageMetadata,
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

