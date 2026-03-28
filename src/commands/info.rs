use console::Style;

use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;

pub struct InfoOptions<'a> {
    pub package: &'a str,
    pub json: bool,
    pub registry: Option<&'a str>,
}

#[allow(clippy::too_many_lines)]
pub async fn run(opts: InfoOptions<'_>) -> Result<(), AppError> {
    let client = ApiClient::new(opts.registry)?;
    let metadata = client.get_package(opts.package).await?;

    if opts.json {
        let json_val = serde_json::to_string_pretty(&serde_json::json!({
            "name": metadata.name,
            "description": metadata.description,
            "distTags": metadata.dist_tags,
            "versions": metadata.versions.keys().collect::<Vec<_>>(),
            "maintainers": metadata.maintainers.iter().map(|m| &m.username).collect::<Vec<_>>(),
            "createdAt": metadata.created_at,
            "updatedAt": metadata.updated_at,
        }))?;
        println!("{json_val}");
        return Ok(());
    }

    let name_style = Style::new().bold().cyan();
    let version_style = Style::new().green();
    let header_style = Style::new().bold();
    let dim_style = Style::new().dim();

    // Name + latest version
    let latest = metadata
        .dist_tags
        .get("latest")
        .map_or("?.?.?", std::string::String::as_str);
    println!(
        "\n{} {}",
        name_style.apply_to(&metadata.name),
        version_style.apply_to(latest)
    );

    // Description
    if let Some(desc) = &metadata.description {
        if !desc.is_empty() {
            println!("{desc}");
        }
    }
    println!();

    // Dist-tags
    if !metadata.dist_tags.is_empty() {
        println!("{}", header_style.apply_to("Dist-Tags:"));
        for (tag, version) in &metadata.dist_tags {
            println!("  {}: {}", tag, version_style.apply_to(version));
        }
        println!();
    }

    // Versions
    if !metadata.versions.is_empty() {
        println!("{}", header_style.apply_to("Versions:"));
        let mut versions: Vec<(&String, &crate::api::types::VersionMetadata)> =
            metadata.versions.iter().collect();
        versions.sort_by(|a, b| {
            let va = semver::Version::parse(&a.1.version);
            let vb = semver::Version::parse(&b.1.version);
            match (va, vb) {
                (Ok(a), Ok(b)) => b.cmp(&a),
                _ => b.0.cmp(a.0),
            }
        });
        for (_, v) in &versions {
            let yanked = v.yanked.unwrap_or(false);
            let type_str = v
                .package_type
                .as_deref()
                .map(|t| format!(" [{t}]"))
                .unwrap_or_default();
            let date = v
                .published_at
                .as_deref()
                .map(|d| format!("  {}", dim_style.apply_to(d)))
                .unwrap_or_default();
            if yanked {
                println!(
                    "  {} {}{}",
                    dim_style.apply_to(&v.version),
                    Style::new().red().apply_to("(yanked)"),
                    date
                );
            } else {
                println!(
                    "  {}{}{}",
                    version_style.apply_to(&v.version),
                    type_str,
                    date
                );
            }
        }
        println!();
    }

    // Maintainers
    if !metadata.maintainers.is_empty() {
        println!("{}", header_style.apply_to("Maintainers:"));
        for m in &metadata.maintainers {
            let role = m
                .role
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            println!("  {}{}", m.username, dim_style.apply_to(role));
        }
        println!();
    }

    // Timestamps
    if let Some(created) = &metadata.created_at {
        display::label_value("Created", created);
    }
    if let Some(updated) = &metadata.updated_at {
        display::label_value("Updated", updated);
    }

    Ok(())
}
