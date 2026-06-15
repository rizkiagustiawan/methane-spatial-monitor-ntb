/// Repository layer for database access
/// 
/// Separates database logic from business logic
/// Each repository handles one domain entity

use sqlx::PgPool;
use sqlx::Row;
use crate::errors::AppError;
use crate::models::*;

/// Methane observations repository
#[allow(dead_code)]
pub struct MethaneRepository;

#[allow(dead_code)]
impl MethaneRepository {
    pub async fn get_recent(pool: &PgPool, limit: i64) -> Result<Vec<MethanePlumeResponse>, AppError> {
        let records = sqlx::query(
            r#"SELECT recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as geometry,
               ST_AsGeoJSON(plume_geometry) as plume_geometry_json, source
             FROM methane_observations ORDER BY recorded_at DESC LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(pool)
        .await?;
        
        let mut results = Vec::new();
        for row in records {
            let geometry_str: String = row.get("geometry");
            let plume_geometry_json: Option<String> = row.get("plume_geometry_json");
            results.push(MethanePlumeResponse {
                recorded_at: row.get("recorded_at"),
                emission_rate_kg_hr: row.get("emission_rate_kg_hr"),
                geometry: serde_json::from_str(&geometry_str).unwrap_or_default(),
                plume_footprint: plume_geometry_json.and_then(|g: String| serde_json::from_str(&g).ok()),
                source: row.get("source"),
            });
        }
        Ok(results)
    }
    
    pub async fn get_active_sources(pool: &PgPool) -> Result<Vec<ActiveSource>, AppError> {
        let records = sqlx::query(
            r#"SELECT id, ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat, emission_rate_kg_hr
               FROM methane_observations WHERE recorded_at > NOW() - INTERVAL '24 hours'"#,
        )
        .fetch_all(pool)
        .await?;

        let mut results = Vec::new();
        for row in records {
            let lon: Option<f64> = row.get("lon");
            let lat: Option<f64> = row.get("lat");
            if let (Some(lon), Some(lat)) = (lon, lat) {
                results.push(ActiveSource {
                    id: row.get("id"),
                    lon,
                    lat,
                    emission_rate_kg_hr: row.get("emission_rate_kg_hr"),
                });
            }
        }
        Ok(results)
    }
    
    pub async fn insert(
        pool: &PgPool,
        recorded_at: chrono::DateTime<chrono::Utc>,
        emission_rate: f64,
        geometry: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location, plume_geometry, source) 
             VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)), ST_GeomFromGeoJSON($3), 'carbon_mapper') 
             ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
        )
        .bind(recorded_at)
        .bind(emission_rate)
        .bind(geometry)
        .execute(pool)
        .await?;
        
        Ok(result.rows_affected() > 0)
    }
}

/// Weather observations repository
#[allow(dead_code)]
pub struct WeatherRepository;

#[allow(dead_code)]
impl WeatherRepository {
    pub async fn get_latest(pool: &PgPool, limit: i64) -> Result<Vec<WeatherObservation>, AppError> {
        let records = sqlx::query_as::<_, WeatherObservation>(
            "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
             FROM weather_observations ORDER BY recorded_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?;
        Ok(records)
    }
    
    pub async fn get_latest_for_region(pool: &PgPool, region: &str) -> Result<Option<WeatherObservation>, AppError> {
        let record = sqlx::query_as::<_, WeatherObservation>(
            r#"SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
               FROM weather_observations 
               WHERE area_id = $1 AND recorded_at > NOW() - INTERVAL '6 hours'
               ORDER BY recorded_at DESC LIMIT 1"#,
        )
        .bind(region)
        .fetch_optional(pool)
        .await?;
        Ok(record)
    }
}

/// Weather forecast repository
#[allow(dead_code)]
pub struct ForecastRepository;

#[allow(dead_code)]
impl ForecastRepository {
    pub async fn get_upcoming(pool: &PgPool, limit: i64) -> Result<Vec<WeatherForecast>, AppError> {
        let records = sqlx::query(
            r#"SELECT forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
               FROM weather_forecasts
               WHERE valid_at > NOW()
               ORDER BY valid_at ASC
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(pool)
        .await?;

        let mut results = Vec::new();
        for row in records {
            results.push(WeatherForecast {
                id: 0,
                forecast_at: row.get("forecast_at"),
                valid_at: row.get("valid_at"),
                area_id: row.get("area_id"),
                wind_speed_ms: row.get("wind_speed_ms"),
                wind_direction_deg: row.get("wind_direction_deg"),
                humidity_percent: row.get("humidity_percent"),
                temperature_c: row.get("temperature_c"),
                data_source: row.get::<String, _>("data_source"),
            });
        }
        Ok(results)
    }
    
    pub async fn get_for_region(pool: &PgPool, region: &str) -> Result<Vec<WeatherForecast>, AppError> {
        let records = sqlx::query(
            r#"SELECT forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
               FROM weather_forecasts 
               WHERE area_id = $1 AND valid_at > NOW() AND valid_at < NOW() + INTERVAL '6 hours'
               ORDER BY valid_at ASC"#,
        )
        .bind(region)
        .fetch_all(pool)
        .await?;

        let mut results = Vec::new();
        for row in records {
            results.push(WeatherForecast {
                id: 0,
                forecast_at: row.get("forecast_at"),
                valid_at: row.get("valid_at"),
                area_id: row.get("area_id"),
                wind_speed_ms: row.get("wind_speed_ms"),
                wind_direction_deg: row.get("wind_direction_deg"),
                humidity_percent: row.get("humidity_percent"),
                temperature_c: row.get("temperature_c"),
                data_source: row.get::<String, _>("data_source"),
            });
        }
        Ok(results)
    }
}

/// Populated zones repository
#[allow(dead_code)]
pub struct ZonesRepository;

#[allow(dead_code)]
impl ZonesRepository {
    pub async fn get_all(pool: &PgPool) -> Result<Vec<serde_json::Value>, AppError> {
        let records = sqlx::query(
            r#"SELECT zone_name, region, zone_type, ST_AsGeoJSON(geometry) as geom 
               FROM populated_zones"#,
        )
        .fetch_all(pool)
        .await?;
        
        let mut results = Vec::new();
        for row in records {
            let geom: String = row.get("geom");
            results.push(serde_json::json!({
                "type": "Feature",
                "properties": {
                    "name": row.get::<String, _>("zone_name"),
                    "region": row.get::<String, _>("region"),
                    "type": row.get::<String, _>("zone_type")
                },
                "geometry": serde_json::from_str::<serde_json::Value>(&geom).unwrap_or_default()
            }));
        }
        Ok(results)
    }
    
    pub async fn get_intersecting(pool: &PgPool, geojson: &str) -> Result<Vec<AffectedZone>, AppError> {
        let records = sqlx::query(
            r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
               FROM populated_zones 
               WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
        )
        .bind(geojson)
        .fetch_all(pool)
        .await?;

        let mut results = Vec::new();
        for row in records {
            results.push(AffectedZone {
                zone_name: row.get("zone_name"),
                region: row.get("region"),
                population_estimate: row.get("population_estimate"),
                zone_type: row.get("zone_type"),
                is_volcanic_zone: row.get("is_volcanic_zone"),
            });
        }
        Ok(results)
    }
}

/// Alert repository
#[allow(dead_code)]
pub struct AlertRepository;

#[allow(dead_code)]
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
        sqlx::query(
            "INSERT INTO evacuation_alerts (region, zone_name, emission_rate_kg_hr, wind_speed_ms, wind_direction_deg, concentration_ppm, stability_class) 
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(region)
        .bind(zone_name)
        .bind(emission_rate)
        .bind(wind_speed)
        .bind(wind_direction)
        .bind(concentration)
        .bind(stability_class)
        .execute(pool)
        .await?;
        
        Ok(())
    }
}

/// Helper struct for active sources
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ActiveSource {
    pub id: uuid::Uuid,
    pub lon: f64,
    pub lat: f64,
    pub emission_rate_kg_hr: f64,
}
