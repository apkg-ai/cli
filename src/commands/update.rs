use std::collections::BTreeMap;
use std::env;

use console::Style;

use crate::api::client::ApiClient;
use crate::commands::install;
use crate::config::lockfile::Lockfile;
use crate::config::{lockfile, manifest};
use crate::error::AppError;
use crate::resolver;
use crate::setup;
use crate::util::display;

pub struct UpdateOptions<'a> {
    pub package: Option<&'a str>,
    pub registry: Option<&'a str>,
    pub latest: bool,
    pub dry_run: bool,
    pub setup_target: Option<setup::SetupTarget>,
}

struct Change {
    name: String,
    current: String,
    updated: String,
    old_range: String,
    new_range: String,
}

#[allow(clippy::too_many_lines)]
pub async fn run(opts: UpdateOptions<'_>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let m = manifest::load(&cwd)?;

    let deps = m.dependencies.clone().unwrap_or_default();
    if deps.is_empty() {
        display::info("No dependencies to update.");
        return Ok(());
    }

    // Determine which packages to update
    let targets: Vec<String> = if let Some(name) = opts.package {
        if !deps.contains_key(name) {
            return Err(AppError::Other(format!(
                "Package \"{name}\" is not in dependencies"
            )));
        }
        vec![name.to_string()]
    } else {
        deps.keys().cloned().collect()
    };

    // Load existing lockfile to determine current versions
    let existing_lockfile = lockfile::load(&cwd)?;

    // Build filtered lockfile — remove targeted packages so resolver fetches fresh
    let filtered_lockfile = existing_lockfile
        .as_ref()
        .map(|lf| filter_lockfile(lf, &targets));

    // Build resolution deps — for --latest, replace targeted ranges with "*"
    let resolution_deps: BTreeMap<String, String> = deps
        .iter()
        .map(|(name, range)| {
            if opts.latest && targets.contains(name) {
                (name.clone(), "*".to_string())
            } else {
                (name.clone(), range.clone())
            }
        })
        .collect();

    let client = ApiClient::new(opts.registry)?;
    let pb = install::make_spinner();
    pb.set_message("Resolving dependencies...");

    let result =
        resolver::resolve(&client, &resolution_deps, filtered_lockfile.as_ref(), &pb).await?;

    pb.finish_and_clear();

    // Compute changes — compare resolved versions against current lockfile
    let changes: Vec<Change> = targets
        .iter()
        .filter_map(|name| {
            let resolved = result.packages.get(name.as_str())?;
            let current_version = existing_lockfile
                .as_ref()
                .and_then(|lf| lockfile::find_by_name(lf, name))
                .map(|entry| entry.version.clone())
                .unwrap_or_default();

            if resolved.version == current_version {
                return None;
            }

            let old_range = deps.get(name).cloned().unwrap_or_default();
            let new_range = format!("^{}", resolved.version);

            Some(Change {
                name: name.clone(),
                current: if current_version.is_empty() {
                    "—".to_string()
                } else {
                    current_version
                },
                updated: resolved.version.clone(),
                old_range,
                new_range,
            })
        })
        .collect();

    if changes.is_empty() {
        display::success("All packages are up to date.");
    } else {
        // Display table
        print_changes_table(&changes, opts.latest);

        if opts.dry_run {
            display::info("No changes written (--dry-run).");
            return Ok(());
        }

        // Download changed packages
        let dl_pb = install::make_spinner();
        for change in &changes {
            if let Some(pkg) = result.packages.get(&change.name) {
                let install_dir = cwd
                    .join("apkg_packages")
                    .join(install::safe_dir_name(&change.name));
                install::download_or_cache(
                    &client,
                    &change.name,
                    &pkg.version,
                    &pkg.integrity,
                    &install_dir,
                    &dl_pb,
                )
                .await?;
            }
        }
        dl_pb.finish_and_clear();

        // Save lockfile
        let lf = install::build_lockfile(&result);
        lockfile::save(&cwd, &lf)?;

        // If --latest, update manifest ranges
        if opts.latest {
            let mut updated_manifest = m;
            if let Some(ref mut manifest_deps) = updated_manifest.dependencies {
                for change in &changes {
                    if manifest_deps.contains_key(&change.name) {
                        manifest_deps.insert(change.name.clone(), change.new_range.clone());
                    }
                }
            }
            manifest::save(&cwd, &updated_manifest)?;
        }

        let pkg_word = if changes.len() == 1 {
            "package"
        } else {
            "packages"
        };
        display::success(&format!("Updated {} {pkg_word}.", changes.len()));
    }

    // Always run setup for all resolved packages to ensure tool configs are in sync
    if let Some(ref target) = opts.setup_target {
        for (name, _pkg) in &result.packages {
            let install_dir = cwd
                .join("apkg_packages")
                .join(install::safe_dir_name(name));
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

fn filter_lockfile(lockfile: &Lockfile, packages_to_update: &[String]) -> Lockfile {
    let mut filtered = lockfile.clone();
    filtered.packages.retain(|key, _| {
        let name = if let Some(idx) = key.rfind('@') {
            if idx > 0 {
                &key[..idx]
            } else {
                key
            }
        } else {
            key
        };
        !packages_to_update.iter().any(|target| target == name)
    });
    filtered
}

#[allow(clippy::cast_possible_truncation)]
fn print_changes_table(changes: &[Change], show_ranges: bool) {
    let bold = Style::new().bold();
    let green = Style::new().green();

    if show_ranges {
        // Calculate column widths
        let name_w = changes
            .iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(7)
            .max(7);
        let cur_w = changes
            .iter()
            .map(|c| c.current.len())
            .max()
            .unwrap_or(7)
            .max(7);
        let upd_w = changes
            .iter()
            .map(|c| c.updated.len())
            .max()
            .unwrap_or(7)
            .max(7);

        eprintln!(
            "  {:<name_w$}  {:<cur_w$}  {:<upd_w$}  {}",
            bold.apply_to("Package"),
            bold.apply_to("Current"),
            bold.apply_to("Updated"),
            bold.apply_to("Range Updated"),
        );
        for change in changes {
            let range_update = format!("{} -> {}", change.old_range, change.new_range);
            eprintln!(
                "  {:<name_w$}  {:<cur_w$}  {:<upd_w$}  {}",
                change.name,
                change.current,
                green.apply_to(&change.updated),
                range_update,
            );
        }
    } else {
        let name_w = changes
            .iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(7)
            .max(7);
        let cur_w = changes
            .iter()
            .map(|c| c.current.len())
            .max()
            .unwrap_or(7)
            .max(7);

        eprintln!(
            "  {:<name_w$}  {:<cur_w$}  {}",
            bold.apply_to("Package"),
            bold.apply_to("Current"),
            bold.apply_to("Updated"),
        );
        for change in changes {
            eprintln!(
                "  {:<name_w$}  {:<cur_w$}  {}",
                change.name,
                change.current,
                green.apply_to(&change.updated),
            );
        }
    }
    eprintln!();
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::lockfile::{LockedPackage, Lockfile, LOCKFILE_VERSION};

    fn make_lockfile(entries: &[(&str, &str)]) -> Lockfile {
        let mut packages = BTreeMap::new();
        for &(name, version) in entries {
            let key = format!("{name}@{version}");
            packages.insert(
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
        Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: String::new(),
            packages,
        }
    }

    #[test]
    fn test_filter_lockfile_removes_targeted() {
        let lf = make_lockfile(&[("foo", "1.0.0"), ("bar", "2.0.0")]);
        let filtered = filter_lockfile(&lf, &["foo".to_string()]);
        assert!(!filtered.packages.contains_key("foo@1.0.0"));
        assert!(filtered.packages.contains_key("bar@2.0.0"));
    }

    #[test]
    fn test_filter_lockfile_handles_scoped() {
        let lf = make_lockfile(&[("@scope/pkg", "1.0.0"), ("bar", "2.0.0")]);
        let filtered = filter_lockfile(&lf, &["@scope/pkg".to_string()]);
        assert!(!filtered.packages.contains_key("@scope/pkg@1.0.0"));
        assert!(filtered.packages.contains_key("bar@2.0.0"));
    }

    #[test]
    fn test_filter_lockfile_preserves_unrelated() {
        let lf = make_lockfile(&[("foo", "1.0.0"), ("bar", "2.0.0"), ("baz", "3.0.0")]);
        let filtered = filter_lockfile(&lf, &["foo".to_string(), "baz".to_string()]);
        assert!(!filtered.packages.contains_key("foo@1.0.0"));
        assert!(filtered.packages.contains_key("bar@2.0.0"));
        assert!(!filtered.packages.contains_key("baz@3.0.0"));
    }

    #[test]
    fn test_filter_lockfile_empty_update_list() {
        let lf = make_lockfile(&[("foo", "1.0.0"), ("bar", "2.0.0")]);
        let filtered = filter_lockfile(&lf, &[]);
        assert_eq!(filtered.packages.len(), 2);
        assert!(filtered.packages.contains_key("foo@1.0.0"));
        assert!(filtered.packages.contains_key("bar@2.0.0"));
    }

    fn make_change(name: &str, current: &str, updated: &str) -> Change {
        Change {
            name: name.to_string(),
            current: current.to_string(),
            updated: updated.to_string(),
            old_range: format!("^{current}"),
            new_range: format!("^{updated}"),
        }
    }

    #[test]
    fn test_print_changes_table_without_ranges() {
        let changes = vec![
            make_change("@test/foo", "1.0.0", "1.1.0"),
            make_change("@test/bar", "2.0.0", "3.0.0"),
        ];
        print_changes_table(&changes, false);
    }

    #[test]
    fn test_print_changes_table_with_ranges() {
        let changes = vec![
            make_change("@test/foo", "1.0.0", "1.1.0"),
            make_change("@test/bar", "2.0.0", "3.0.0"),
        ];
        print_changes_table(&changes, true);
    }

    #[test]
    fn test_print_changes_table_single() {
        let changes = vec![make_change("pkg", "1.0.0", "1.0.1")];
        print_changes_table(&changes, false);
        print_changes_table(&changes, true);
    }

    #[test]
    fn test_print_changes_table_empty() {
        print_changes_table(&[], false);
        print_changes_table(&[], true);
    }
}
