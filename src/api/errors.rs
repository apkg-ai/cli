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
    match response.json::<ApiErrorEnvelope>().await {
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
