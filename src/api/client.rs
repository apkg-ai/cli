use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::multipart;

use crate::api::errors::parse_error_response;
use crate::api::types::{
    CreateTokenResponse, DistTagResult, ListKeysResponse, ListTokensResponse, LoginResponse,
    MfaChallengeResponse, PackageMetadata, ProvenanceAttestation, PublishResponse,
    RegisterKeyResponse, RegistrySigningKeyCollection, RevokeKeyResponse, RotateKeyResponse,
    SearchResponse, VersionMetadata, WhoamiResponse,
};
use crate::config::credentials;
use crate::config::settings::Settings;
use crate::error::AppError;

pub struct ApiClient {
    client: reqwest::Client,
    settings: Settings,
    registry_override: Option<String>,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(registry_override: Option<&str>) -> Result<Self, AppError> {
        let settings = Settings::load()?;
        let token = credentials::load()?.map(|c| c.access_token);

        let client = reqwest::Client::builder()
            .user_agent(format!("apkg-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            settings,
            registry_override: registry_override.map(String::from),
            token,
        })
    }

    fn url(&self, service: &str, path: &str) -> String {
        // Per-service config is the most specific — always wins
        let base = if let Some(svc_url) = self.settings.service_url(service) {
            svc_url.trim_end_matches('/').to_string()
        } else if let Some(ovr) = &self.registry_override {
            ovr.trim_end_matches('/').to_string()
        } else if let Ok(env_reg) = std::env::var("APKG_REGISTRY") {
            env_reg.trim_end_matches('/').to_string()
        } else {
            self.settings.base_url(service)
        };
        format!("{base}{path}")
    }

    fn auth_headers(&self) -> Result<HeaderMap, AppError> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &self.token {
            let val = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| AppError::Other(format!("Invalid auth header: {e}")))?;
            headers.insert(AUTHORIZATION, val);
        }
        Ok(headers)
    }

    fn require_auth_headers(&self) -> Result<HeaderMap, AppError> {
        if self.token.is_none() {
            return Err(AppError::AuthRequired);
        }
        self.auth_headers()
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<LoginResponse, AppError> {
        let url = self.url("auth", "/auth/login");
        let body = serde_json::json!({
            "username": username,
            "password": password,
        });
        let resp = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn mfa_challenge(
        &self,
        mfa_token: &str,
        code: &str,
    ) -> Result<MfaChallengeResponse, AppError> {
        let url = self.url("mfa", "/auth/mfa/challenge");
        let body = serde_json::json!({
            "mfaToken": mfa_token,
            "code": code,
        });
        let resp = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn whoami(&self) -> Result<WhoamiResponse, AppError> {
        let url = self.url("auth", "/auth/whoami");
        let headers = self.require_auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn get_package(&self, name: &str) -> Result<PackageMetadata, AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url("package", &format!("/packages/{encoded_name}"));
        let headers = self.auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn download_tarball(
        &self,
        name: &str,
        version: &str,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/{version}/tarball"),
        );
        let headers = self.auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            let integrity = resp
                .headers()
                .get("x-integrity")
                .and_then(|v| v.to_str().ok())
                .map(std::string::ToString::to_string);
            let bytes = resp.bytes().await?;
            Ok((bytes.to_vec(), integrity))
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(format!("{name}@{version}")))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn publish(
        &self,
        name: &str,
        metadata_json: &str,
        tarball_bytes: Vec<u8>,
    ) -> Result<PublishResponse, AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url("package", &format!("/packages/{encoded_name}"));
        let headers = self.require_auth_headers()?;

        let form = multipart::Form::new()
            .text("metadata", metadata_json.to_string())
            .part(
                "tarball",
                multipart::Part::bytes(tarball_bytes)
                    .file_name("package.tgz")
                    .mime_str("application/gzip")
                    .map_err(|e| AppError::Other(format!("Failed to build multipart: {e}")))?,
            );

        let resp = self
            .client
            .put(&url)
            .headers(headers)
            .multipart(form)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn register_key(
        &self,
        public_key: &str,
        name: &str,
    ) -> Result<RegisterKeyResponse, AppError> {
        let url = self.url("key", "/auth/keys");
        let headers = self.require_auth_headers()?;
        let body = serde_json::json!({
            "publicKey": public_key,
            "name": name,
        });
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn list_keys(&self) -> Result<ListKeysResponse, AppError> {
        let url = self.url("key", "/auth/keys");
        let headers = self.require_auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn revoke_key(
        &self,
        key_id: &str,
        reason: &str,
        message: Option<&str>,
    ) -> Result<RevokeKeyResponse, AppError> {
        let encoded_key_id = urlencoding::encode(key_id);
        let url = self.url("key", &format!("/auth/keys/{encoded_key_id}/revoke"));
        let headers = self.require_auth_headers()?;
        let mut body = serde_json::json!({
            "reason": reason,
        });
        if let Some(msg) = message {
            body["message"] = serde_json::Value::String(msg.to_string());
        }
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn rotate_key(
        &self,
        old_key_id: &str,
        new_public_key: &str,
        attestation: &str,
    ) -> Result<RotateKeyResponse, AppError> {
        let url = self.url("key", "/auth/keys/rotate");
        let headers = self.require_auth_headers()?;
        let body = serde_json::json!({
            "oldKeyId": old_key_id,
            "newPublicKey": new_public_key,
            "attestation": attestation,
        });
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn create_token(
        &self,
        name: &str,
        scopes: &[String],
        expires_in: &str,
        package_scope: Option<&str>,
    ) -> Result<CreateTokenResponse, AppError> {
        let url = self.url("token", "/auth/tokens");
        let headers = self.require_auth_headers()?;
        let mut body = serde_json::json!({
            "name": name,
            "scopes": scopes,
            "expiresIn": expires_in,
        });
        if let Some(scope) = package_scope {
            body["packageScope"] = serde_json::Value::String(scope.to_string());
        }
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn list_tokens(&self) -> Result<ListTokensResponse, AppError> {
        let url = self.url("token", "/auth/tokens");
        let headers = self.require_auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn revoke_token(&self, id: &str) -> Result<(), AppError> {
        let encoded_id = urlencoding::encode(id);
        let url = self.url("token", &format!("/auth/tokens/{encoded_id}"));
        let headers = self.require_auth_headers()?;
        let resp = self.client.delete(&url).headers(headers).send().await?;

        if resp.status().as_u16() == 204 || resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn search(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResponse, AppError> {
        let encoded_q = urlencoding::encode(query);
        let url = self.url(
            "search",
            &format!("/search?q={encoded_q}&limit={limit}&offset={offset}"),
        );
        let resp = self.client.get(&url).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn deprecate_package(
        &self,
        name: &str,
        message: Option<&str>,
    ) -> Result<PackageMetadata, AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url("package", &format!("/packages/{encoded_name}"));
        let headers = self.require_auth_headers()?;
        let deprecated = match message {
            Some(msg) => serde_json::Value::String(msg.to_string()),
            None => serde_json::Value::Null,
        };
        let body = serde_json::json!({ "deprecated": deprecated });
        let resp = self
            .client
            .patch(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn deprecate_version(
        &self,
        name: &str,
        version: &str,
        message: Option<&str>,
    ) -> Result<VersionMetadata, AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_version = urlencoding::encode(version);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/{encoded_version}"),
        );
        let headers = self.require_auth_headers()?;
        let deprecated = match message {
            Some(msg) => serde_json::Value::String(msg.to_string()),
            None => serde_json::Value::Null,
        };
        let body = serde_json::json!({ "deprecated": deprecated });
        let resp = self
            .client
            .patch(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(format!("{name}@{version}")))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn set_dist_tag(
        &self,
        name: &str,
        tag: &str,
        version: &str,
    ) -> Result<DistTagResult, AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_tag = urlencoding::encode(tag);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/dist-tags/{encoded_tag}"),
        );
        let headers = self.require_auth_headers()?;
        let body = serde_json::json!({ "version": version });
        let resp = self
            .client
            .put(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn get_registry_signing_keys(
        &self,
    ) -> Result<RegistrySigningKeyCollection, AppError> {
        let url = self.url("registry", "/registry/signing-keys");
        let resp = self.client.get(&url).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn get_provenance(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<ProvenanceAttestation>, AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_version = urlencoding::encode(version);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/{encoded_version}/provenance"),
        );
        let headers = self.auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            let body: ProvenanceAttestation = parse_json_response(resp).await?;
            Ok(Some(body))
        } else if resp.status().as_u16() == 404 {
            Ok(None)
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn remove_dist_tag(&self, name: &str, tag: &str) -> Result<(), AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_tag = urlencoding::encode(tag);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/dist-tags/{encoded_tag}"),
        );
        let headers = self.require_auth_headers()?;
        let resp = self.client.delete(&url).headers(headers).send().await?;

        if resp.status().as_u16() == 204 || resp.status().is_success() {
            Ok(())
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }
}

async fn parse_json_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, AppError> {
    let body = resp.text().await?;
    serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("Failed to parse response: {e}\nBody: {body}")))
}

fn encode_package_name(name: &str) -> String {
    urlencoding::encode(name).to_string()
}
