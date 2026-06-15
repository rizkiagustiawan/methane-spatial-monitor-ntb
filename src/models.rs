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
    pub plume_geometry_json: Option<String>, // From ST_AsGeoJSON (observed footprint)
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MethanePlumeObserved {
    pub id: Uuid,
    pub recorded_at: DateTime<Utc>,
    pub emission_rate_kg_hr: f64,
    pub location: serde_json::Value,
    pub plume_footprint: Option<serde_json::Value>, // Actual observed plume geometry
    pub source: String,
    pub affected_zones: Vec<AffectedZone>,
    pub exposure_alert: bool,
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

// ─── Weather Forecast ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WeatherForecast {
    pub id: i32,
    pub forecast_at: DateTime<Utc>,
    pub valid_at: DateTime<Utc>,
    pub area_id: String,
    pub wind_speed_ms: Option<f64>,
    pub wind_direction_deg: Option<f64>,
    pub humidity_percent: Option<f64>,
    pub temperature_c: Option<f64>,
    pub data_source: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenMeteoForecastResponse {
    pub hourly: HourlyForecast,
}

#[derive(Debug, Deserialize)]
pub struct HourlyForecast {
    pub time: Vec<String>,
    pub temperature_2m: Vec<f64>,
    pub relative_humidity_2m: Vec<f64>,
    pub wind_speed_10m: Vec<f64>,
    pub wind_direction_10m: Vec<f64>,
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
pub struct PlumeAnalysis {
    pub source_id: Uuid,
    pub source_lon: f64,
    pub source_lat: f64,
    pub emission_rate_kg_hr: f64,
    pub recorded_at: DateTime<Utc>,
    pub observed: ObservedPlume,
    pub forecast: Vec<ForecastedPlume>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ObservedPlume {
    pub plume_footprint: Option<serde_json::Value>, // Actual satellite observation
    pub affected_zones: Vec<AffectedZone>,
    pub exposure_alert: bool,
    pub source: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ForecastedPlume {
    pub valid_at: DateTime<Utc>,
    pub wind_speed_ms: f64,
    pub wind_direction_deg: f64,
    pub stability_class: char,
    pub spread_angle_deg: f64,
    pub max_distance_m: f64,
    pub concentration_at_1km_ppm: f64,
    pub plume_geojson: serde_json::Value,
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
    pub plume_footprint: Option<serde_json::Value>,
    pub source: Option<String>,
}

// ─── Health / Stats ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub database: ComponentHealth,
    pub dem_file: ComponentHealth,
    pub last_bmkg_fetch: Option<DateTime<Utc>>,
    pub last_carbon_mapper_fetch: Option<DateTime<Utc>>,
    pub last_emit_fetch: Option<DateTime<Utc>>,
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
    pub emit_fetches: std::sync::atomic::AtomicU64,
    pub emit_errors: std::sync::atomic::AtomicU64,
    pub emit_plumes_ingested: std::sync::atomic::AtomicU64,
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

// ─── EMIT STAC Models (NASA GHG Center) ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EmitStacResponse {
    pub features: Vec<EmitStacFeature>,
    #[serde(default)]
    pub links: Vec<StacLink>,
}

#[derive(Debug, Deserialize)]
pub struct EmitStacFeature {
    pub geometry: serde_json::Value,
    pub properties: EmitStacProperties,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EmitStacProperties {
    pub datetime: String,
    #[serde(default)]
    pub ch4_plume_emission_rate: Option<f64>,
    #[serde(default)]
    pub ch4_plume_id: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub instrument: Option<String>,
}

// ─── Carbon Mapper API Models ────────────────────────────────────────────────
// Source: Carbon Mapper Product Guide v1.1.6 (Feb 14, 2025)
// https://carbonmapper.org/articles/product-guide

/// Carbon Mapper API response for plumes
#[derive(Debug, Deserialize)]
pub struct CarbonMapperPlumeResponse {
    pub bbox_count: u32,
    pub total_count: u32,
    pub limit: u32,
    pub offset: u32,
    pub items: Vec<CarbonMapperPlume>,
    #[serde(default)]
    pub nearby_items: Vec<serde_json::Value>,
}

/// Individual plume from Carbon Mapper API
/// Source: Product Guide - "Plume List Fields"
#[derive(Debug, Deserialize, Clone)]
pub struct CarbonMapperPlume {
    pub id: Uuid,
    pub plume_id: String,
    pub gas: String,  // CH4 or CO2
    pub geometry_json: serde_json::Value,
    pub scene_id: Option<String>,
    pub scene_timestamp: DateTime<Utc>,
    pub instrument: String,  // tan, emi, ang, av3, GAO
    pub platform: String,    // ISS, Tanager-1, etc.
    
    // Emission data
    pub emission_auto: f64,  // kg/hr
    pub emission_uncertainty_auto: Option<f64>,  // ± kg/hr
    
    // Wind data
    pub wind_speed_avg_auto: Option<f64>,  // m/s
    pub wind_speed_std_auto: Option<f64>,
    pub wind_direction_avg_auto: Option<f64>,  // degrees
    pub wind_direction_std_auto: Option<f64>,
    pub wind_source_auto: Option<String>,  // HRRR, ECMWF_IFS, ERA5
    
    // Plume geometry
    pub plume_bounds: Option<Vec<f64>>,  // [min_lon, min_lat, max_lon, max_lat]
    pub plume_length: Option<f64>,  // meters
    
    // Quality
    pub plume_quality: Option<String>,  // good, questionable, bad
    
    // Sector attribution
    pub sector: Option<String>,  // IPCC sector code (1B2, 6A, etc.)
    
    // URLs for data products
    pub plume_png: Option<String>,
    pub plume_rgb_png: Option<String>,
    pub plume_tif: Option<String>,
    pub con_tif: Option<String>,  // concentration map (ppm-m)
    pub rgb_png: Option<String>,
    
    // Metadata
    pub collection: Option<String>,
    pub cmf_type: Option<String>,  // mfa, mfm, mfma
    pub status: Option<String>,  // published, etc.
    pub hide_emission: Option<bool>,
    pub published_at: Option<DateTime<Utc>>,
    
    // IME (Integrated Mass Enhancement)
    pub ime: Option<f64>,  // kg of methane in plume
    pub ime_uncertainty: Option<f64>,
    
    // Additional fields
    pub emission_version: Option<String>,
    pub processing_software: Option<String>,
    pub gsd: Option<f64>,  // ground sampling distance
    pub sensitivity_mode: Option<String>,
    pub off_nadir: Option<f64>,  // degrees
    pub mission_phase: Option<String>,
    pub provider: Option<String>,
}

/// Sector attribution codes
/// Source: Product Guide - "Sector Attribution Codes"
#[derive(Debug, Deserialize, Clone, Serialize)]
pub enum IpccSector {
    #[serde(rename = "1A1")]
    ElectricityGeneration,
    #[serde(rename = "1B1a")]
    CoalMining,
    #[serde(rename = "1B2")]
    OilAndGas,
    #[serde(rename = "4A")]
    EntericFermentation,
    #[serde(rename = "4B")]
    ManureManagement,
    #[serde(rename = "6A")]
    SolidWaste,
    #[serde(rename = "6B")]
    WasteWater,
    #[serde(rename = "Other")]
    Other,
}

impl IpccSector {
    pub fn from_code(code: &str) -> Self {
        match code {
            "1A1" => Self::ElectricityGeneration,
            "1B1a" => Self::CoalMining,
            "1B2" => Self::OilAndGas,
            "4A" => Self::EntericFermentation,
            "4B" => Self::ManureManagement,
            "6A" => Self::SolidWaste,
            "6B" => Self::WasteWater,
            _ => Self::Other,
        }
    }
    
    pub fn to_name(&self) -> &'static str {
        match self {
            Self::ElectricityGeneration => "Electricity Generation",
            Self::CoalMining => "Coal Mining",
            Self::OilAndGas => "Oil & Natural Gas",
            Self::EntericFermentation => "Enteric Fermentation",
            Self::ManureManagement => "Manure Management",
            Self::SolidWaste => "Solid Waste",
            Self::WasteWater => "Waste Water",
            Self::Other => "Other",
        }
    }
}

/// PHME (Potentially Harmful Methane Event) criteria
/// Source: Product Guide - "L3A-PHME"
#[derive(Debug, Deserialize, Clone)]
pub struct PhmeCriteria {
    /// Proximity-only: plume origin is within 100 m of nearest sensitive receptor
    pub proximity_threshold_m: f64,  // 100m
    
    /// Size and proximity: plume length > 1000m AND overlaps sensitive receptor
    pub size_threshold_m: f64,  // 1000m
}

impl Default for PhmeCriteria {
    fn default() -> Self {
        Self {
            proximity_threshold_m: 100.0,
            size_threshold_m: 1000.0,
        }
    }
}

impl PhmeCriteria {
    /// Check if a plume qualifies as PHME
    pub fn is_phme(&self, plume_length_m: Option<f64>, distance_to_receptor_m: Option<f64>) -> bool {
        // Proximity-only: plume origin within 100m of sensitive receptor
        if let Some(dist) = distance_to_receptor_m {
            if dist <= self.proximity_threshold_m {
                return true;
            }
        }
        
        // Size and proximity: plume > 1000m AND overlaps receptor
        if let (Some(length), Some(dist)) = (plume_length_m, distance_to_receptor_m) {
            if length > self.size_threshold_m && dist <= self.size_threshold_m {
                return true;
            }
        }
        
        false
    }
}

#[cfg(test)]
mod emit_tests {
    use super::*;

    #[test]
    fn test_emit_stac_response_deserialization() {
        let json = r#"{
            "features": [
                {
                    "geometry": {"type": "Point", "coordinates": [116.5, -8.7]},
                    "properties": {
                        "datetime": "2024-01-15T10:30:00Z",
                        "ch4_plume_emission_rate": 150.5,
                        "ch4_plume_id": "emit-plume-001",
                        "platform": "ISS",
                        "instrument": "EMIT"
                    },
                    "id": "feature-001"
                }
            ],
            "links": [
                {"rel": "next", "href": "https://example.com/next"}
            ]
        }"#;

        let response: EmitStacResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.features.len(), 1);
        assert_eq!(response.links.len(), 1);
        
        let feature = &response.features[0];
        assert_eq!(feature.properties.datetime, "2024-01-15T10:30:00Z");
        assert_eq!(feature.properties.ch4_plume_emission_rate, Some(150.5));
        assert_eq!(feature.properties.ch4_plume_id, Some("emit-plume-001".to_string()));
        assert_eq!(feature.properties.platform, Some("ISS".to_string()));
        assert_eq!(feature.properties.instrument, Some("EMIT".to_string()));
        assert_eq!(feature.id, Some("feature-001".to_string()));
    }

    #[test]
    fn test_emit_stac_response_empty_features() {
        let json = r#"{"features": [], "links": []}"#;
        let response: EmitStacResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.features.len(), 0);
        assert_eq!(response.links.len(), 0);
    }

    #[test]
    fn test_emit_stac_response_missing_optional_fields() {
        let json = r#"{
            "features": [
                {
                    "geometry": {"type": "Point", "coordinates": [116.5, -8.7]},
                    "properties": {
                        "datetime": "2024-01-15T10:30:00Z"
                    }
                }
            ]
        }"#;

        let response: EmitStacResponse = serde_json::from_str(json).unwrap();
        let feature = &response.features[0];
        assert_eq!(feature.properties.ch4_plume_emission_rate, None);
        assert_eq!(feature.properties.ch4_plume_id, None);
        assert_eq!(feature.properties.platform, None);
        assert_eq!(feature.properties.instrument, None);
        assert_eq!(feature.id, None);
    }

    #[test]
    fn test_emit_stac_response_missing_links() {
        let json = r#"{
            "features": [
                {
                    "geometry": {"type": "Point", "coordinates": [116.5, -8.7]},
                    "properties": {"datetime": "2024-01-15T10:30:00Z"}
                }
            ]
        }"#;

        let response: EmitStacResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.links.len(), 0); // Should default to empty
    }

    #[test]
    fn test_emit_feature_geometry_extraction() {
        let json = r#"{
            "features": [
                {
                    "geometry": {"type": "Point", "coordinates": [116.5, -8.7]},
                    "properties": {"datetime": "2024-01-15T10:30:00Z"}
                }
            ]
        }"#;

        let response: EmitStacResponse = serde_json::from_str(json).unwrap();
        let feature = &response.features[0];
        
        // Extract coordinates like emit_tracker_task does
        let (lon, lat) = if let Some(coords) = feature.geometry.get("coordinates") {
            if let Some(arr) = coords.as_array() {
                if arr.len() >= 2 {
                    (arr[0].as_f64().unwrap_or(0.0), arr[1].as_f64().unwrap_or(0.0))
                } else { (0.0, 0.0) }
            } else { (0.0, 0.0) }
        } else { (0.0, 0.0) };
        
        assert_eq!(lon, 116.5);
        assert_eq!(lat, -8.7);
    }
}
