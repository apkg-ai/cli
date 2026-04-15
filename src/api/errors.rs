use std::fmt::Write;

use serde::Deserialize;

use crate::error::AppError;

#[derive(Debug, Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiErrorBody {
    code: String,
    message: String,
    #[serde(default)]
    details: Vec<ApiErrorDetail>,
    #[serde(default)]
    #[allow(dead_code)]
    request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    #[serde(default)]
    field: Option<String>,
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    code: Option<String>,
}

pub async fn parse_error_response(response: reqwest::Response) -> AppError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    format_api_error(status, &body)
}

fn format_api_error(status: u16, body: &str) -> AppError {
    match serde_json::from_str::<ApiErrorEnvelope>(body) {
        Ok(envelope) => {
            let mut message = envelope.error.message;
            if !envelope.error.details.is_empty() {
                for detail in &envelope.error.details {
                    if let Some(field) = &detail.field {
                        let _ = write!(message, "\n  - {field}: {}", detail.message);
                    } else {
                        let _ = write!(message, "\n  - {}", detail.message);
                    }
                }
            }
            if status == 401 {
                return AppError::AuthFailed(message);
            }
            AppError::Api {
                code: envelope.error.code,
                message,
                status,
            }
        }
        Err(_) => AppError::Api {
            code: format!("HTTP_{status}"),
            message: format!("Server returned status {status}"),
            status,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_api_error_with_message() {
        let body = r#"{"error":{"code":"NOT_FOUND","message":"Package not found","details":[]}}"#;
        let err = format_api_error(404, body);
        assert!(err.to_string().contains("Package not found"));
    }

    #[test]
    fn test_format_api_error_with_details() {
        let body = r#"{"error":{"code":"VALIDATION","message":"Validation failed","details":[{"field":"name","message":"required"},{"message":"invalid format"}]}}"#;
        let err = format_api_error(400, body);
        let msg = err.to_string();
        assert!(msg.contains("Validation failed"));
        assert!(msg.contains("name: required"));
        assert!(msg.contains("invalid format"));
    }

    #[test]
    fn test_format_api_error_401() {
        let body = r#"{"error":{"code":"UNAUTHORIZED","message":"Invalid credentials","details":[]}}"#;
        let err = format_api_error(401, body);
        match err {
            AppError::AuthFailed(msg) => assert!(msg.contains("Invalid credentials")),
            _ => panic!("expected AuthFailed"),
        }
    }

    #[test]
    fn test_format_api_error_non_json() {
        let err = format_api_error(500, "Internal Server Error");
        match err {
            AppError::Api { code, status, .. } => {
                assert_eq!(code, "HTTP_500");
                assert_eq!(status, 500);
            }
            _ => panic!("expected Api error"),
        }
    }

    #[test]
    fn test_format_api_error_empty_body() {
        let err = format_api_error(502, "");
        match err {
            AppError::Api { code, status, .. } => {
                assert_eq!(code, "HTTP_502");
                assert_eq!(status, 502);
            }
            _ => panic!("expected Api error"),
        }
    }

    #[test]
    fn test_format_api_error_401_with_details() {
        let body = r#"{"error":{"code":"UNAUTHORIZED","message":"Auth failed","details":[{"field":"token","message":"expired"}]}}"#;
        let err = format_api_error(401, body);
        match err {
            AppError::AuthFailed(msg) => {
                assert!(msg.contains("Auth failed"));
                assert!(msg.contains("token: expired"));
            }
            _ => panic!("expected AuthFailed"),
        }
    }

    #[test]
    fn test_format_api_error_non_401_code() {
        let body = r#"{"error":{"code":"FORBIDDEN","message":"Access denied","details":[]}}"#;
        let err = format_api_error(403, body);
        match err {
            AppError::Api {
                code,
                message,
                status,
            } => {
                assert_eq!(code, "FORBIDDEN");
                assert_eq!(status, 403);
                assert!(message.contains("Access denied"));
            }
            _ => panic!("expected Api error"),
        }
    }

    #[test]
    fn test_format_api_error_with_request_id() {
        let body = r#"{"error":{"code":"INTERNAL","message":"Something broke","requestId":"req-abc-123","details":[]}}"#;
        let err = format_api_error(500, body);
        match err {
            AppError::Api { code, message, .. } => {
                assert_eq!(code, "INTERNAL");
                assert!(message.contains("Something broke"));
            }
            _ => panic!("expected Api error"),
        }
    }

    #[test]
    fn test_format_api_error_detail_without_field() {
        let body = r#"{"error":{"code":"VALIDATION","message":"Bad input","details":[{"message":"must be non-empty"}]}}"#;
        let err = format_api_error(400, body);
        let msg = err.to_string();
        assert!(msg.contains("Bad input"));
        assert!(msg.contains("  - must be non-empty"));
    }

    #[test]
    fn test_format_api_error_detail_with_code() {
        let body = r#"{"error":{"code":"VALIDATION","message":"Invalid","details":[{"field":"version","message":"bad semver","code":"INVALID_FORMAT"}]}}"#;
        let err = format_api_error(400, body);
        let msg = err.to_string();
        assert!(msg.contains("version: bad semver"));
    }
}
