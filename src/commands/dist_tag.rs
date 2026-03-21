use console::Style;

use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;
use crate::util::package::parse_package_spec;

pub enum DistTagAction<'a> {
    Add {
        package_at_version: &'a str,
        tag: &'a str,
    },
    Rm {
        package: &'a str,
        tag: &'a str,
    },
    Ls {
        package: &'a str,
    },
}

pub async fn run(action: DistTagAction<'_>, registry: Option<&str>) -> Result<(), AppError> {
    match action {
        DistTagAction::Add {
            package_at_version,
            tag,
        } => run_add(package_at_version, tag, registry).await,
        DistTagAction::Rm { package, tag } => run_rm(package, tag, registry).await,
        DistTagAction::Ls { package } => run_ls(package, registry).await,
    }
}

async fn run_add(
    package_at_version: &str,
    tag: &str,
    registry: Option<&str>,
) -> Result<(), AppError> {
    let (name, version) = parse_package_spec(package_at_version);
    let version = version.ok_or_else(|| {
        AppError::Other(
            "Version is required. Usage: qpm dist-tag add <pkg>@<version> <tag>".to_string(),
        )
    })?;

    let client = ApiClient::new(registry)?;
    let result = client.set_dist_tag(&name, tag, version).await?;

    display::success(&format!(
        "Set tag \"{}\" on {}@{}",
        result.tag, name, result.version
    ));

    Ok(())
}

async fn run_rm(package: &str, tag: &str, registry: Option<&str>) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    client.remove_dist_tag(package, tag).await?;

    display::success(&format!("Removed tag \"{tag}\" from {package}"));

    Ok(())
}

async fn run_ls(package: &str, registry: Option<&str>) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let metadata = client.get_package(package).await?;

    if metadata.dist_tags.is_empty() {
        println!("No dist-tags for {package}");
        return Ok(());
    }

    let tag_style = Style::new().bold();
    let version_style = Style::new().green();

    // Find longest tag name for alignment
    let max_len = metadata.dist_tags.keys().map(String::len).max().unwrap_or(0);

    for (tag, version) in &metadata.dist_tags {
        println!(
            "{}  {}",
            tag_style.apply_to(format!("{tag:<max_len$}")),
            version_style.apply_to(version),
        );
    }

    Ok(())
}
