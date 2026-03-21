use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::{display, package::parse_package_spec};

pub struct DeprecateOptions<'a> {
    pub target: &'a str,
    pub message: Option<&'a str>,
    pub registry: Option<&'a str>,
}

pub async fn run(opts: DeprecateOptions<'_>) -> Result<(), AppError> {
    let (name, version) = parse_package_spec(opts.target);
    let client = ApiClient::new(opts.registry)?;

    if let Some(ver) = version {
        client.deprecate_version(&name, ver, opts.message).await?;
        if opts.message.is_some() {
            display::success(&format!("Deprecated {name}@{ver}"));
        } else {
            display::success(&format!("Removed deprecation from {name}@{ver}"));
        }
    } else {
        client.deprecate_package(&name, opts.message).await?;
        if opts.message.is_some() {
            display::success(&format!("Deprecated {name}"));
        } else {
            display::success(&format!("Removed deprecation from {name}"));
        }
    }

    Ok(())
}
