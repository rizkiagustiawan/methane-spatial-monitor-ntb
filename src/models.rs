use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

// ─── Methane Observation ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct MethaneObservation {
    pub id: Uuid,
    pub recorded_at: DateTime<Utc>,
    pub emission_rate_kg_hr: f64,
    pub location_json: String, // From ST_AsGeoJSON
    #[serde(rename = "green_area_ha")]
    pub total_green_area_hectares: f64,
}

// ─── Weather Observation ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WeatherObservation {
    pub recorded_at: DateTime<Utc>,
    pub area_id: String,
    pub wind_speed_ms: Option<f64>,
    pub wind_direction_deg: Option<f64>,
    pub humidity_percent: Option<f64>,
    pub temperature_c: Option<f64>,
    pub data_source: String,
}

// ─── Plume Prediction ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PlumePrediction {
    pub emission_rate_kg_hr: f64,
    pub wind_speed_ms: f64,
    pub wind_direction_deg: f64,
    pub plume_line_json: String,
    pub high_uncertainty_smear: bool,
    pub exposure_alert: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct MultiPlumePrediction {
    pub source_id: Uuid,
    pub source_lon: f64,
    pub source_lat: f64,
    pub emission_rate_kg_hr: f64,
    pub wind_speed_ms: f64,
    pub wind_direction_deg: f64,
    pub stability_class: char,
    pub spread_angle_deg: f64,
    pub max_distance_m: f64,
    pub concentration_at_1km_ppm: f64,
    pub plume_geojson: serde_json::Value,
    pub high_uncertainty_smear: bool,
    pub terrain_blocked: bool,
    pub terrain_block_distance_m: Option<f64>,
    pub affected_zones: Vec<AffectedZone>,
    pub exposure_alert: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct AffectedZone {
    pub zone_name: String,
    pub region: String,
    pub population_estimate: Option<i32>,
    pub zone_type: String,
    pub is_volcanic_zone: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MethanePlumeResponse {
    pub recorded_at: DateTime<Utc>,
    pub emission_rate_kg_hr: f64,
    pub geometry: serde_json::Value,
}

// ─── Health / Stats ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub database: ComponentHealth,
    pub dem_file: ComponentHealth,
    pub last_bmkg_fetch: Option<DateTime<Utc>>,
    pub last_carbon_mapper_fetch: Option<DateTime<Utc>>,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct ComponentHealth {
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SystemStats {
    pub total_plumes: i64,
    pub plumes_last_24h: i64,
    pub plumes_last_7d: i64,
    pub avg_emission_rate: f64,
    pub max_emission_rate: f64,
    pub total_weather_records: i64,
    pub weather_records_last_24h: i64,
    pub total_alerts: i64,
    pub alerts_last_24h: i64,
    pub active_zones: i64,
    pub latest_plume_at: Option<DateTime<Utc>>,
    pub latest_weather_at: Option<DateTime<Utc>>,
}

// ─── Metrics ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct AppMetrics {
    pub requests_total: std::sync::atomic::AtomicU64,
    pub request_errors: std::sync::atomic::AtomicU64,
    pub carbon_mapper_fetches: std::sync::atomic::AtomicU64,
    pub carbon_mapper_errors: std::sync::atomic::AtomicU64,
    pub bmkg_fetches: std::sync::atomic::AtomicU64,
    pub bmkg_errors: std::sync::atomic::AtomicU64,
    pub alerts_sent: std::sync::atomic::AtomicU64,
    pub plumes_ingested: std::sync::atomic::AtomicU64,
}

// ─── BMKG JSON Models ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BmkgResponse {
    pub data: Vec<BmkgDataGroup>,
}

#[derive(Debug, Deserialize)]
pub struct BmkgDataGroup {
    pub cuaca: Vec<Vec<BmkgForecastItem>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BmkgForecastItem {
    #[allow(dead_code)]
    pub local_datetime: String,
    pub t: f64,
    pub hu: f64,
    pub ws: f64,
    #[allow(dead_code)]
    pub wd: String,
    pub wd_deg: f64,
}

// ─── Open-Meteo Models ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OpenMeteoResponse {
    pub current: CurrentWeather,
}

#[derive(Debug, Deserialize)]
pub struct CurrentWeather {
    pub temperature_2m: f64,
    pub relative_humidity_2m: f64,
    pub wind_speed_10m: f64,
    pub wind_direction_10m: f64,
}

// ─── STAC Models (Carbon Mapper) ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StacResponse {
    pub features: Vec<StacFeature>,
    #[serde(default)]
    pub links: Vec<StacLink>,
}

#[derive(Debug, Deserialize)]
pub struct StacFeature {
    pub geometry: serde_json::Value,
    pub properties: StacProperties,
}

#[derive(Debug, Deserialize)]
pub struct StacProperties {
    pub datetime: String,
    #[serde(default)]
    pub emission_rate_kg_hr: f64,
}

#[derive(Debug, Deserialize)]
pub struct StacLink {
    pub rel: String,
    pub href: String,
}
