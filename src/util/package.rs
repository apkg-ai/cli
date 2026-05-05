use std::sync::LazyLock;

use regex_lite::Regex;

use crate::error::AppError;

/// Matches a valid apkg package name — scoped (`@scope/name`) or unscoped
/// (`name`). Allowed characters: lowercase letters, digits, `-`, `.`, `_`.
/// The name may not start or end with a separator; the scope segment may not
/// contain dots or underscores. Safe to use as a path segment under
/// `apkg_packages/` because it contains no `..`, no path separators beyond the
/// scope slash, no null bytes, and no other shell-special characters.
pub static PACKAGE_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(@[a-z0-9-]+/)?[a-z0-9]([a-z0-9._-]*[a-z0-9])?$").unwrap());

/// Matches a scoped package name — `@scope/name` only. Used by `publish` and
/// `init` to enforce the registry's namespacing rule.
pub static SCOPED_PACKAGE_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@[a-z0-9-]+/[a-z0-9]([a-z0-9._-]*[a-z0-9])?$").unwrap());

/// Returns `true` if `name` is safe to use as a path segment under
/// `apkg_packages/`.
pub fn is_safe_package_name(name: &str) -> bool {
    PACKAGE_NAME_RE.is_match(name)
}

/// Validate that `name` is safe to use as a filesystem path segment.
/// Returns the name unchanged on success so callers can write
/// `cwd.join("apkg_packages").join(validate_package_name(n)?)`.
pub fn validate_package_name(name: &str) -> Result<&str, AppError> {
    if is_safe_package_name(name) {
        Ok(name)
    } else {
        Err(AppError::Other(format!(
            "Invalid package name '{name}': must match @scope/name or name, lowercase letters/digits/dots/hyphens/underscores only."
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepCategory {
    Dependencies,
    DevDependencies,
    PeerDependencies,
}

impl DepCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dependencies => "dependencies",
            Self::DevDependencies => "devDependencies",
            Self::PeerDependencies => "peerDependencies",
        }
    }
}

/// Parse a package spec like `name`, `name@version`, `@scope/name`, or `@scope/name@version`
/// into `(name, Option<version>)`.
pub fn parse_package_spec(spec: &str) -> (String, Option<&str>) {
    // Handle scoped packages: @scope/name@version
    if let Some(rest) = spec.strip_prefix('@') {
        // Find the second @ (version separator)
        if let Some(at_pos) = rest.find('@') {
            let slash_pos = rest.find('/');
            // Make sure the @ is after the slash (it's a version, not part of scope)
            if let Some(sp) = slash_pos {
                if at_pos > sp {
                    let name = &spec[..=at_pos]; // +1 for the leading @
                    let version = &spec[at_pos + 2..]; // +2 to skip both @ chars
                    return (name.to_string(), Some(version));
                }
            }
        }
        (spec.to_string(), None)
    } else if let Some(at_pos) = spec.find('@') {
        let name = &spec[..at_pos];
        let version = &spec[at_pos + 1..];
        (name.to_string(), Some(version))
    } else {
        (spec.to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dep_category_label() {
        assert_eq!(DepCategory::Dependencies.label(), "dependencies");
        assert_eq!(DepCategory::DevDependencies.label(), "devDependencies");
        assert_eq!(DepCategory::PeerDependencies.label(), "peerDependencies");
    }

    #[test]
    fn test_parse_unscoped() {
        let (name, ver) = parse_package_spec("my-package");
        assert_eq!(name, "my-package");
        assert_eq!(ver, None);
    }

    #[test]
    fn test_parse_unscoped_with_version() {
        let (name, ver) = parse_package_spec("my-package@1.2.3");
        assert_eq!(name, "my-package");
        assert_eq!(ver, Some("1.2.3"));
    }

    #[test]
    fn test_parse_scoped() {
        let (name, ver) = parse_package_spec("@scope/pkg");
        assert_eq!(name, "@scope/pkg");
        assert_eq!(ver, None);
    }

    #[test]
    fn test_parse_scoped_with_version() {
        let (name, ver) = parse_package_spec("@scope/pkg@2.0.0");
        assert_eq!(name, "@scope/pkg");
        assert_eq!(ver, Some("2.0.0"));
    }

    #[test]
    fn test_parse_scoped_with_tag() {
        let (name, ver) = parse_package_spec("@scope/pkg@latest");
        assert_eq!(name, "@scope/pkg");
        assert_eq!(ver, Some("latest"));
    }

    // --- validate_package_name ---

    #[test]
    fn test_validate_accepts_scoped() {
        assert!(validate_package_name("@acme/foo").is_ok());
        assert!(validate_package_name("@a/b").is_ok());
        assert!(validate_package_name("@my-org/my.pkg_v2").is_ok());
    }

    #[test]
    fn test_validate_accepts_unscoped() {
        assert!(validate_package_name("foo").is_ok());
        assert!(validate_package_name("my-pkg").is_ok());
        assert!(validate_package_name("a1b2c3").is_ok());
    }

    #[test]
    fn test_validate_rejects_path_traversal_simple() {
        assert!(validate_package_name("../evil").is_err());
        assert!(validate_package_name("..").is_err());
    }

    #[test]
    fn test_validate_rejects_path_traversal_in_scope() {
        assert!(validate_package_name("@evil/../foo").is_err());
        assert!(validate_package_name("@../foo").is_err());
    }

    #[test]
    fn test_validate_rejects_absolute_path() {
        assert!(validate_package_name("/etc/passwd").is_err());
        assert!(validate_package_name("/foo").is_err());
    }

    #[test]
    fn test_validate_rejects_windows_separator() {
        assert!(validate_package_name("foo\\bar").is_err());
        assert!(validate_package_name("@scope\\foo").is_err());
    }

    #[test]
    fn test_validate_rejects_null_byte() {
        assert!(validate_package_name("foo\0bar").is_err());
    }

    #[test]
    fn test_validate_rejects_nested_scope() {
        // Extra slash: second path segment beyond the scope is not allowed.
        assert!(validate_package_name("@a/b/c").is_err());
    }

    #[test]
    fn test_validate_rejects_uppercase() {
        assert!(validate_package_name("@Foo/bar").is_err());
        assert!(validate_package_name("Foo").is_err());
    }

    #[test]
    fn test_validate_rejects_empty() {
        assert!(validate_package_name("").is_err());
    }

    #[test]
    fn test_validate_rejects_leading_dot() {
        assert!(validate_package_name(".hidden").is_err());
        assert!(validate_package_name("@a/.hidden").is_err());
    }

    #[test]
    fn test_is_safe_package_name_matches_validate() {
        assert!(is_safe_package_name("@acme/foo"));
        assert!(is_safe_package_name("foo"));
        assert!(!is_safe_package_name("../evil"));
        assert!(!is_safe_package_name(""));
    }
}
