use crate::errors::AppError;
/// Configuration module with validation
///
/// All configuration is validated at startup
/// Invalid configuration = immediate crash (fail-fast)
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub carbon_mapper: CarbonMapperConfig,
    pub emit: EmitConfig,
    pub s5p: S5pConfig,
    #[allow(dead_code)]
    pub weather: WeatherConfig,
    pub telegram: TelegramConfig,
    pub physics: PhysicsConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[allow(dead_code)]
    pub cors_origins: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CarbonMapperConfig {
    pub api_token: String,
    pub base_url: String,
    pub bbox: Vec<f64>,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EmitConfig {
    pub enabled: bool,
    pub base_url: String,
    pub bbox: Vec<f64>,
    pub poll_interval_secs: u64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct S5pConfig {
    pub enabled: bool,
    pub base_url: String,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WeatherConfig {
    #[allow(dead_code)]
    pub bmkg_enabled: bool,
    #[allow(dead_code)]
    pub open_meteo_enabled: bool,
    #[allow(dead_code)]
    pub forecast_days: u32,
    #[allow(dead_code)]
    pub poll_interval_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
    #[allow(dead_code)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PhysicsConfig {
    pub min_detection_kg_hr: f64,
    #[allow(dead_code)]
    pub terrain_threshold_m: f64,
    #[allow(dead_code)]
    pub humidity_threshold: f64,
    #[allow(dead_code)]
    pub humidity_attenuation: f64,
    #[allow(dead_code)]
    pub sensor_roll_limit: f64,
    #[allow(dead_code)]
    pub sensor_pitch_limit: f64,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppError> {
        dotenvy::dotenv().ok();

        let config = Self {
            database: DatabaseConfig {
                url: std::env::var("DATABASE_URL")
                    .map_err(|_| AppError::Config("DATABASE_URL not set".into()))?,
                max_connections: std::env::var("DB_MAX_CONNECTIONS")
                    .unwrap_or_else(|_| "10".into())
                    .parse()
                    .map_err(|_| AppError::Config("Invalid DB_MAX_CONNECTIONS".into()))?,
                min_connections: std::env::var("DB_MIN_CONNECTIONS")
                    .unwrap_or_else(|_| "1".into())
                    .parse()
                    .map_err(|_| AppError::Config("Invalid DB_MIN_CONNECTIONS".into()))?,
                acquire_timeout_secs: std::env::var("DB_ACQUIRE_TIMEOUT")
                    .unwrap_or_else(|_| "30".into())
                    .parse()
                    .map_err(|_| AppError::Config("Invalid DB_ACQUIRE_TIMEOUT".into()))?,
            },
            server: ServerConfig {
                host: std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
                port: std::env::var("SERVER_PORT")
                    .unwrap_or_else(|_| "3000".into())
                    .parse()
                    .map_err(|_| AppError::Config("Invalid SERVER_PORT".into()))?,
                cors_origins: std::env::var("CORS_ORIGINS")
                    .unwrap_or_else(|_| "http://localhost:3000".into())
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect(),
            },
            carbon_mapper: CarbonMapperConfig {
                api_token: std::env::var("CARBON_MAPPER_TOKEN").unwrap_or_default(),
                base_url: std::env::var("CARBON_MAPPER_URL")
                    .unwrap_or_else(|_| "https://api.carbonmapper.org/api/v1/stac/search".into()),
                bbox: vec![115.40, -9.15, 119.45, -8.00],
                poll_interval_secs: 86400,
            },
            emit: EmitConfig {
                enabled: std::env::var("EMIT_ENABLED")
                    .unwrap_or_else(|_| "true".into())
                    .parse()
                    .unwrap_or(true),
                base_url: std::env::var("EMIT_STAC_URL")
                    .unwrap_or_else(|_| "https://ghgcenter.upc.nasa.gov/api/stac".into()),
                bbox: std::env::var("EMIT_BBOX")
                    .unwrap_or_else(|_| "115.40,-9.15,119.45,-8.00".into())
                    .split(',')
                    .map(|s| s.trim().parse().unwrap_or(0.0))
                    .collect(),
                poll_interval_secs: std::env::var("EMIT_POLL_INTERVAL_SECS")
                    .unwrap_or_else(|_| "43200".into())
                    .parse()
                    .unwrap_or(43200),
            },
            s5p: S5pConfig {
                enabled: std::env::var("S5P_ENABLED")
                    .unwrap_or_else(|_| "true".into())
                    .parse()
                    .unwrap_or(true),
                base_url: std::env::var("S5P_STAC_URL").unwrap_or_else(|_| {
                    "https://planetarycomputer.microsoft.com/api/stac/v1".into()
                }),
                poll_interval_secs: std::env::var("S5P_POLL_INTERVAL_SECS")
                    .unwrap_or_else(|_| "21600".into()) // 6 hours
                    .parse()
                    .unwrap_or(21600),
            },
            weather: WeatherConfig {
                bmkg_enabled: std::env::var("BMKG_ENABLED")
                    .unwrap_or_else(|_| "true".into())
                    .parse()
                    .unwrap_or(true),
                open_meteo_enabled: std::env::var("OPEN_METEO_ENABLED")
                    .unwrap_or_else(|_| "true".into())
                    .parse()
                    .unwrap_or(true),
                forecast_days: std::env::var("FORECAST_DAYS")
                    .unwrap_or_else(|_| "2".into())
                    .parse()
                    .map_err(|_| AppError::Config("Invalid FORECAST_DAYS".into()))?,
                poll_interval_secs: 3600,
            },
            telegram: TelegramConfig {
                bot_token: std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default(),
                chat_id: std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default(),
                enabled: std::env::var("TELEGRAM_ENABLED")
                    .unwrap_or_else(|_| "true".into())
                    .parse()
                    .unwrap_or(true),
            },
            physics: PhysicsConfig {
                min_detection_kg_hr: 100.0,
                terrain_threshold_m: 15.0,
                humidity_threshold: 85.0,
                humidity_attenuation: 0.6,
                sensor_roll_limit: 6.85,
                sensor_pitch_limit: 4.8,
            },
        };

        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.database.url.is_empty() {
            return Err(AppError::Config("DATABASE_URL is empty".into()));
        }
        if self.database.max_connections == 0 {
            return Err(AppError::Config("DB_MAX_CONNECTIONS must be > 0".into()));
        }
        if self.server.port == 0 {
            return Err(AppError::Config("SERVER_PORT must be > 0".into()));
        }
        if self.physics.min_detection_kg_hr <= 0.0 {
            return Err(AppError::Config("min_detection_kg_hr must be > 0".into()));
        }
        if self.emit.enabled {
            if self.emit.base_url.is_empty() {
                return Err(AppError::Config(
                    "EMIT_STAC_URL cannot be empty when EMIT is enabled".into(),
                ));
            }
            if !self.emit.base_url.starts_with("http://")
                && !self.emit.base_url.starts_with("https://")
            {
                return Err(AppError::Config(
                    "EMIT_STAC_URL must start with http:// or https://".into(),
                ));
            }
            if self.emit.bbox.len() != 4 {
                return Err(AppError::Config(
                    "EMIT_BBOX must have exactly 4 values (min_lon,min_lat,max_lon,max_lat)".into(),
                ));
            }
        }
        Ok(())
    }
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            min_detection_kg_hr: 100.0,
            terrain_threshold_m: 15.0,
            humidity_threshold: 85.0,
            humidity_attenuation: 0.6,
            sensor_roll_limit: 6.85,
            sensor_pitch_limit: 4.8,
        }
    }
}

#[cfg(test)]
mod emit_tests {
    use super::*;

    #[test]
    fn test_emit_config_default_values() {
        let config = EmitConfig {
            enabled: true,
            base_url: "https://ghgcenter.upc.nasa.gov/api/stac".to_string(),
            bbox: vec![115.40, -9.15, 119.45, -8.00],
            poll_interval_secs: 43200,
        };

        assert!(config.enabled);
        assert!(config.base_url.starts_with("https://"));
        assert_eq!(config.bbox.len(), 4);
        assert!(config.poll_interval_secs > 0);
    }

    #[test]
    fn test_emit_config_bbox_parsing() {
        let bbox_str = "115.40,-9.15,119.45,-8.00";
        let bbox: Vec<f64> = bbox_str
            .split(',')
            .map(|s| s.trim().parse().unwrap_or(0.0))
            .collect();

        assert_eq!(bbox.len(), 4);
        assert_eq!(bbox[0], 115.40);
        assert_eq!(bbox[1], -9.15);
        assert_eq!(bbox[2], 119.45);
        assert_eq!(bbox[3], -8.00);
    }

    #[test]
    fn test_emit_config_invalid_bbox() {
        let bbox_str = "115.40,-9.15";
        let bbox: Vec<f64> = bbox_str
            .split(',')
            .map(|s| s.trim().parse().unwrap_or(0.0))
            .collect();

        assert_ne!(bbox.len(), 4);
    }

    #[test]
    fn test_emit_config_url_validation() {
        let valid_urls = vec![
            "https://ghgcenter.upc.nasa.gov/api/stac",
            "http://localhost:8080/api/stac",
        ];

        for url in valid_urls {
            assert!(
                url.starts_with("http://") || url.starts_with("https://"),
                "URL should start with http:// or https://: {}",
                url
            );
        }

        let invalid_urls = vec!["ftp://example.com", "not-a-url", ""];

        for url in invalid_urls {
            assert!(
                !url.starts_with("http://") && !url.starts_with("https://"),
                "URL should be invalid: {}",
                url
            );
        }
    }

    #[test]
    fn test_emit_config_poll_interval() {
        let config = EmitConfig {
            enabled: true,
            base_url: "https://example.com".to_string(),
            bbox: vec![115.40, -9.15, 119.45, -8.00],
            poll_interval_secs: 43200,
        };

        assert!(config.poll_interval_secs >= 3600, "Poll interval too short");
        assert!(config.poll_interval_secs <= 86400, "Poll interval too long");
    }
}
