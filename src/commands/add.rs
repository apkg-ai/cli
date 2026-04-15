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
    let manifest_range = format!("^{}", direct_pkg.version);
    let deps = match opts.category {
        DepCategory::Dependencies => m.dependencies.get_or_insert_with(BTreeMap::new),
        DepCategory::DevDependencies => m.dev_dependencies.get_or_insert_with(BTreeMap::new),
        DepCategory::PeerDependencies => m.peer_dependencies.get_or_insert_with(BTreeMap::new),
    };
    deps.insert(name.clone(), manifest_range.clone());
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

    let pkg_count = result.packages.len();
    if pkg_count > 1 {
        display::info(&format!(
            "Also installed {} transitive dependenc{}",
            pkg_count - 1,
            if pkg_count == 2 { "y" } else { "ies" }
        ));
    }

    // Run setup for ALL resolved packages
    if let Some(target) = opts.setup_target {
        for (pkg_name, _pkg) in &result.packages {
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
