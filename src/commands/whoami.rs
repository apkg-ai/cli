use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;

pub async fn run(registry: Option<&str>) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let resp = client.whoami().await?;

    display::label_value("Username", &resp.username);
    display::label_value("Email", &resp.email);
    if let Some(mfa) = resp.mfa_enabled {
        display::label_value("MFA", if mfa { "enabled" } else { "disabled" });
    }
    if !resp.scopes.is_empty() {
        display::label_value("Scopes", &resp.scopes.join(", "));
    }
    if !resp.orgs.is_empty() {
        display::label_value(
            "Organizations",
            &resp
                .orgs
                .iter()
                .map(|o| format!("{} ({})", o.name, o.role))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Ok(())
}
