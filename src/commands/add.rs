use std::collections::BTreeMap;
use std::env;

use crate::api::client::ApiClient;
use crate::commands::install;
use crate::config::lockfile::{Lockfile, LOCKFILE_VERSION};
use crate::config::{lockfile, manifest};
use crate::error::AppError;
use crate::resolver;
use crate::setup;
use crate::util::package::DepCategory;
use crate::util::{display, package::parse_package_spec};

pub struct AddOptions<'a> {
    pub package: &'a str,
    pub registry: Option<&'a str>,
    pub category: DepCategory,
    pub setup_target: Option<setup::SetupTarget>,
}

/// Build the caret range written into the manifest for a resolved version.
fn manifest_range(version: &str) -> String {
    format!("^{version}")
}

/// Insert a dependency into the correct manifest category and return the range.
fn update_manifest_deps(
    m: &mut manifest::Manifest,
    category: DepCategory,
    name: &str,
    version: &str,
) -> String {
    let range = manifest_range(version);
    let deps = match category {
        DepCategory::Dependencies => m.dependencies.get_or_insert_with(BTreeMap::new),
        DepCategory::DevDependencies => m.dev_dependencies.get_or_insert_with(BTreeMap::new),
        DepCategory::PeerDependencies => m.peer_dependencies.get_or_insert_with(BTreeMap::new),
    };
    deps.insert(name.to_string(), range.clone());
    range
}

/// Format the "Also installed N transitive dependency/ies" message.
/// Returns `None` when `pkg_count <= 1` (no transitive deps).
fn format_transitive_message(pkg_count: usize) -> Option<String> {
    if pkg_count <= 1 {
        return None;
    }
    let n = pkg_count - 1;
    Some(format!(
        "Also installed {n} transitive dependenc{}",
        if n == 1 { "y" } else { "ies" }
    ))
}

pub async fn run(opts: AddOptions<'_>) -> Result<(), AppError> {
    let (name, version_spec) = parse_package_spec(opts.package);
    let cwd = env::current_dir()?;

    // Manifest must exist for `add`
    let mut m = manifest::load(&cwd)?;

    let client = ApiClient::new(opts.registry)?;
    let pb = install::make_spinner();

    // Pre-resolve dist-tags to a version range the resolver can handle
    let range = match version_spec {
        Some(spec) if install::is_dist_tag(spec) => {
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
    for (pkg_name, pkg) in &result.packages {
        let pkg_install_dir = cwd.join("apkg_packages").join(install::safe_dir_name(pkg_name));
        install::download_or_cache(
            &client,
            pkg_name,
            &pkg.version,
            &pkg.integrity,
            &pkg_install_dir,
            &pb,
        )
        .await?;
    }

    pb.finish_and_clear();

    // Update manifest — only for the direct dependency
    let direct_pkg = result.packages.get(&name).ok_or_else(|| {
        AppError::Other(format!("Resolver did not resolve {name}"))
    })?;
    let manifest_range = update_manifest_deps(&mut m, opts.category, &name, &direct_pkg.version);
    manifest::save(&cwd, &m)?;

    // Merge into existing lockfile
    let mut lf = existing_lockfile.unwrap_or_else(|| Lockfile {
        lockfile_version: LOCKFILE_VERSION,
        requires: true,
        resolved: chrono::Utc::now().to_rfc3339(),
        packages: BTreeMap::new(),
    });
    install::merge_into_lockfile(&mut lf, &result);
    lockfile::save(&cwd, &lf)?;

    // Display info
    display::success(&format!("Added {name}@{}", direct_pkg.version));
    display::label_value("Range", &manifest_range);
    display::label_value("Saved to", opts.category.label());
    let direct_install_dir = cwd.join("apkg_packages").join(install::safe_dir_name(&name));
    display::label_value("Location", &direct_install_dir.display().to_string());
    display::label_value("Integrity", &direct_pkg.integrity);

    if let Some(msg) = format_transitive_message(result.packages.len()) {
        display::info(&msg);
    }

    // Run setup for ALL resolved packages
    if let Some(target) = opts.setup_target {
        for pkg_name in result.packages.keys() {
            let pkg_install_dir =
                cwd.join("apkg_packages").join(install::safe_dir_name(pkg_name));
            let report = setup::run_setup(&setup::SetupContext {
                project_root: cwd.clone(),
                install_dir: pkg_install_dir,
                target: target.clone(),
            });
            setup::display_report(&report);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{Manifest, PackageType};

    fn make_manifest() -> Manifest {
        Manifest {
            name: "@test/project".to_string(),
            version: "0.1.0".to_string(),
            package_type: PackageType::Project,
            description: String::new(),
            license: "MIT".to_string(),
            readme: None,
            keywords: None,
            authors: None,
            repository: None,
            homepage: None,
            platform: vec!["claude-code".to_string()],
            dependencies: None,
            dev_dependencies: None,
            peer_dependencies: None,
            scripts: None,
            hook_permissions: None,
        }
    }

    // --- manifest_range ---

    #[test]
    fn test_manifest_range() {
        assert_eq!(manifest_range("1.0.0"), "^1.0.0");
        assert_eq!(manifest_range("0.2.3"), "^0.2.3");
    }

    // --- update_manifest_deps ---

    #[test]
    fn test_update_manifest_deps_creates_dependencies() {
        let mut m = make_manifest();
        assert!(m.dependencies.is_none());

        let range = update_manifest_deps(&mut m, DepCategory::Dependencies, "foo", "1.0.0");

        assert_eq!(range, "^1.0.0");
        let deps = m.dependencies.unwrap();
        assert_eq!(deps.get("foo").unwrap(), "^1.0.0");
    }

    #[test]
    fn test_update_manifest_deps_creates_dev_dependencies() {
        let mut m = make_manifest();
        assert!(m.dev_dependencies.is_none());

        update_manifest_deps(&mut m, DepCategory::DevDependencies, "bar", "2.0.0");

        let deps = m.dev_dependencies.unwrap();
        assert_eq!(deps.get("bar").unwrap(), "^2.0.0");
    }

    #[test]
    fn test_update_manifest_deps_creates_peer_dependencies() {
        let mut m = make_manifest();
        assert!(m.peer_dependencies.is_none());

        update_manifest_deps(&mut m, DepCategory::PeerDependencies, "baz", "3.0.0");

        let deps = m.peer_dependencies.unwrap();
        assert_eq!(deps.get("baz").unwrap(), "^3.0.0");
    }

    #[test]
    fn test_update_manifest_deps_appends_to_existing() {
        let mut m = make_manifest();
        let mut existing = BTreeMap::new();
        existing.insert("old-pkg".to_string(), "^0.1.0".to_string());
        m.dependencies = Some(existing);

        update_manifest_deps(&mut m, DepCategory::Dependencies, "new-pkg", "1.0.0");

        let deps = m.dependencies.unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps.get("old-pkg").unwrap(), "^0.1.0");
        assert_eq!(deps.get("new-pkg").unwrap(), "^1.0.0");
    }

    #[test]
    fn test_update_manifest_deps_overwrites_existing_version() {
        let mut m = make_manifest();
        let mut existing = BTreeMap::new();
        existing.insert("foo".to_string(), "^1.0.0".to_string());
        m.dependencies = Some(existing);

        update_manifest_deps(&mut m, DepCategory::Dependencies, "foo", "2.0.0");

        let deps = m.dependencies.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps.get("foo").unwrap(), "^2.0.0");
    }

    // --- format_transitive_message ---

    #[test]
    fn test_format_transitive_none_for_zero() {
        assert!(format_transitive_message(0).is_none());
    }

    #[test]
    fn test_format_transitive_none_for_single() {
        assert!(format_transitive_message(1).is_none());
    }

    #[test]
    fn test_format_transitive_singular() {
        let msg = format_transitive_message(2).unwrap();
        assert_eq!(msg, "Also installed 1 transitive dependency");
    }

    #[test]
    fn test_format_transitive_plural() {
        let msg = format_transitive_message(4).unwrap();
        assert_eq!(msg, "Also installed 3 transitive dependencies");
    }
}
