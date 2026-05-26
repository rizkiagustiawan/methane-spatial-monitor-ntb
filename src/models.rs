use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct MethaneObservation {
    pub id: Uuid,
    pub recorded_at: DateTime<Utc>,
    pub emission_rate_kg_hr: f64,
    pub location_json: String, // From ST_AsGeoJSON
    #[serde(rename = "green_area_ha")]
    pub total_green_area_hectares: f64,
}

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

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PlumePrediction {
    pub emission_rate_kg_hr: f64,
    pub wind_speed_ms: f64,
    pub wind_direction_deg: f64,
    pub plume_line_json: String, // ST_AsGeoJSON representation of the LineString
    pub high_uncertainty_smear: bool,
    pub exposure_alert: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MethanePlumeResponse {
    pub recorded_at: DateTime<Utc>,
    pub emission_rate_kg_hr: f64,
    pub geometry: serde_json::Value,
}

// BMKG JSON Models
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

// Open-Meteo Models
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

// STAC Models (Carbon Mapper)
#[derive(Debug, Deserialize)]
pub struct StacResponse {
    pub features: Vec<StacFeature>,
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
