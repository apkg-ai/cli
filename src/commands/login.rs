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

fn prompt_mfa_code() -> Result<String, AppError> {
    display::info("MFA is enabled for this account.");
    Input::new()
        .with_prompt("Enter TOTP code or recovery code")
        .interact_text()
        .map_err(|e| AppError::Other(format!("Input error: {e}")))
}

async fn complete_mfa_challenge(
    client: &ApiClient,
    mfa_token: &str,
    code: &str,
) -> Result<(String, String), AppError> {
    let resp = client.mfa_challenge(mfa_token, code).await?;
    Ok((resp.access_token, resp.refresh_token))
}

fn persist_credentials(
    registry_arg: Option<&str>,
    username: &str,
    access_token: String,
    refresh_token: String,
) -> Result<(), AppError> {
    let settings = Settings::load()?;
    let registry_url = resolve_registry(registry_arg, &settings);

    credentials::save(&Credentials {
        registry: registry_url,
        access_token,
        refresh_token,
        username: username.to_string(),
    })?;

    display::success(&format!("Logged in as {username}"));
    Ok(())
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
            let code = prompt_mfa_code()?;
            complete_mfa_challenge(&client, &mfa_token, &code).await?
        }
    };

    persist_credentials(registry, &username, access_token, refresh_token)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use super::*;
    use crate::test_utils::env_lock;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
        let _lock = env_lock();
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
        let _lock = env_lock();
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
        let _lock = env_lock();
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
        let _lock = env_lock();
        unsafe { std::env::remove_var("APKG_REGISTRY") };
        let settings = Settings::default();
        let result = resolve_registry(None, &settings);
        assert_eq!(result, crate::config::DEFAULT_REGISTRY);
    }

    // --- persist_credentials tests ---

    #[test]
    fn test_persist_credentials_writes_file_with_cli_registry() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };
        unsafe { std::env::remove_var("APKG_REGISTRY") };

        persist_credentials(
            Some("http://override.test"),
            "alice",
            "tok_abc".to_string(),
            "rt_def".to_string(),
        )
        .unwrap();

        let loaded = credentials::load().unwrap().expect("credentials written");
        assert_eq!(loaded.registry, "http://override.test");
        assert_eq!(loaded.username, "alice");
        assert_eq!(loaded.access_token, "tok_abc");
        assert_eq!(loaded.refresh_token, "rt_def");
    }

    #[test]
    fn test_persist_credentials_defaults_to_settings_registry() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };
        unsafe { std::env::remove_var("APKG_REGISTRY") };

        let settings = Settings {
            registry: Some("http://settings.test".to_string()),
            ..Default::default()
        };
        settings.save().unwrap();

        persist_credentials(None, "alice", "tok".to_string(), "rt".to_string()).unwrap();

        let loaded = credentials::load().unwrap().expect("credentials written");
        assert_eq!(loaded.registry, "http://settings.test");
    }

    #[test]
    fn test_persist_credentials_falls_back_to_default_registry() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };
        unsafe { std::env::remove_var("APKG_REGISTRY") };

        persist_credentials(None, "alice", "tok".to_string(), "rt".to_string()).unwrap();

        let loaded = credentials::load().unwrap().expect("credentials written");
        assert_eq!(loaded.registry, crate::config::DEFAULT_REGISTRY);
    }

    // --- complete_mfa_challenge tests ---

    #[tokio::test]
    async fn test_complete_mfa_challenge_success() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/mfa/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "accessToken": "tok_mfa",
                "refreshToken": "rt_mfa",
                "expiresIn": 3600,
                "tokenType": "Bearer"
            })))
            .mount(&server)
            .await;

        let client = ApiClient::new(Some(&server.uri())).unwrap();
        let (access, refresh) = complete_mfa_challenge(&client, "mfa_tok", "123456")
            .await
            .unwrap();
        assert_eq!(access, "tok_mfa");
        assert_eq!(refresh, "rt_mfa");
    }

    #[tokio::test]
    async fn test_complete_mfa_challenge_returns_error_on_server_failure() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/mfa/challenge"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = ApiClient::new(Some(&server.uri())).unwrap();
        let err = complete_mfa_challenge(&client, "mfa_tok", "wrong")
            .await
            .unwrap_err();
        // Should surface as an API / network error, not silently succeed.
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("error") || msg.contains("500"),
            "unexpected error message: {err}"
        );
    }
}
