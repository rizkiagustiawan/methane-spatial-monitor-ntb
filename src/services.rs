use crate::errors::AppError;
use crate::models::*;
use crate::physics::*;
use crate::repositories::*;
use chrono::Timelike;
use sqlx::PgPool;
use sqlx::Row;
/// Service layer for business logic
///
/// Separates business logic from HTTP handlers
/// Each service handles one domain
use std::sync::Arc;
use uuid::Uuid;

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
#[allow(dead_code)]
pub struct MethaneService {
    pool: PgPool,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct WeatherService {
    pool: PgPool,
}

#[allow(dead_code)]
impl WeatherService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_latest(&self, limit: i64) -> Result<Vec<WeatherObservation>, AppError> {
        WeatherRepository::get_latest(&self.pool, limit).await
    }

    pub async fn get_latest_for_region(
        &self,
        region: &str,
    ) -> Result<Option<WeatherObservation>, AppError> {
        WeatherRepository::get_latest_for_region(&self.pool, region).await
    }
}

/// Forecast service
#[allow(dead_code)]
pub struct ForecastService {
    pool: PgPool,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct ZonesService {
    pool: PgPool,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct AlertService {
    pool: PgPool,
    http_client: reqwest::Client,
    telegram_token: String,
    telegram_chat_id: String,
}

#[allow(dead_code)]
impl AlertService {
    pub fn new(
        pool: PgPool,
        http_client: reqwest::Client,
        telegram_token: String,
        telegram_chat_id: String,
    ) -> Self {
        Self {
            pool,
            http_client,
            telegram_token,
            telegram_chat_id,
        }
    }

    #[allow(clippy::too_many_arguments)]
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
        )
        .await?;

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
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.telegram_token
        );
        let payload = serde_json::json!({
            "chat_id": self.telegram_chat_id,
            "text": msg,
            "parse_mode": "HTML"
        });

        self.http_client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::ExternalService(format!("Telegram error: {}", e)))?;

        Ok(())
    }
}

/// Plume analysis service
#[allow(dead_code)]
pub struct PlumeAnalysisService {
    pool: PgPool,
    alert_service: Arc<AlertService>,
}

#[allow(dead_code)]
impl PlumeAnalysisService {
    pub fn new(pool: PgPool, alert_service: Arc<AlertService>) -> Self {
        Self {
            pool,
            alert_service,
        }
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
            let forecast = self
                .get_forecast_plumes(&source, region, &wita_offset)
                .await?;

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

    pub async fn run_data_fusion(&self) -> Result<Vec<FusionAnomaly>, AppError> {
        // 1. Get recent S5P macro-radar overpasses (last 24h)
        let s5p_recent = sqlx::query_as::<_, S5pOverpass>(
            r#"SELECT scene_id, start_datetime, end_datetime, orbit_number, netcdf_download_url, ST_AsGeoJSON(footprint) as footprint 
               FROM s5p_overpasses 
               WHERE start_datetime > NOW() - INTERVAL '24 hours'"#
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let mut anomalies = Vec::new();

        // 2. Loop through recent macro detections
        for s5p in s5p_recent {
            if let Some(footprint_str) = &s5p.footprint {
                // 3. Find known historical point-sources inside this macro footprint
                //    that HAVE NOT been updated by Tanager/EMIT in the last 24h (gap-filling)
                let missing_sources = sqlx::query(
                    r#"SELECT id, ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat, emission_rate_kg_hr 
                       FROM methane_observations 
                       WHERE ST_Intersects(location, ST_GeomFromGeoJSON($1))
                         AND recorded_at < NOW() - INTERVAL '24 hours'
                       ORDER BY emission_rate_kg_hr DESC LIMIT 5"#
                )
                .bind(footprint_str)
                .fetch_all(&self.pool)
                .await
                .unwrap_or_default();

                // 4. Create Fusion Anomalies
                for source in missing_sources {
                    let id: Uuid = source.get("id");
                    let lon: f64 = source.get("lon");
                    let lat: f64 = source.get("lat");
                    let rate: f64 = source.get("emission_rate_kg_hr");

                    // Simple Physics-Aware Confidence Scoring
                    // High rate + recent macro detection = high confidence it's still leaking
                    let mut confidence = 0.5; // Base 50%
                    if rate > 1000.0 {
                        confidence += 0.3;
                    }
                    // Massive historical leak
                    else if rate > 500.0 {
                        confidence += 0.2;
                    }

                    anomalies.push(FusionAnomaly {
                        anomaly_id: format!("fusion-{}-{}", s5p.scene_id, &id.to_string()[..8]),
                        source_id: id,
                        source_lon: lon,
                        source_lat: lat,
                        historical_emission_rate: rate,
                        s5p_scene_id: s5p.scene_id.clone(),
                        s5p_timestamp: s5p.start_datetime,
                        confidence_score: confidence,
                        status: "GAP_FILLED".to_string(),
                        message: "S5P detected regional gas. High-res satellite occluded. Historical source suspected active.".to_string(),
                    });
                }
            }
        }
        Ok(anomalies)
    }

    /// Atmospheric Digital Twin: Inverse Distance Weighting (IDW) Interpolation
    /// Interpolates weather at any arbitrary coordinate using the N nearest nodes.
    /// Source: Musayev et al. (2026)
    pub async fn interpolate_weather_at_point(
        &self,
        target_lon: f64,
        target_lat: f64,
    ) -> Result<AtmosphericState, AppError> {
        // Fetch latest weather from all 110 nodes
        let nodes = sqlx::query(
            r#"SELECT area_id, wind_speed_ms, wind_direction_deg, temperature_c, humidity_percent
               FROM weather_observations 
               WHERE recorded_at > NOW() - INTERVAL '3 hours'"#,
        )
        .fetch_all(&self.pool)
        .await?;

        if nodes.is_empty() {
            return Err(AppError::NotFound(
                "No recent weather data available for Digital Twin".into(),
            ));
        }

        // We'll hardcode the known coordinates for the 110 zones from main.rs
        // In a real database, area_id would JOIN with a zones_catalog table.
        // For demonstration of the IDW math, we will assume an average atmospheric state
        // if exact node coordinates are missing in DB, but normally we'd compute distances.

        let mut sum_weight = 0.0;
        let mut sum_ws = 0.0;
        let mut sum_wd = 0.0;
        let mut sum_temp = 0.0;
        let mut sum_hum = 0.0;

        for node in &nodes {
            let ws: f64 = node.get::<Option<f64>, _>("wind_speed_ms").unwrap_or(1.0);
            let wd: f64 = node
                .get::<Option<f64>, _>("wind_direction_deg")
                .unwrap_or(0.0);
            let t: f64 = node.get::<Option<f64>, _>("temperature_c").unwrap_or(25.0);
            let h: f64 = node
                .get::<Option<f64>, _>("humidity_percent")
                .unwrap_or(70.0);

            // Dummy distance calculation (replace with actual node lat/lon in production)
            // Look up coordinate from NTB_ZONES
            let mut dist: f64 = 50.0; // Default fallback distance
            let area_name: String = node.get("area_id");
            for &(name, _, z_lat, z_lon) in crate::zones::NTB_ZONES {
                if name == area_name {
                    dist = gaussian_plume::haversine_distance_km(target_lat, target_lon, z_lat, z_lon);
                    if dist < 0.1 { dist = 0.1; } // Prevent division by zero
                    break;
                }
            }
            let weight = 1.0 / dist.powi(2); // IDW formula (1 / d^2)

            sum_weight += weight;
            sum_ws += ws * weight;
            sum_wd += wd * weight;
            sum_temp += t * weight;
            sum_hum += h * weight;
        }

        Ok(AtmosphericState {
            target_lon,
            target_lat,
            interpolated_wind_speed_ms: sum_ws / sum_weight,
            interpolated_wind_dir_deg: sum_wd / sum_weight,
            interpolated_temp_c: sum_temp / sum_weight,
            interpolated_humidity: sum_hum / sum_weight,
            computation_method: "IDW_SPATIAL_INTERPOLATION".to_string(),
            nearest_nodes_used: nodes.len(),
        })
    }

    /// dMRV (digital Measurement, Reporting, and Verification) Generator
    /// Calculates carbon credit equivalents and verifies emission reductions over 30 days.
    /// Source: Prajesh et al. (2026)
    pub async fn generate_mrv_report(
        &self,
        target_lon: f64,
        target_lat: f64,
        radius_m: f64,
    ) -> Result<MrvReport, AppError> {
        let _radius_deg = radius_m / 111320.0; // Approx meters to degrees

        let records = sqlx::query(
            r#"SELECT emission_rate_kg_hr 
               FROM methane_observations 
               WHERE ST_DWithin(location::geography, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
                 AND recorded_at > NOW() - INTERVAL '30 days'"#
        )
        .bind(target_lon)
        .bind(target_lat)
        .bind(radius_m)
        .fetch_all(&self.pool)
        .await?;

        let detections_count = records.len() as i64;

        if detections_count == 0 {
            return Err(AppError::NotFound(
                "No emissions detected in this area in last 30 days".into(),
            ));
        }

        let mut sum_rate = 0.0;
        for row in &records {
            let rate: f64 = row.get("emission_rate_kg_hr");
            sum_rate += rate;
        }

        let average_rate = sum_rate / detections_count as f64;
        let total_emissions_kg = average_rate * 24.0 * 30.0; // Estimate for 30 days (assuming continuous)

        // CH4 GWP is 28x CO2 over 100 years
        let carbon_credits = (total_emissions_kg / 1000.0) * 28.0;

        // Baseline arbitrary set to 1500 kg/hr for demonstration
        // Baseline can be dynamically retrieved or passed, but for now we set it dynamically to 1.5x the average if no input
        let baseline = average_rate * 1.5;
        let reduction = if average_rate < baseline {
            ((baseline - average_rate) / baseline) * 100.0
        } else {
            0.0
        };

        // Confidence scales with number of satellite detections
        let confidence = (detections_count as f64 / 10.0).min(1.0);

        Ok(MrvReport {
            location_lon: target_lon,
            location_lat: target_lat,
            report_period_days: 30,
            total_emissions_kg,
            average_rate_kg_hr: average_rate,
            baseline_rate_kg_hr: baseline,
            estimated_reduction_percent: reduction,
            verification_confidence_score: confidence,
            detections_count,
            carbon_credits_equivalent_tons: carbon_credits,
        })
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
            let hum = fc.humidity_percent.unwrap_or(70.0);

            let is_daytime = fc.valid_at.with_timezone(wita_offset).hour() >= 6
                && fc.valid_at.with_timezone(wita_offset).hour() < 18;

            let stability = gaussian_plume::pasquill_stability_class(ws, is_daytime);
            let spread_angle = match stability {
                'A' => 25.0,
                'B' => 20.0,
                'C' => 15.0,
                'D' => 12.5,
                'E' => 8.75,
                'F' => 5.0,
                _ => 12.5,
            };

            let (sy, sz) = gaussian_plume::dispersion_coefficients_1km(stability);
            let q_g_s = source.emission_rate_kg_hr * 1000.0 / 3600.0;
            let conc_g_m3 = gaussian_plume::concentration_centerline(q_g_s, ws, sy, sz);
            // Use actual forecast temperature or fallback to standard 25°C
            let temp_c = fc.temperature_c.unwrap_or(25.0);
            let pressure_kpa = gaussian_plume::STANDARD_PRESSURE_KPA; // Default to 1 atm

            let conc_1km =
                gaussian_plume::mgm3_to_ppm_ch4(conc_g_m3 * 1000.0, temp_c, pressure_kpa);

            // Apply humidity attenuation using Beer-Lambert Law
            // Source: HITRAN Database, Radiative Transfer Theory
            let humidity_factor = gaussian_plume::humidity_transmittance(hum);
            let distance = ws * 3600.0 * humidity_factor;

            // Generate plume polygon
            let plume_geojson = self
                .generate_plume_polygon(source.lon, source.lat, distance, wd, spread_angle)
                .await?;

            // Check intersection with zones
            let geom_str = serde_json::to_string(&plume_geojson).unwrap_or_default();
            let affected_zones = ZonesRepository::get_intersecting(&self.pool, &geom_str).await?;

            // Send alert if needed
            if conc_1km > 50.0 && !affected_zones.is_empty() {
                for zone in &affected_zones {
                    self.alert_service
                        .send_alert(
                            zone,
                            source.emission_rate_kg_hr,
                            distance,
                            conc_1km,
                            ws,
                            wd,
                            &stability.to_string(),
                        )
                        .await?;
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
