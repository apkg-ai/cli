use std::env;

use indicatif::{ProgressBar, ProgressStyle};

use crate::api::client::ApiClient;
use crate::config::manifest;
use crate::error::AppError;
use crate::util::{display, integrity, tarball};

pub async fn run(registry: Option<&str>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let m = manifest::load(&cwd)?;

    display::info(&format!("Publishing {}@{} ...", m.name, m.version));

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message("Creating tarball...");
    pb.enable_steady_tick(std::time::Duration::from_millis(80));

    let data = tarball::create_tarball(&cwd)?;
    let hash = integrity::sha256_integrity(&data);
    let size = data.len();

    pb.set_message("Uploading to registry...");

    // Build metadata with only the fields the publish API accepts
    let mut metadata = serde_json::json!({
        "name": m.name,
        "version": m.version,
        "type": m.package_type,
        "integrity": hash,
    });
    let obj = metadata.as_object_mut().unwrap();
    if !m.description.is_empty() {
        obj.insert("description".into(), serde_json::json!(m.description));
    }
    if !m.license.is_empty() {
        obj.insert("license".into(), serde_json::json!(m.license));
    }
    if let Some(kw) = &m.keywords {
        obj.insert("keywords".into(), serde_json::json!(kw));
    }
    if let Some(deps) = &m.dependencies {
        obj.insert("dependencies".into(), serde_json::json!(deps));
    }

    let metadata_json = serde_json::to_string(&metadata)?;
    let client = ApiClient::new(registry)?;
    let resp = client.publish(&m.name, &metadata_json, data).await?;

    pb.finish_and_clear();

    display::success(&format!("Published {}@{}", resp.name, resp.version));
    display::label_value("Size", &display::format_size(size));
    display::label_value("Integrity", &hash);
    if let Some(server_integrity) = &resp.integrity {
        display::label_value("Server Integrity", server_integrity);
    }

    Ok(())
}
