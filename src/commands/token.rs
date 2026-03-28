use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;

pub enum TokenAction<'a> {
    Create {
        name: &'a str,
        scopes: &'a [String],
        expires_in: &'a str,
        package_scope: Option<&'a str>,
    },
    List {
        json: bool,
    },
    Revoke {
        id: &'a str,
    },
}

pub async fn run(action: TokenAction<'_>, registry: Option<&str>) -> Result<(), AppError> {
    match action {
        TokenAction::Create {
            name,
            scopes,
            expires_in,
            package_scope,
        } => create(registry, name, scopes, expires_in, package_scope).await,
        TokenAction::List { json } => list(registry, json).await,
        TokenAction::Revoke { id } => revoke(registry, id).await,
    }
}

async fn create(
    registry: Option<&str>,
    name: &str,
    scopes: &[String],
    expires_in: &str,
    package_scope: Option<&str>,
) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let resp = client
        .create_token(name, scopes, expires_in, package_scope)
        .await?;

    display::success(&format!("Created API token: {}", resp.name));
    println!();
    display::label_value("Token", &resp.token);
    display::warn("This token will not be shown again. Copy it now!");
    println!();
    display::label_value("ID", &resp.id);
    display::label_value("Scopes", &resp.scopes.join(", "));
    if let Some(scope) = &resp.package_scope {
        display::label_value("Package scope", scope);
    }
    display::label_value("Expires at", &resp.expires_at);
    display::label_value("Created at", &resp.created_at);

    Ok(())
}

async fn list(registry: Option<&str>, json: bool) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let resp = client.list_tokens().await?;

    if json {
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "tokens": resp.tokens.iter().map(|t| serde_json::json!({
                "id": t.id,
                "name": t.name,
                "scopes": t.scopes,
                "expiresIn": t.expires_in,
                "packageScope": t.package_scope,
                "lastUsed": t.last_used,
                "expiresAt": t.expires_at,
                "createdAt": t.created_at,
            })).collect::<Vec<_>>()
        }))
        .map_err(|e| AppError::Other(format!("Failed to serialize JSON: {e}")))?;
        println!("{output}");
        return Ok(());
    }

    if resp.tokens.is_empty() {
        display::info("No API tokens found. Create one with: apkg token create --name <name> --scopes <scopes>");
        return Ok(());
    }

    println!(
        "{:<12}  {:<20}  {:<20}  {:<10}  LAST USED",
        "ID", "NAME", "SCOPES", "EXPIRES"
    );
    println!("{}", "-".repeat(80));
    for token in &resp.tokens {
        let short_id = if token.id.len() > 8 {
            &token.id[..8]
        } else {
            &token.id
        };
        let scopes = token.scopes.join(",");
        let last_used = token.last_used.as_deref().unwrap_or("never");
        println!(
            "{:<12}  {:<20}  {:<20}  {:<10}  {}",
            short_id, token.name, scopes, token.expires_in, last_used
        );
    }
    display::info(&format!("\n{} token(s)", resp.tokens.len()));

    Ok(())
}

async fn revoke(registry: Option<&str>, id: &str) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    client.revoke_token(id).await?;

    display::success(&format!("Revoked API token: {id}"));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_action_create_construction() {
        let scopes = vec!["read".to_string(), "publish".to_string()];
        let action = TokenAction::Create {
            name: "ci-token",
            scopes: &scopes,
            expires_in: "90d",
            package_scope: Some("@myorg/*"),
        };
        match action {
            TokenAction::Create {
                name,
                scopes,
                expires_in,
                package_scope,
            } => {
                assert_eq!(name, "ci-token");
                assert_eq!(scopes.len(), 2);
                assert_eq!(expires_in, "90d");
                assert_eq!(package_scope, Some("@myorg/*"));
            }
            _ => panic!("expected Create variant"),
        }
    }

    #[test]
    fn test_token_action_list_construction() {
        let action = TokenAction::List { json: true };
        match action {
            TokenAction::List { json } => assert!(json),
            _ => panic!("expected List variant"),
        }
    }

    #[test]
    fn test_token_action_revoke_construction() {
        let action = TokenAction::Revoke {
            id: "550e8400-e29b-41d4-a716-446655440000",
        };
        match action {
            TokenAction::Revoke { id } => {
                assert_eq!(id, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("expected Revoke variant"),
        }
    }
}
