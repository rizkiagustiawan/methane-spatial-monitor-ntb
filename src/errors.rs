use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("HTTP client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Sensor error: {0}")]
    Sensor(String),

    #[error("Physics error: {0}")]
    Physics(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("External service error: {0}")]
    ExternalService(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Http(_) => StatusCode::BAD_GATEWAY,
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Sensor(_) => StatusCode::BAD_REQUEST,
            AppError::Physics(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::ExternalService(_) => StatusCode::BAD_GATEWAY,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            AppError::Database(_) => "database_error",
            AppError::Http(_) => "http_error",
            AppError::Config(_) => "config_error",
            AppError::Sensor(_) => "sensor_error",
            AppError::Physics(_) => "physics_error",
            AppError::NotFound(_) => "not_found",
            AppError::Validation(_) => "validation_error",
            AppError::ExternalService(_) => "external_service_error",
            AppError::Internal(_) => "internal_error",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let error_type = self.error_type();
        let message = self.to_string();

        tracing::error!("{}: {}", error_type, message);

        let body = json!({
            "error": {
                "type": error_type,
                "message": message,
            }
        });

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        assert_eq!(AppError::NotFound("test".to_string()).status_code(), StatusCode::NOT_FOUND);
        assert_eq!(AppError::Validation("test".to_string()).status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(AppError::Sensor("test".to_string()).status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(AppError::Physics("test".to_string()).status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(AppError::Config("test".to_string()).status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(AppError::Internal("test".to_string()).status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(AppError::ExternalService("test".to_string()).status_code(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_error_types() {
        assert_eq!(AppError::NotFound("test".to_string()).error_type(), "not_found");
        assert_eq!(AppError::Validation("test".to_string()).error_type(), "validation_error");
        assert_eq!(AppError::Sensor("test".to_string()).error_type(), "sensor_error");
        assert_eq!(AppError::Physics("test".to_string()).error_type(), "physics_error");
        assert_eq!(AppError::Config("test".to_string()).error_type(), "config_error");
        assert_eq!(AppError::Internal("test".to_string()).error_type(), "internal_error");
        assert_eq!(AppError::ExternalService("test".to_string()).error_type(), "external_service_error");
    }

    #[test]
    fn test_error_messages() {
        let err = AppError::NotFound("plume not found".to_string());
        assert_eq!(err.to_string(), "Not found: plume not found");

        let err = AppError::Validation("invalid coordinates".to_string());
        assert_eq!(err.to_string(), "Validation error: invalid coordinates");

        let err = AppError::Sensor("below detection limit".to_string());
        assert_eq!(err.to_string(), "Sensor error: below detection limit");
    }

    #[test]
    fn test_error_from_sqlx() {
        // Test that sqlx::Error converts to AppError::Database
        let sqlx_err = sqlx::Error::RowNotFound;
        let app_err: AppError = sqlx_err.into();
        assert_eq!(app_err.error_type(), "database_error");
    }
}
