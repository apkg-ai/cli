use std::collections::{BTreeMap, VecDeque};

use indicatif::ProgressBar;
use semver::VersionReq;

use crate::api::client::ApiClient;
use crate::api::types::{PackageMetadata, VersionMetadata};
use crate::config::lockfile::{self, Lockfile};
use crate::error::AppError;

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
pub async fn resolve(
    client: &ApiClient,
    dependencies: &BTreeMap<String, String>,
    existing_lockfile: Option<&Lockfile>,
    pb: &ProgressBar,
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
                        let pkg = ResolvedPackage {
                            version: locked.version.clone(),
                            tarball_url: locked.resolved.clone(),
                            integrity: locked.integrity.clone(),
                            package_type: locked.package_type.clone(),
                            dependencies: locked.dependencies.clone(),
                            peer_dependencies: locked.peer_dependencies.clone(),
                        };
                        // Push transitive deps from locked entry as exact versions
                        for (dep_name, dep_version) in &locked.dependencies {
                            queue.push_back((dep_name.clone(), format!("={dep_version}")));
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

        let deps = version_meta.dependencies.clone().unwrap_or_default();

        let pkg = ResolvedPackage {
            version: version_meta.version.clone(),
            tarball_url: dist.tarball.clone(),
            integrity: dist.integrity.clone(),
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
}
