use dialoguer::{Input, Password};

use crate::api::client::ApiClient;
use crate::api::types::LoginResponse;
use crate::config::credentials::{self, Credentials};
use crate::config::settings::Settings;
use crate::error::AppError;
use crate::util::display;

#[derive(Debug)]
enum TokenOutcome {
    Tokens {
        access_token: String,
        refresh_token: String,
    },
    MfaRequired {
        mfa_token: String,
    },
}

fn extract_tokens(resp: LoginResponse) -> Result<TokenOutcome, AppError> {
    if resp.requires_mfa() {
        let mfa_token = resp
            .mfa_token
            .ok_or_else(|| AppError::Other("MFA required but no MFA token returned".to_string()))?;
        Ok(TokenOutcome::MfaRequired { mfa_token })
    } else {
        let access_token = resp
            .access_token
            .ok_or_else(|| AppError::Other("Login response missing access token".to_string()))?;
        let refresh_token = resp
            .refresh_token
            .ok_or_else(|| AppError::Other("Login response missing refresh token".to_string()))?;
        Ok(TokenOutcome::Tokens {
            access_token,
            refresh_token,
        })
    }
}

fn resolve_registry(registry_arg: Option<&str>, settings: &Settings) -> String {
    registry_arg
        .map(std::string::ToString::to_string)
        .or_else(|| std::env::var("APKG_REGISTRY").ok())
        .or_else(|| settings.registry.clone())
        .unwrap_or_else(|| crate::config::DEFAULT_REGISTRY.to_string())
}

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

    let (access_token, refresh_token) = match extract_tokens(resp)? {
        TokenOutcome::Tokens {
            access_token,
            refresh_token,
        } => (access_token, refresh_token),
        TokenOutcome::MfaRequired { mfa_token } => {
            display::info("MFA is enabled for this account.");
            let code: String = Input::new()
                .with_prompt("Enter TOTP code or recovery code")
                .interact_text()
                .map_err(|e| AppError::Other(format!("Input error: {e}")))?;

            let mfa_resp = client.mfa_challenge(&mfa_token, &code).await?;
            (mfa_resp.access_token, mfa_resp.refresh_token)
        }
    };

    let settings = Settings::load()?;
    let registry_url = resolve_registry(registry, &settings);

    credentials::save(&Credentials {
        registry: registry_url,
        access_token,
        refresh_token,
        username: username.clone(),
    })?;

    display::success(&format!("Logged in as {username}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_response(
        access_token: Option<&str>,
        refresh_token: Option<&str>,
        mfa_required: Option<bool>,
        mfa_token: Option<&str>,
    ) -> LoginResponse {
        LoginResponse {
            access_token: access_token.map(String::from),
            refresh_token: refresh_token.map(String::from),
            expires_in: None,
            token_type: None,
            mfa_required,
            mfa_token: mfa_token.map(String::from),
        }
    }

    // --- extract_tokens tests ---

    #[test]
    fn test_extract_tokens_success() {
        let resp = make_response(Some("tok_abc"), Some("rt_def"), None, None);
        let result = extract_tokens(resp).unwrap();
        match result {
            TokenOutcome::Tokens {
                access_token,
                refresh_token,
            } => {
                assert_eq!(access_token, "tok_abc");
                assert_eq!(refresh_token, "rt_def");
            }
            TokenOutcome::MfaRequired { .. } => panic!("expected Tokens variant"),
        }
    }

    #[test]
    fn test_extract_tokens_mfa_false_explicit() {
        let resp = make_response(Some("tok_abc"), Some("rt_def"), Some(false), None);
        let result = extract_tokens(resp).unwrap();
        match result {
            TokenOutcome::Tokens {
                access_token,
                refresh_token,
            } => {
                assert_eq!(access_token, "tok_abc");
                assert_eq!(refresh_token, "rt_def");
            }
            TokenOutcome::MfaRequired { .. } => panic!("expected Tokens variant"),
        }
    }

    #[test]
    fn test_extract_tokens_mfa_required() {
        let resp = make_response(None, None, Some(true), Some("mfa_tok_123"));
        let result = extract_tokens(resp).unwrap();
        match result {
            TokenOutcome::MfaRequired { mfa_token } => {
                assert_eq!(mfa_token, "mfa_tok_123");
            }
            TokenOutcome::Tokens { .. } => panic!("expected MfaRequired variant"),
        }
    }

    #[test]
    fn test_extract_tokens_mfa_takes_priority_over_tokens() {
        let resp = make_response(Some("tok_abc"), Some("rt_def"), Some(true), Some("mfa_tok"));
        let result = extract_tokens(resp).unwrap();
        match result {
            TokenOutcome::MfaRequired { mfa_token } => {
                assert_eq!(mfa_token, "mfa_tok");
            }
            TokenOutcome::Tokens { .. } => panic!("expected MfaRequired variant"),
        }
    }

    #[test]
    fn test_extract_tokens_mfa_missing_token() {
        let resp = make_response(None, None, Some(true), None);
        let err = extract_tokens(resp).unwrap_err();
        assert!(err
            .to_string()
            .contains("MFA required but no MFA token returned"));
    }

    #[test]
    fn test_extract_tokens_missing_access_token() {
        let resp = make_response(None, Some("rt_def"), None, None);
        let err = extract_tokens(resp).unwrap_err();
        assert!(err
            .to_string()
            .contains("Login response missing access token"));
    }

    #[test]
    fn test_extract_tokens_missing_refresh_token() {
        let resp = make_response(Some("tok_abc"), None, None, None);
        let err = extract_tokens(resp).unwrap_err();
        assert!(err
            .to_string()
            .contains("Login response missing refresh token"));
    }

    // --- resolve_registry tests ---

    #[test]
    fn test_resolve_registry_cli_arg_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("APKG_REGISTRY", "http://env.example") };
        let settings = Settings {
            registry: Some("http://settings.example".to_string()),
            ..Default::default()
        };
        let result = resolve_registry(Some("http://arg.example"), &settings);
        assert_eq!(result, "http://arg.example");
        unsafe { std::env::remove_var("APKG_REGISTRY") };
    }

    #[test]
    fn test_resolve_registry_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("APKG_REGISTRY", "http://env.example") };
        let settings = Settings {
            registry: Some("http://settings.example".to_string()),
            ..Default::default()
        };
        let result = resolve_registry(None, &settings);
        assert_eq!(result, "http://env.example");
        unsafe { std::env::remove_var("APKG_REGISTRY") };
    }

    #[test]
    fn test_resolve_registry_settings() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("APKG_REGISTRY") };
        let settings = Settings {
            registry: Some("http://settings.example".to_string()),
            ..Default::default()
        };
        let result = resolve_registry(None, &settings);
        assert_eq!(result, "http://settings.example");
    }

    #[test]
    fn test_resolve_registry_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("APKG_REGISTRY") };
        let settings = Settings::default();
        let result = resolve_registry(None, &settings);
        assert_eq!(result, crate::config::DEFAULT_REGISTRY);
    }
}
