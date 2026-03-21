use dialoguer::{Input, Password};

use crate::api::client::ApiClient;
use crate::config::credentials::{self, Credentials};
use crate::config::settings::Settings;
use crate::error::AppError;
use crate::util::display;

pub async fn run(registry: Option<&str>) -> Result<(), AppError> {
    let username: String = Input::new()
        .with_prompt("Username")
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

    let password: String = Password::new()
        .with_prompt("Password")
        .interact()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

    let client = ApiClient::new(registry)?;
    let resp = client.login(&username, &password).await?;

    let (access_token, refresh_token) = if resp.requires_mfa() {
        let mfa_token = resp.mfa_token.ok_or_else(|| {
            AppError::Other("MFA required but no MFA token returned".to_string())
        })?;

        display::info("MFA is enabled for this account.");
        let code: String = Input::new()
            .with_prompt("Enter TOTP code or recovery code")
            .interact_text()
            .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

        let mfa_resp = client.mfa_challenge(&mfa_token, &code).await?;
        (mfa_resp.access_token, mfa_resp.refresh_token)
    } else {
        let access_token = resp.access_token.ok_or_else(|| {
            AppError::Other("Login response missing access token".to_string())
        })?;
        let refresh_token = resp.refresh_token.ok_or_else(|| {
            AppError::Other("Login response missing refresh token".to_string())
        })?;
        (access_token, refresh_token)
    };

    let settings = Settings::load()?;
    let registry_url = registry
        .map(std::string::ToString::to_string)
        .or_else(|| std::env::var("QPM_REGISTRY").ok())
        .or(settings.registry.clone())
        .unwrap_or_else(|| crate::config::DEFAULT_REGISTRY.to_string());

    credentials::save(&Credentials {
        registry: registry_url,
        access_token,
        refresh_token,
        username: username.clone(),
    })?;

    display::success(&format!("Logged in as {username}"));
    Ok(())
}
