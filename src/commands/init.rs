use std::env;
use std::path::Path;

use dialoguer::{Confirm, Input, Select};

use crate::config::credentials;
use crate::config::manifest::{self, Author, Manifest, PackageType, MANIFEST_FILE};
use crate::error::AppError;
use crate::util::display;
use crate::util::git;

#[derive(Clone, Copy)]
pub struct InitOptions {
    pub force: bool,
}

pub fn run(opts: InitOptions) -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let manifest_path = cwd.join(MANIFEST_FILE);

    if manifest_path.exists() && !opts.force {
        return Err(AppError::FileExists(MANIFEST_FILE.to_string()));
    }

    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-package")
        .to_string();

    let creds = credentials::load().ok().flatten();
    let scope = creds.as_ref().map(|c| c.username.as_str());

    let default_name = match scope {
        Some(s) => format!("@{s}/{dir_name}"),
        None => format!("@scope/{dir_name}"),
    };

    let name: String = Input::new()
        .with_prompt("Package name (@scope/name)")
        .default(default_name)
        .validate_with(|input: &String| -> Result<(), String> {
            validate_package_name(input)
        })
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

    let version: String = Input::new()
        .with_prompt("Version")
        .default("0.1.0".to_string())
        .validate_with(|input: &String| -> Result<(), String> {
            semver::Version::parse(input)
                .map(|_| ())
                .map_err(|e| format!("Invalid semver: {e}"))
        })
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

    let type_idx = Select::new()
        .with_prompt("Package type")
        .items(PackageType::VARIANTS)
        .default(0)
        .interact()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;
    let package_type: PackageType =
        serde_json::from_str(&format!("\"{}\"", PackageType::VARIANTS[type_idx]))
            .map_err(|e| AppError::Other(format!("Type parse error: {e}")))?;

    let description: String = Input::new()
        .with_prompt("Description")
        .default(String::new())
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

    let license: String = Input::new()
        .with_prompt("License")
        .default("MIT".to_string())
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

    let keywords_input: String = Input::new()
        .with_prompt("Keywords (comma-separated)")
        .default(String::new())
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;
    let keywords: Option<Vec<String>> = if keywords_input.is_empty() {
        None
    } else {
        Some(
            keywords_input
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        )
    };

    let readme = if Path::new("README.md").exists() {
        Some("README.md".to_string())
    } else {
        None
    };

    let authors = {
        let name = git::get_user_name().or_else(|| scope.map(|s| s.to_string()));
        let email = git::get_user_email();
        name.map(|n| {
            vec![Author {
                name: n,
                email,
                extra: std::collections::BTreeMap::new(),
            }]
        })
    };
    let repository = git::get_repository_url().or_else(|| {
        scope.map(|s| format!("https://github.com/{s}/{dir_name}"))
    });

    let m = Manifest {
        name: name.clone(),
        version,
        package_type,
        description,
        license,
        readme,
        keywords,
        authors,
        repository,
        homepage: None,
        dependencies: None,
        dev_dependencies: None,
        peer_dependencies: None,
        scripts: None,
        hook_permissions: None,
    };

    manifest::save(&cwd, &m)?;

    display::success(&format!("Created {MANIFEST_FILE} for {name}"));

    if Confirm::new()
        .with_prompt("Would you like to see the generated manifest?")
        .default(false)
        .interact()
        .unwrap_or(false)
    {
        let content = std::fs::read_to_string(&manifest_path)?;
        println!("{content}");
    }

    Ok(())
}

fn validate_package_name(name: &str) -> Result<(), String> {
    if name.len() > 214 {
        return Err("Package name must be 214 characters or fewer".to_string());
    }
    let re = regex_lite::Regex::new(r"^@[a-z0-9-]+/[a-z0-9]([a-z0-9._-]*[a-z0-9])?$").unwrap();
    if !re.is_match(name) {
        return Err(
            "Package name must be scoped: @username/name or @org/name. Use lowercase letters, numbers, hyphens.".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_package_names() {
        assert!(validate_package_name("@user/pkg").is_ok());
        assert!(validate_package_name("@org/a-b.c_d").is_ok());
        assert!(validate_package_name("@a/b").is_ok());
        assert!(validate_package_name("@my-org/my-package").is_ok());
        assert!(validate_package_name("@user/pkg123").is_ok());
    }

    #[test]
    fn test_invalid_no_scope() {
        assert!(validate_package_name("no-scope").is_err());
    }

    #[test]
    fn test_invalid_empty_name() {
        assert!(validate_package_name("@user/").is_err());
    }

    #[test]
    fn test_invalid_empty_scope() {
        assert!(validate_package_name("@/pkg").is_err());
    }

    #[test]
    fn test_invalid_uppercase() {
        assert!(validate_package_name("@USER/pkg").is_err());
        assert!(validate_package_name("@user/Pkg").is_err());
    }

    #[test]
    fn test_invalid_too_long() {
        let long = format!("@user/{}", "a".repeat(210));
        assert!(validate_package_name(&long).is_err());
    }
}
