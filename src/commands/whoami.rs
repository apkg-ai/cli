use crate::api::client::ApiClient;
use crate::api::types::WhoamiResponse;
use crate::error::AppError;
use crate::util::display;

fn format_response(resp: &WhoamiResponse) -> Vec<(&str, String)> {
    let mut pairs = vec![
        ("Username", resp.username.clone()),
        ("Email", resp.email.clone()),
    ];
    if let Some(mfa) = resp.mfa_enabled {
        pairs.push(("MFA", if mfa { "enabled" } else { "disabled" }.to_string()));
    }
    if !resp.scopes.is_empty() {
        pairs.push(("Scopes", resp.scopes.join(", ")));
    }
    if !resp.orgs.is_empty() {
        pairs.push((
            "Organizations",
            resp.orgs
                .iter()
                .map(|o| format!("{} ({})", o.name, o.role))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    pairs
}

pub async fn run(registry: Option<&str>) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let resp = client.whoami().await?;

    for (label, value) in format_response(&resp) {
        display::label_value(label, &value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::OrgMembership;

    fn make_response(
        mfa_enabled: Option<bool>,
        scopes: Vec<&str>,
        orgs: Vec<(&str, &str)>,
    ) -> WhoamiResponse {
        WhoamiResponse {
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            mfa_enabled,
            scopes: scopes.into_iter().map(String::from).collect(),
            orgs: orgs
                .into_iter()
                .map(|(name, role)| OrgMembership {
                    name: name.to_string(),
                    role: role.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn test_format_response_basic() {
        let resp = make_response(None, vec![], vec![]);
        let pairs = format_response(&resp);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("Username", "alice".to_string()));
        assert_eq!(pairs[1], ("Email", "alice@example.com".to_string()));
    }

    #[test]
    fn test_format_response_mfa_enabled() {
        let resp = make_response(Some(true), vec![], vec![]);
        let pairs = format_response(&resp);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[2], ("MFA", "enabled".to_string()));
    }

    #[test]
    fn test_format_response_mfa_disabled() {
        let resp = make_response(Some(false), vec![], vec![]);
        let pairs = format_response(&resp);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[2], ("MFA", "disabled".to_string()));
    }

    #[test]
    fn test_format_response_scopes() {
        let resp = make_response(None, vec!["read", "publish"], vec![]);
        let pairs = format_response(&resp);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[2], ("Scopes", "read, publish".to_string()));
    }

    #[test]
    fn test_format_response_orgs() {
        let resp = make_response(None, vec![], vec![("acme", "admin"), ("tools", "member")]);
        let pairs = format_response(&resp);
        assert_eq!(pairs.len(), 3);
        assert_eq!(
            pairs[2],
            ("Organizations", "acme (admin), tools (member)".to_string())
        );
    }

    #[test]
    fn test_format_response_all_fields() {
        let resp = make_response(Some(true), vec!["read"], vec![("acme", "owner")]);
        let pairs = format_response(&resp);
        assert_eq!(pairs.len(), 5);
        assert_eq!(pairs[0].0, "Username");
        assert_eq!(pairs[1].0, "Email");
        assert_eq!(pairs[2], ("MFA", "enabled".to_string()));
        assert_eq!(pairs[3], ("Scopes", "read".to_string()));
        assert_eq!(pairs[4], ("Organizations", "acme (owner)".to_string()));
    }
}
