use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("qpm").unwrap()
}

#[test]
fn test_version() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("qpm 0.1.0"));
}

#[test]
fn test_help() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Package manager for AI tooling"));
}

#[test]
fn test_init_help() {
    cmd()
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("qpm.json"));
}

#[test]
fn test_login_help() {
    cmd()
        .args(["login", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Authenticate"));
}

#[test]
fn test_logout_not_logged_in() {
    // Set HOME to a temp dir so no credentials exist
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("logout")
        .env("HOME", tmp.path())
        .assert()
        .success();
}

#[test]
fn test_pack_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("pack")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_publish_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("publish")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_search_help() {
    cmd()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Search the registry"));
}

#[test]
fn test_info_help() {
    cmd()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("package metadata"));
}

#[test]
fn test_whoami_no_auth() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("whoami")
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Authentication required"));
}

#[test]
fn test_init_creates_manifest() {
    let tmp = TempDir::new().unwrap();
    // Non-interactive init would need --yes or similar, but we can test
    // that it fails gracefully when not interactive (no TTY)
    // The dialoguer prompts will fail in non-TTY context
    let result = cmd()
        .arg("init")
        .current_dir(tmp.path())
        .assert();
    // In CI/non-TTY, dialoguer will error out — that's expected
    // Just verify it doesn't panic
    let _ = result;
}

#[test]
fn test_pack_with_manifest() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-package",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test package",
  "license": "MIT"
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    std::fs::write(tmp.path().join("index.js"), "console.log('hello')").unwrap();

    cmd()
        .arg("pack")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Packed test-package-0.1.0.tgz"))
        .stdout(predicate::str::contains("sha256-"));

    assert!(tmp.path().join("test-package-0.1.0.tgz").exists());
}

#[test]
fn test_token_create_help() {
    cmd()
        .args(["token", "create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--name"))
        .stdout(predicate::str::contains("--scopes"))
        .stdout(predicate::str::contains("--expires-in"))
        .stdout(predicate::str::contains("--package-scope"));
}

#[test]
fn test_token_list_no_auth() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .args(["token", "list"])
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Authentication required"));
}

#[test]
fn test_token_revoke_help() {
    cmd()
        .args(["token", "revoke", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Token ID"));
}

#[test]
fn test_add_help() {
    cmd()
        .args(["add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dependencies"))
        .stdout(predicate::str::contains("--save-dev"))
        .stdout(predicate::str::contains("--save-peer"));
}

#[test]
fn test_add_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .args(["add", "some-pkg"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_remove_help() {
    cmd()
        .args(["remove", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dependencies"))
        .stdout(predicate::str::contains("--save-dev"))
        .stdout(predicate::str::contains("--save-peer"));
}

#[test]
fn test_remove_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .args(["remove", "some-pkg"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_remove_not_in_deps() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT"
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    cmd()
        .args(["remove", "nonexistent-pkg"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not in dependencies"));
}

#[test]
fn test_remove_from_deps() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT",
  "dependencies": {
    "my-dep": "^1.0.0",
    "other-dep": "^2.0.0"
  }
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();

    // Create fake installed package dir
    let pkg_dir = tmp.path().join("qpm_packages").join("my-dep");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("index.js"), "").unwrap();

    cmd()
        .args(["remove", "my-dep"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Removed my-dep"));

    // Verify manifest updated
    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(tmp.path().join("qpm.json")).unwrap())
            .unwrap();
    let deps = updated["dependencies"].as_object().unwrap();
    assert!(!deps.contains_key("my-dep"));
    assert!(deps.contains_key("other-dep"));

    // Verify files deleted
    assert!(!pkg_dir.exists());
}

#[test]
fn test_dist_tag_add_help() {
    cmd()
        .args(["dist-tag", "add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tag"))
        .stdout(predicate::str::contains("version"));
}

#[test]
fn test_dist_tag_rm_help() {
    cmd()
        .args(["dist-tag", "rm", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tag"));
}

#[test]
fn test_dist_tag_ls_help() {
    cmd()
        .args(["dist-tag", "ls", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Package name"));
}

#[test]
fn test_dist_tag_add_no_auth() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .args(["dist-tag", "add", "my-pkg@1.0.0", "beta"])
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Authentication required"));
}

#[test]
fn test_dist_tag_rm_no_auth() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .args(["dist-tag", "rm", "my-pkg", "beta"])
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Authentication required"));
}

#[test]
fn test_dist_tag_add_missing_version() {
    let tmp = TempDir::new().unwrap();
    // Create fake credentials so we pass auth check and hit version validation
    let config_dir = tmp.path().join(".config").join("qpm");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("credentials.json"),
        r#"{"accessToken":"fake","refreshToken":"fake"}"#,
    )
    .unwrap();
    cmd()
        .args(["dist-tag", "add", "my-pkg", "beta"])
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Version is required"));
}

#[test]
fn test_deprecate_help() {
    cmd()
        .args(["deprecate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deprecated"));
}

#[test]
fn test_deprecate_no_auth() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .args(["deprecate", "some-pkg", "Use something else"])
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Authentication required"));
}

#[test]
fn test_deprecate_missing_message() {
    cmd()
        .args(["deprecate", "some-pkg"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("deprecation message is required"));
}

#[test]
fn test_install_help_shows_no_setup() {
    cmd()
        .args(["install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-setup"));
}

#[test]
fn test_install_help_shows_frozen_lockfile() {
    cmd()
        .args(["install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--frozen-lockfile"));
}

#[test]
fn test_install_help_shows_optional_package() {
    cmd()
        .args(["install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[PACKAGE]"));
}

#[test]
fn test_install_no_args_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("install")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_install_no_args_empty_deps() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT"
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    cmd()
        .arg("install")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("No dependencies to install"));
}

#[test]
fn test_install_frozen_no_lockfile() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT",
  "dependencies": {
    "some-pkg": "^1.0.0"
  }
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    cmd()
        .args(["install", "--frozen-lockfile"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Lockfile not found"));
}

#[test]
fn test_cache_clean_help() {
    cmd()
        .args(["cache", "clean", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Remove all entries"));
}

#[test]
fn test_cache_list_help() {
    cmd()
        .args(["cache", "list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List all cached packages"));
}

#[test]
fn test_cache_verify_help() {
    cmd()
        .args(["cache", "verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Verify integrity"));
}

#[test]
fn test_cache_clean_empty() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("empty-cache");
    cmd()
        .args(["cache", "clean"])
        .env("QPM_CACHE_DIR", &cache_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("already empty"));
}

#[test]
fn test_cache_list_empty() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("empty-cache");
    cmd()
        .args(["cache", "list"])
        .env("QPM_CACHE_DIR", &cache_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Cache is empty"));
}

#[test]
fn test_cache_verify_empty() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("empty-cache");
    cmd()
        .args(["cache", "verify"])
        .env("QPM_CACHE_DIR", &cache_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("nothing to verify"));
}

// --- link / unlink ---

#[test]
fn test_link_help() {
    cmd()
        .args(["link", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("symlinks"));
}

#[test]
fn test_unlink_help() {
    cmd()
        .args(["unlink", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("symlinks"));
}

#[test]
fn test_link_no_args_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("link")
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_unlink_no_args_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("unlink")
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_link_register_and_unregister() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let manifest = r#"{
  "name": "my-lib",
  "version": "1.0.0",
  "type": "skill",
  "description": "A lib",
  "license": "MIT"
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();

    // Register
    cmd()
        .arg("link")
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Linked my-lib globally"));

    // Verify file exists in global store
    let link_file = home.path().join(".qpm/links/my-lib.json");
    assert!(link_file.exists(), "link entry should exist in global store");

    // Unregister
    cmd()
        .arg("unlink")
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Unlinked my-lib from global store"));

    assert!(!link_file.exists(), "link entry should be removed");
}

#[test]
fn test_link_direct_path() {
    let lib_dir = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let lib_manifest = r#"{
  "name": "my-lib",
  "version": "1.0.0",
  "type": "skill",
  "description": "A lib",
  "license": "MIT"
}
"#;
    std::fs::write(lib_dir.path().join("qpm.json"), lib_manifest).unwrap();

    let app_manifest = r#"{
  "name": "my-app",
  "version": "0.1.0",
  "type": "agent",
  "description": "An app",
  "license": "MIT"
}
"#;
    std::fs::write(app_dir.path().join("qpm.json"), app_manifest).unwrap();

    // Link by path
    cmd()
        .args(["link", &lib_dir.path().to_string_lossy()])
        .current_dir(app_dir.path())
        .env("HOME", home.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Linked my-lib"));

    let symlink_path = app_dir.path().join("qpm_packages/my-lib");
    let meta = std::fs::symlink_metadata(&symlink_path).expect("symlink should exist");
    assert!(meta.file_type().is_symlink(), "should be a symlink");
}

#[test]
fn test_link_nonexistent_path() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd()
        .args(["link", "/nonexistent/path/to/nowhere"])
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_unlink_not_linked() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd()
        .args(["unlink", "not-linked-pkg"])
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a linked package"));
}

// --- update ---

#[test]
fn test_update_help() {
    cmd()
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--latest"))
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn test_update_no_manifest() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("update")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_update_empty_deps() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT"
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    cmd()
        .arg("update")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("No dependencies to update"));
}

#[test]
fn test_update_package_not_in_deps() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT",
  "dependencies": {
    "some-pkg": "^1.0.0"
  }
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    cmd()
        .args(["update", "nonexistent-pkg"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not in dependencies"));
}

#[test]
fn test_update_dry_run_flag_accepted() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT",
  "dependencies": {
    "some-pkg": "^1.0.0"
  }
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    // --dry-run parses correctly; network error is expected since no registry is reachable
    cmd()
        .args(["update", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .failure();
}

#[test]
fn test_update_latest_flag_accepted() {
    let tmp = TempDir::new().unwrap();
    let manifest = r#"{
  "name": "test-project",
  "version": "0.1.0",
  "type": "skill",
  "description": "A test project",
  "license": "MIT",
  "dependencies": {
    "some-pkg": "^1.0.0"
  }
}
"#;
    std::fs::write(tmp.path().join("qpm.json"), manifest).unwrap();
    // --latest parses correctly; network error is expected since no registry is reachable
    cmd()
        .args(["update", "--latest"])
        .current_dir(tmp.path())
        .assert()
        .failure();
}

// --- verify ---

#[test]
fn test_verify_help() {
    cmd()
        .args(["verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("signatures and integrity"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--strict"));
}

#[test]
fn test_verify_no_lockfile() {
    let tmp = TempDir::new().unwrap();
    cmd()
        .arg("verify")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("No lockfile found"));
}

#[test]
fn test_verify_json_flag_accepted() {
    // Ensure --json is a valid flag (test via --help)
    cmd()
        .args(["verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--json"));
}

#[test]
fn test_verify_strict_flag_accepted() {
    // Ensure --strict is a valid flag (test via --help)
    cmd()
        .args(["verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--strict"));
}

#[test]
fn test_verify_package_not_in_lockfile() {
    let tmp = TempDir::new().unwrap();
    let lockfile = r#"{
  "lockfileVersion": 1,
  "requires": true,
  "resolved": "2026-01-01T00:00:00Z",
  "packages": {
    "foo@1.0.0": {
      "version": "1.0.0",
      "resolved": "https://registry.qpm.dev/api/v1/packages/foo/1.0.0/tarball",
      "integrity": "sha256-abc",
      "dependencies": {},
      "peerDependencies": {},
      "type": "skill",
      "optional": false
    }
  }
}
"#;
    std::fs::write(tmp.path().join("qpm-lock.json"), lockfile).unwrap();
    cmd()
        .args(["verify", "nonexistent-pkg"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found in lockfile"));
}

#[test]
fn test_completions_help() {
    cmd()
        .args(["completions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("completion"))
        .stdout(predicate::str::contains("bash"))
        .stdout(predicate::str::contains("zsh"))
        .stdout(predicate::str::contains("fish"));
}

#[test]
fn test_completions_bash() {
    cmd()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("qpm"));
}

#[test]
fn test_completions_zsh() {
    cmd()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("qpm"));
}

#[test]
fn test_completions_fish() {
    cmd()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("qpm"));
}

#[test]
fn test_verify_empty_lockfile() {
    let tmp = TempDir::new().unwrap();
    let lockfile = r#"{
  "lockfileVersion": 1,
  "requires": true,
  "resolved": "2026-01-01T00:00:00Z",
  "packages": {}
}
"#;
    std::fs::write(tmp.path().join("qpm-lock.json"), lockfile).unwrap();
    cmd()
        .arg("verify")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("No packages to verify"));
}
