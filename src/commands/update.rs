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

    let result = resolver::resolve(
        &client,
        &resolution_deps,
        filtered_lockfile.as_ref(),
        &pb,
        &cwd,
    )
    .await?;

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
        let names: Vec<&str> = changes.iter().map(|c| c.name.as_str()).collect();
        install::download_resolved_subset(&client, &result, &names, &cwd, &dl_pb).await?;
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
    install::run_setup_for_result(&result, &cwd, opts.setup_target.as_ref());

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
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use std::collections::BTreeMap;

    use super::*;
    use crate::config::lockfile::{LockedPackage, Lockfile, LOCKFILE_VERSION};
    use crate::test_utils::env_lock;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Restores CWD to the crate root on drop. Declare **after** the tempdir
    /// in a test so it drops *before* the tempdir is deleted.
    struct CwdGuard;
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
        }
    }

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

    // --- async tests for run() ---

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

    #[tokio::test]
    async fn test_update_empty_deps() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_project_manifest(tmp.path(), &[]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run(UpdateOptions {
            package: None,
            registry: Some("http://localhost:1"),
            latest: false,
            dry_run: false,
            setup_target: None,
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_unknown_package() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run(UpdateOptions {
            package: Some("nonexistent"),
            registry: Some("http://localhost:1"),
            latest: false,
            dry_run: false,
            setup_target: None,
        })
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not in dependencies"));
    }

    // --- End-to-end tests via wiremock ---

    /// Minimal `.tar.zst` with a single `apkg.json` so resolver's extract-to-
    /// discover-deps path works cleanly.
    fn make_test_tarball() -> Vec<u8> {
        use std::io::Write;
        let buf = Vec::new();
        let enc = zstd::Encoder::new(buf, 3).unwrap();
        let mut archive = tar::Builder::new(enc);
        let content = br#"{"name":"dummy","version":"1.0.0","type":"skill","description":"","license":"MIT","platform":["claude"]}"#;
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "apkg.json", &content[..])
            .unwrap();
        let enc = archive.into_inner().unwrap();
        let mut enc = enc;
        enc.flush().unwrap();
        enc.finish().unwrap()
    }

    /// Build a `PackageMetadata` JSON that satisfies the resolver:
    ///   - `distTags.latest` points to `version`
    ///   - `versions[version]` has the `dist.integrity` the resolver checks
    fn package_metadata_json(name: &str, version: &str, integrity: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "distTags": { "latest": version },
            "versions": {
                version: {
                    "version": version,
                    "type": "skill",
                    "dist": {
                        "tarball": format!("https://example.test/{name}/{version}/tarball"),
                        "integrity": integrity,
                    },
                    "dependencies": {}
                }
            }
        })
    }

    #[tokio::test]
    async fn test_update_dry_run_reports_no_changes_when_up_to_date() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        let tarball = make_test_tarball();
        let integrity = crate::util::integrity::sha256_integrity(&tarball);

        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        // Seed lockfile at 1.0.0 — resolver filters it, but the server returns
        // the same version → no change.
        let seeded = make_lockfile(&[("foo", "1.0.0")]);
        lockfile::save(tmp.path(), &seeded).unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/foo"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(package_metadata_json("foo", "1.0.0", &integrity)),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/foo/1\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let result = run(UpdateOptions {
            package: Some("foo"),
            registry: Some(&server.uri()),
            latest: false,
            dry_run: true,
            setup_target: None,
        })
        .await;

        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[tokio::test]
    async fn test_update_writes_lockfile_when_version_advances() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        let tarball = make_test_tarball();
        let integrity = crate::util::integrity::sha256_integrity(&tarball);

        // Seed lockfile at 1.0.0, server advertises 1.0.1 → update should
        // resolve to 1.0.1, download, and save a new lockfile entry.
        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        let seeded = make_lockfile(&[("foo", "1.0.0")]);
        lockfile::save(tmp.path(), &seeded).unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/foo"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(package_metadata_json("foo", "1.0.1", &integrity)),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/foo/1\\.0\\.1/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let result = run(UpdateOptions {
            package: Some("foo"),
            registry: Some(&server.uri()),
            latest: false,
            dry_run: false,
            setup_target: None,
        })
        .await;

        assert!(result.is_ok(), "{:?}", result.err());

        // Lockfile should now reference 1.0.1.
        let lf = lockfile::load(tmp.path()).unwrap().unwrap();
        assert!(
            lf.packages.contains_key("foo@1.0.1"),
            "lockfile missing updated version: {:?}",
            lf.packages.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_update_latest_rewrites_manifest_range() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let _cwd = CwdGuard;

        let tarball = make_test_tarball();
        let integrity = crate::util::integrity::sha256_integrity(&tarball);

        // Manifest pins ^1.0.0; --latest + a 2.0.0 server version should
        // rewrite the manifest range to ^2.0.0.
        write_project_manifest(tmp.path(), &[("foo", "^1.0.0")]);
        let seeded = make_lockfile(&[("foo", "1.0.0")]);
        lockfile::save(tmp.path(), &seeded).unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/foo"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(package_metadata_json("foo", "2.0.0", &integrity)),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("/packages/foo/2\\.0\\.0/tarball"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball.clone()))
            .mount(&server)
            .await;

        let result = run(UpdateOptions {
            package: Some("foo"),
            registry: Some(&server.uri()),
            latest: true,
            dry_run: false,
            setup_target: None,
        })
        .await;

        assert!(result.is_ok(), "{:?}", result.err());

        // Manifest range should be rewritten to ^2.0.0.
        let m = manifest::load(tmp.path()).unwrap();
        assert_eq!(m.dependencies.unwrap().get("foo").unwrap(), "^2.0.0");
    }
}
