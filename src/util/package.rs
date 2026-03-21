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
}
