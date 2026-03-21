use std::env;
use std::path::Path;

use dialoguer::{Confirm, Input, Select};

use crate::config::manifest::{self, Manifest, PackageType, MANIFEST_FILE};
use crate::error::AppError;
use crate::util::display;

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

    let name: String = Input::new()
        .with_prompt("Package name")
        .default(dir_name)
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

    let m = Manifest {
        name: name.clone(),
        version,
        package_type,
        description,
        license,
        readme,
        keywords,
        author: None,
        repository: None,
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
    let re = regex_lite::Regex::new(r"^(@[a-z0-9-]+/)?[a-z0-9]([a-z0-9._-]*[a-z0-9])?$").unwrap();
    if !re.is_match(name) {
        return Err(
            "Invalid name. Use lowercase letters, numbers, hyphens. Scoped: @org/name".to_string(),
        );
    }
    Ok(())
}
