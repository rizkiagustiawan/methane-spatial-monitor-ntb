/// Repository layer for database access
/// 
/// Separates database logic from business logic
/// Each repository handles one domain entity

use sqlx::PgPool;
use crate::errors::AppError;
use crate::models::*;

/// Methane observations repository
pub struct MethaneRepository;

impl MethaneRepository {
    pub async fn get_recent(pool: &PgPool, limit: i64) -> Result<Vec<MethanePlumeResponse>, AppError> {
        let records = sqlx::query!(
            r#"SELECT recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as "geometry!",
               ST_AsGeoJSON(plume_geometry) as "plume_geometry_json", source
             FROM methane_observations ORDER BY recorded_at DESC LIMIT $1"#,
            limit
        ).fetch_all(pool).await?;
        
        Ok(records.into_iter().map(|row| MethanePlumeResponse {
            recorded_at: row.recorded_at,
            emission_rate_kg_hr: row.emission_rate_kg_hr,
            geometry: serde_json::from_str(&row.geometry).unwrap_or_default(),
            plume_footprint: row.plume_geometry_json.and_then(|g| serde_json::from_str(&g).ok()),
            source: row.source,
        }).collect())
    }
    
    pub async fn get_active_sources(pool: &PgPool) -> Result<Vec<ActiveSource>, AppError> {
        Ok(sqlx::query!(
            r#"SELECT id, ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat, emission_rate_kg_hr
               FROM methane_observations WHERE recorded_at > NOW() - INTERVAL '24 hours'"#
        ).fetch_all(pool).await?
        .into_iter()
        .filter_map(|r| {
            Some(ActiveSource {
                id: r.id,
                lon: r.lon?,
                lat: r.lat?,
                emission_rate_kg_hr: r.emission_rate_kg_hr,
            })
        })
        .collect())
    }
    
    pub async fn insert(
        pool: &PgPool,
        recorded_at: chrono::DateTime<chrono::Utc>,
        emission_rate: f64,
        geometry: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query!(
            "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location, plume_geometry, source) 
             VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)), ST_GeomFromGeoJSON($3), 'carbon_mapper') 
             ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
            recorded_at, emission_rate, geometry
        ).execute(pool).await?;
        
        Ok(result.rows_affected() > 0)
    }
}

/// Weather observations repository
pub struct WeatherRepository;

impl WeatherRepository {
    pub async fn get_latest(pool: &PgPool, limit: i64) -> Result<Vec<WeatherObservation>, AppError> {
        Ok(sqlx::query_as!(WeatherObservation,
            "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
             FROM weather_observations ORDER BY recorded_at DESC LIMIT $1",
            limit
        ).fetch_all(pool).await?)
    }
    
    pub async fn get_latest_for_region(pool: &PgPool, region: &str) -> Result<Option<WeatherObservation>, AppError> {
        Ok(sqlx::query_as!(WeatherObservation,
            r#"SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
               FROM weather_observations 
               WHERE area_id = $1 AND recorded_at > NOW() - INTERVAL '6 hours'
               ORDER BY recorded_at DESC LIMIT 1"#,
            region
        ).fetch_optional(pool).await?)
    }
}

/// Weather forecast repository
pub struct ForecastRepository;

impl ForecastRepository {
    pub async fn get_upcoming(pool: &PgPool, limit: i64) -> Result<Vec<WeatherForecast>, AppError> {
        Ok(sqlx::query!(
            r#"SELECT forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
               FROM weather_forecasts
               WHERE valid_at > NOW()
               ORDER BY valid_at ASC
               LIMIT $1"#,
            limit
        ).fetch_all(pool).await?
        .into_iter()
        .map(|r| WeatherForecast {
            id: 0,
            forecast_at: r.forecast_at,
            valid_at: r.valid_at,
            area_id: r.area_id,
            wind_speed_ms: r.wind_speed_ms,
            wind_direction_deg: r.wind_direction_deg,
            humidity_percent: r.humidity_percent,
            temperature_c: r.temperature_c,
            data_source: r.data_source.unwrap_or_default(),
        })
        .collect())
    }
    
    pub async fn get_for_region(pool: &PgPool, region: &str) -> Result<Vec<WeatherForecast>, AppError> {
        Ok(sqlx::query!(
            r#"SELECT forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
               FROM weather_forecasts 
               WHERE area_id = $1 AND valid_at > NOW() AND valid_at < NOW() + INTERVAL '6 hours'
               ORDER BY valid_at ASC"#,
            region
        ).fetch_all(pool).await?
        .into_iter()
        .map(|r| WeatherForecast {
            id: 0,
            forecast_at: r.forecast_at,
            valid_at: r.valid_at,
            area_id: r.area_id,
            wind_speed_ms: r.wind_speed_ms,
            wind_direction_deg: r.wind_direction_deg,
            humidity_percent: r.humidity_percent,
            temperature_c: r.temperature_c,
            data_source: r.data_source.unwrap_or_default(),
        })
        .collect())
    }
}

/// Populated zones repository
pub struct ZonesRepository;

impl ZonesRepository {
    pub async fn get_all(pool: &PgPool) -> Result<Vec<serde_json::Value>, AppError> {
        let records = sqlx::query!(
            r#"SELECT zone_name, region, zone_type, ST_AsGeoJSON(geometry) as "geom!" 
               FROM populated_zones"#
        ).fetch_all(pool).await?;
        
        Ok(records.into_iter().map(|r| {
            serde_json::json!({
                "type": "Feature",
                "properties": { "name": r.zone_name, "region": r.region, "type": r.zone_type },
                "geometry": serde_json::from_str::<serde_json::Value>(&r.geom).unwrap_or_default()
            })
        }).collect())
    }
    
    pub async fn get_intersecting(pool: &PgPool, geojson: &str) -> Result<Vec<AffectedZone>, AppError> {
        Ok(sqlx::query!(
            r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
               FROM populated_zones 
               WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
            geojson
        ).fetch_all(pool).await?
        .into_iter()
        .map(|r| AffectedZone {
            zone_name: r.zone_name,
            region: r.region,
            population_estimate: r.population_estimate,
            zone_type: r.zone_type,
            is_volcanic_zone: r.is_volcanic_zone,
        })
        .collect())
    }
}

/// Alert repository
pub struct AlertRepository;

impl AlertRepository {
    pub async fn insert(
        pool: &PgPool,
        region: &str,
        zone_name: &str,
        emission_rate: f64,
        wind_speed: f64,
        wind_direction: f64,
        concentration: f64,
        stability_class: &str,
    ) -> Result<(), AppError> {
        sqlx::query!(
            "INSERT INTO evacuation_alerts (region, zone_name, emission_rate_kg_hr, wind_speed_ms, wind_direction_deg, concentration_ppm, stability_class) 
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            region, zone_name, emission_rate, wind_speed, wind_direction, concentration, stability_class
        ).execute(pool).await?;
        
        Ok(())
    }
}

/// Helper struct for active sources
#[derive(Debug, Clone)]
pub struct ActiveSource {
    pub id: uuid::Uuid,
    pub lon: f64,
    pub lat: f64,
    pub emission_rate_kg_hr: f64,
}
