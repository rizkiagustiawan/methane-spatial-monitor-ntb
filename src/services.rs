/// Service layer for business logic
/// 
/// Separates business logic from HTTP handlers
/// Each service handles one domain

use std::sync::Arc;
use sqlx::PgPool;
use sqlx::Row;
use chrono::Timelike;
use crate::errors::AppError;
use crate::models::*;
use crate::repositories::*;
use crate::physics::*;

/// Get region from coordinates
/// Returns the nearest NTB region based on lat/lon
pub fn get_region_from_coords(lon: f64, lat: f64) -> &'static str {
    let zones = [
        ("Lombok Barat", -8.6818, 116.1240),
        ("Lombok Tengah", -8.7167, 116.2667),
        ("Lombok Timur", -8.6500, 116.5333),
        ("Lombok Utara", -8.3500, 116.4000),
        ("Kota Mataram", -8.5833, 116.1167),
        ("Sumbawa Barat", -8.7333, 116.8500),
        ("Sumbawa", -8.5000, 117.4167),
        ("Dompu", -8.5333, 118.4667),
        ("Bima", -8.6500, 118.6167),
        ("Kota Bima", -8.4667, 118.7167),
    ];

    let mut nearest_region = "Lombok Barat";
    let mut min_dist = f64::MAX;

    for (name, z_lat, z_lon) in zones {
        let dist = ((lat - z_lat).powi(2) + (lon - z_lon).powi(2)).sqrt();
        if dist < min_dist {
            min_dist = dist;
            nearest_region = name;
        }
    }
    nearest_region
}

/// Methane service
pub struct MethaneService {
    pool: PgPool,
}

impl MethaneService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    pub async fn get_plumes(&self, limit: i64) -> Result<Vec<MethanePlumeResponse>, AppError> {
        MethaneRepository::get_recent(&self.pool, limit).await
    }
    
    pub async fn get_active_sources(&self) -> Result<Vec<ActiveSource>, AppError> {
        MethaneRepository::get_active_sources(&self.pool).await
    }
}

/// Weather service
pub struct WeatherService {
    pool: PgPool,
}

impl WeatherService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    pub async fn get_latest(&self, limit: i64) -> Result<Vec<WeatherObservation>, AppError> {
        WeatherRepository::get_latest(&self.pool, limit).await
    }
    
    pub async fn get_latest_for_region(&self, region: &str) -> Result<Option<WeatherObservation>, AppError> {
        WeatherRepository::get_latest_for_region(&self.pool, region).await
    }
}

/// Forecast service
pub struct ForecastService {
    pool: PgPool,
}

impl ForecastService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    pub async fn get_upcoming(&self, limit: i64) -> Result<Vec<WeatherForecast>, AppError> {
        ForecastRepository::get_upcoming(&self.pool, limit).await
    }
    
    pub async fn get_for_region(&self, region: &str) -> Result<Vec<WeatherForecast>, AppError> {
        ForecastRepository::get_for_region(&self.pool, region).await
    }
}

/// Zones service
pub struct ZonesService {
    pool: PgPool,
}

impl ZonesService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    pub async fn get_all(&self) -> Result<Vec<serde_json::Value>, AppError> {
        ZonesRepository::get_all(&self.pool).await
    }
    
    pub async fn get_intersecting(&self, geojson: &str) -> Result<Vec<AffectedZone>, AppError> {
        ZonesRepository::get_intersecting(&self.pool, geojson).await
    }
}

/// Alert service
pub struct AlertService {
    pool: PgPool,
    http_client: reqwest::Client,
    telegram_token: String,
    telegram_chat_id: String,
}

impl AlertService {
    pub fn new(
        pool: PgPool,
        http_client: reqwest::Client,
        telegram_token: String,
        telegram_chat_id: String,
    ) -> Self {
        Self { pool, http_client, telegram_token, telegram_chat_id }
    }
    
    pub async fn send_alert(
        &self,
        zone: &AffectedZone,
        emission_rate: f64,
        distance: f64,
        concentration: f64,
        wind_speed: f64,
        wind_direction: f64,
        stability_class: &str,
    ) -> Result<(), AppError> {
        // Log to database
        AlertRepository::insert(
            &self.pool,
            &zone.region,
            &zone.zone_name,
            emission_rate,
            wind_speed,
            wind_direction,
            concentration,
            stability_class,
        ).await?;
        
        // Send Telegram notification
        if !self.telegram_token.is_empty() && !self.telegram_chat_id.is_empty() {
            let msg = format!(
                "⚠️ <b>EVACUATION ALERT</b>\n\n<b>Zone:</b> {} ({})\n<b>Emission:</b> {:.2} kg/hr\n<b>Max Dist:</b> {:.0}m\n<b>Est. Conc:</b> {:.1} ppm",
                zone.zone_name, zone.region, emission_rate, distance, concentration
            );
            
            self.send_telegram(&msg).await?;
        }
        
        Ok(())
    }
    
    async fn send_telegram(&self, msg: &str) -> Result<(), AppError> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.telegram_token);
        let payload = serde_json::json!({
            "chat_id": self.telegram_chat_id,
            "text": msg,
            "parse_mode": "HTML"
        });
        
        self.http_client.post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::ExternalService(format!("Telegram error: {}", e)))?;
        
        Ok(())
    }
}

/// Plume analysis service
pub struct PlumeAnalysisService {
    pool: PgPool,
    alert_service: Arc<AlertService>,
}

impl PlumeAnalysisService {
    pub fn new(pool: PgPool, alert_service: Arc<AlertService>) -> Self {
        Self { pool, alert_service }
    }
    
    pub async fn get_analysis(&self) -> Result<Vec<PlumeAnalysis>, AppError> {
        let sources = MethaneRepository::get_active_sources(&self.pool).await?;
        let mut analyses = Vec::new();
        
        let wita_offset = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        
        for source in sources {
            // Source: Carbon Mapper Product Guide - "90-180 kg/hr (90% Probability of Detection)"
            if source.emission_rate_kg_hr < tanager1::DETECTION_90PCT_KG_HR {
                continue;
            }
            
            let region = get_region_from_coords(source.lon, source.lat);
            
            // Get observed plume
            let observed = self.get_observed_plume(&source).await?;
            
            // Get forecast plumes
            let forecast = self.get_forecast_plumes(&source, region, &wita_offset).await?;
            
            analyses.push(PlumeAnalysis {
                source_id: source.id,
                source_lon: source.lon,
                source_lat: source.lat,
                emission_rate_kg_hr: source.emission_rate_kg_hr,
                recorded_at: chrono::Utc::now(),
                observed,
                forecast,
            });
        }
        
        Ok(analyses)
    }
    
    async fn get_observed_plume(&self, source: &ActiveSource) -> Result<ObservedPlume, AppError> {
        // Get plume geometry from database
        let plume_geometry: Option<serde_json::Value> = sqlx::query(
            r#"SELECT ST_AsGeoJSON(plume_geometry) as geom FROM methane_observations WHERE id = $1"#,
        )
        .bind(source.id)
        .fetch_optional(&self.pool)
        .await?
        .and_then(|r| {
            let geom: Option<String> = r.get("geom");
            geom.and_then(|g: String| serde_json::from_str(&g).ok())
        });
        
        // Check intersection with zones
        let affected_zones = if let Some(ref geom) = plume_geometry {
            let geom_str = serde_json::to_string(geom).unwrap_or_default();
            ZonesRepository::get_intersecting(&self.pool, &geom_str).await?
        } else {
            Vec::new()
        };
        
        Ok(ObservedPlume {
            plume_footprint: plume_geometry,
            exposure_alert: !affected_zones.is_empty(),
            affected_zones,
            source: "carbon_mapper".to_string(),
        })
    }
    
    async fn get_forecast_plumes(
        &self,
        source: &ActiveSource,
        region: &str,
        wita_offset: &chrono::FixedOffset,
    ) -> Result<Vec<ForecastedPlume>, AppError> {
        let forecasts = ForecastRepository::get_for_region(&self.pool, region).await?;
        let mut result = Vec::new();
        
        for fc in forecasts {
            let ws = fc.wind_speed_ms.unwrap_or(1.0);
            let wd = fc.wind_direction_deg.unwrap_or(0.0);
            let hum = fc.humidity_percent.unwrap_or(0.0);
            
            let is_daytime = fc.valid_at.with_timezone(wita_offset).hour() >= 6 
                && fc.valid_at.with_timezone(wita_offset).hour() < 18;
            
            let stability = gaussian_plume::pasquill_stability_class(ws, is_daytime);
            let spread_angle = match stability {
                'A' => 25.0, 'B' => 20.0, 'C' => 15.0,
                'D' => 12.5, 'E' => 8.75, 'F' => 5.0,
                _ => 12.5,
            };
            
            let (sy, sz) = gaussian_plume::dispersion_coefficients_1km(stability);
            let q_g_s = source.emission_rate_kg_hr * 1000.0 / 3600.0;
            let conc_g_m3 = gaussian_plume::concentration_centerline(q_g_s, ws, sy, sz);
            let conc_1km = gaussian_plume::mgm3_to_ppm_ch4(conc_g_m3 * 1000.0);
            
            let mut distance = ws * 3600.0;
            if hum > 85.0 { distance *= 0.60; }
            
            // Generate plume polygon
            let plume_geojson = self.generate_plume_polygon(source.lon, source.lat, distance, wd, spread_angle).await?;
            
            // Check intersection with zones
            let geom_str = serde_json::to_string(&plume_geojson).unwrap_or_default();
            let affected_zones = ZonesRepository::get_intersecting(&self.pool, &geom_str).await?;
            
            // Send alert if needed
            if conc_1km > 50.0 && !affected_zones.is_empty() {
                for zone in &affected_zones {
                    self.alert_service.send_alert(
                        zone,
                        source.emission_rate_kg_hr,
                        distance,
                        conc_1km,
                        ws,
                        wd,
                        &stability.to_string(),
                    ).await?;
                }
            }
            
            result.push(ForecastedPlume {
                valid_at: fc.valid_at,
                wind_speed_ms: ws,
                wind_direction_deg: wd,
                stability_class: stability,
                spread_angle_deg: spread_angle,
                max_distance_m: distance,
                concentration_at_1km_ppm: conc_1km,
                plume_geojson,
                terrain_blocked: false,
                terrain_block_distance_m: None,
                exposure_alert: !affected_zones.is_empty(),
                affected_zones,
            });
        }
        
        Ok(result)
    }
    
    async fn generate_plume_polygon(
        &self,
        lon: f64,
        lat: f64,
        distance: f64,
        wind_direction: f64,
        spread_angle: f64,
    ) -> Result<serde_json::Value, AppError> {
        let result = sqlx::query(
            r#"WITH plume AS (
                SELECT ST_MakePolygon(ST_MakeLine(ARRAY[
                    ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geometry,
                    ST_Project(ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geography, $3::FLOAT8, radians($4::FLOAT8 + 180.0 - $5::FLOAT8))::geometry,
                    ST_Project(ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geography, $3::FLOAT8, radians($4::FLOAT8 + 180.0 + $5::FLOAT8))::geometry,
                    ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geometry
                ])) as geom
               )
               SELECT ST_AsGeoJSON(geom) as json FROM plume"#,
        )
        .bind(lon)
        .bind(lat)
        .bind(distance)
        .bind(wind_direction)
        .bind(spread_angle)
        .fetch_one(&self.pool)
        .await?;
        
        let json_str: String = result.get("json");
        Ok(serde_json::from_str(&json_str).unwrap_or_default())
    }
}
