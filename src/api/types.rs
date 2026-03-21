use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub mfa_required: Option<bool>,
    #[serde(default)]
    pub mfa_token: Option<String>,
}

impl LoginResponse {
    pub fn requires_mfa(&self) -> bool {
        self.mfa_required.unwrap_or(false)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MfaChallengeResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub token_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhoamiResponse {
    pub username: String,
    pub email: String,
    #[serde(default)]
    pub mfa_enabled: Option<bool>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub orgs: Vec<OrgMembership>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgMembership {
    pub name: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageMetadata {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub dist_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub versions: BTreeMap<String, VersionMetadata>,
    #[serde(default)]
    pub maintainers: Vec<Maintainer>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub readme: Option<String>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionMetadata {
    pub version: String,
    #[serde(rename = "type")]
    #[serde(default)]
    pub package_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub dist: Option<DistInfo>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub yanked: Option<bool>,
    #[serde(default)]
    pub dependencies: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistInfo {
    pub tarball: String,
    pub integrity: String,
    #[serde(default)]
    pub signatures: Vec<Signature>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Maintainer {
    pub username: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResponse {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub integrity: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(rename = "type")]
    #[serde(default)]
    pub package_type: Option<String>,
    #[serde(default)]
    pub publisher: Option<SearchPublisher>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPublisher {
    pub username: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterKeyResponse {
    pub key_id: String,
    pub name: String,
    pub algorithm: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListKeysResponse {
    pub keys: Vec<KeyInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    pub key_id: String,
    pub name: String,
    pub algorithm: String,
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateKeyResponse {
    pub new_key_id: String,
    pub old_key_id: String,
    pub rotated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeKeyResponse {
    pub key_id: String,
    pub status: String,
    pub revoked_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTokenResponse {
    pub id: String,
    pub name: String,
    pub token: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub package_scope: Option<String>,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTokensResponse {
    pub tokens: Vec<TokenInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in: String,
    #[serde(default)]
    pub package_scope: Option<String>,
    #[serde(default)]
    pub last_used: Option<String>,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistTagResult {
    pub tag: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySigningKeyCollection {
    pub keys: Vec<RegistrySigningKey>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySigningKey {
    pub key_id: String,
    pub public_key: String,
    pub algorithm: String,
    pub status: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceAttestation {
    #[serde(default)]
    pub predicate: Option<serde_json::Value>,
}
