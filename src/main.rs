use axum::{extract::State, http::StatusCode, response::{Html, IntoResponse, Json as AxumJson}, routing::get, Router};
use chrono::{FixedOffset, Timelike, Utc};
use dotenvy::dotenv;
use gdal::Dataset;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{error, info, warn};
use tower_http::cors::{AllowOrigin, CorsLayer};

mod models;
use models::*;

/// Shared application state passed to all handlers and background tasks.
struct AppState {
    pool: Pool<Postgres>,
    http_client: reqwest::Client,
}

fn get_elevation_at_point(lon: f64, lat: f64) -> Option<f32> {
    let dataset = Dataset::open("ntb_dem.tif").ok()?;
    let gt = dataset.geo_transform().ok()?;
    let band = dataset.rasterband(1).ok()?;

    // Check for degenerate GeoTransform (division by zero)
    let det = gt[1] * gt[5] - gt[2] * gt[4];
    if det.abs() < 1e-12 {
        error!("DEM GeoTransform has zero determinant -- degenerate transform");
        return None;
    }

    let inv_det = 1.0 / det;
    let x = (inv_det * (gt[5] * (lon - gt[0]) - gt[2] * (lat - gt[3]))) as isize;
    let y = (inv_det * (-gt[4] * (lon - gt[0]) + gt[1] * (lat - gt[3]))) as isize;

    let (size_x, size_y) = band.size();
    if x < 0 || y < 0 || x >= size_x as isize || y >= size_y as isize {
        return None;
    }

    let rv = band.read_as::<f32>((x, y), (1, 1), (1, 1), None).ok()?;
    Some(rv.data()[0])
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("Failed to connect to Postgres");

    let http_client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .expect("Failed to build HTTP client");

    let shared_state = Arc::new(AppState {
        pool,
        http_client,
    });

    // Spawn Background Tasks
    let state_stac = Arc::clone(&shared_state);
    tokio::spawn(async move {
        // Wait for server to be ready before first tick
        tokio::time::sleep(Duration::from_secs(5)).await;
        carbon_mapper_tracker_task(state_stac).await;
    });

    let state_bmkg = Arc::clone(&shared_state);
    tokio::spawn(async move {
        // Wait for server to be ready before first tick
        tokio::time::sleep(Duration::from_secs(5)).await;
        bmkg_tracker_task(state_bmkg).await;
    });

    // CORS: allow same-origin + configurable origins
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            let origin_str = origin.as_bytes();
            // Allow localhost origins for development
            origin_str.starts_with(b"http://localhost")
                || origin_str.starts_with(b"http://127.0.0.1")
                || origin_str.starts_with(b"https://localhost")
        }))
        .allow_methods([axum::http::Method::GET])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    // API Routes
    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("../frontend/index.html")) }))
        .route("/health", get(|| async { "OK" }))
        .route("/api/weather", get(get_latest_weather))
        .route("/api/methane", get(get_latest_methane))
        .route("/api/methane/plumes", get(get_methane_plumes))
        .route("/api/plume-prediction", get(get_plume_prediction))
        .layer(cors)
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("Failed to bind to 0.0.0.0:3000 -- is another process using this port?");
    info!("Server running on http://localhost:3000");
    axum::serve(listener, app)
        .await
        .expect("Server encountered a fatal error");
}

async fn get_methane_plumes(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match sqlx::query!(
        r#"SELECT recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as "geometry!"
         FROM methane_observations
         ORDER BY recorded_at DESC
         LIMIT 100"#
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(records) => {
            let plumes: Vec<MethanePlumeResponse> = records
                .into_iter()
                .map(|row| MethanePlumeResponse {
                    recorded_at: row.recorded_at,
                    emission_rate_kg_hr: row.emission_rate_kg_hr,
                    geometry: serde_json::from_str(&row.geometry).unwrap_or_default(),
                })
                .collect();
            (StatusCode::OK, AxumJson(json!(plumes))).into_response()
        }
        Err(e) => {
            error!("Database error fetching methane plumes: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "Failed to fetch plume data"}))).into_response()
        }
    }
}

async fn get_latest_weather(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match sqlx::query_as!(
        WeatherObservation,
        "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
         FROM weather_observations
         ORDER BY recorded_at DESC
         LIMIT 10"
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(records) => {
            (StatusCode::OK, AxumJson(json!(records))).into_response()
        }
        Err(e) => {
            error!("Database error fetching weather data: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "Failed to fetch weather data"}))).into_response()
        }
    }
}

async fn get_latest_methane(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match sqlx::query_as!(
        MethaneObservation,
        r#"SELECT id, recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as "location_json!", 0.0::FLOAT8 as "total_green_area_hectares!"
         FROM methane_observations
         ORDER BY recorded_at DESC
         LIMIT 10"#
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(records) => {
            (StatusCode::OK, AxumJson(json!(records))).into_response()
        }
        Err(e) => {
            error!("Database error fetching methane observations: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "Failed to fetch methane data"}))).into_response()
        }
    }
}

fn get_pasquill_stability_class(wind_speed_ms: f64, is_daytime: bool) -> char {
    if is_daytime {
        if wind_speed_ms < 3.0 {
            'A'
        } else if wind_speed_ms < 5.0 {
            'B'
        } else {
            'C'
        }
    } else if wind_speed_ms < 3.0 {
        'F'
    } else if wind_speed_ms < 5.0 {
        'E'
    } else {
        'D'
    }
}

fn get_plume_spread_angle(stability_class: char) -> f64 {
    match stability_class {
        'A' => 25.0,
        'B' => 20.0,
        'C' => 15.0,
        'D' => 12.5,
        'E' => 8.75,
        'F' => 5.0,
        _ => 12.5,
    }
}

fn get_region_from_coords(lon: f64, lat: f64) -> &'static str {
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

async fn send_telegram_alert(client: &reqwest::Client, msg: &str) {
    let token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default();
    if token.is_empty() || chat_id.is_empty() {
        error!("Telegram configuration missing (TOKEN/CHAT_ID)");
        return;
    }

    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": msg,
        "parse_mode": "HTML"
    });

    if let Err(e) = client.post(url).json(&payload).send().await {
        error!("Failed to send Telegram alert: {}", e);
    } else {
        info!("Telegram alert sent successfully.");
    }
}

async fn get_plume_prediction(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // 1. Fetch latest source
    let source = match sqlx::query!(
        r#"
        SELECT 
            ST_X(location::geometry) as lon, 
            ST_Y(location::geometry) as lat,
            emission_rate_kg_hr
        FROM methane_observations 
        ORDER BY recorded_at DESC LIMIT 1
        "#
    ).fetch_optional(&state.pool).await {
        Ok(s) => s,
        Err(e) => {
            error!("Database error fetching latest methane source: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "Database error"}))).into_response();
        }
    };

    let source = match source {
        Some(s) => s,
        None => return (StatusCode::OK, AxumJson(json!(null))).into_response(),
    };

    let source_lon = source.lon.unwrap_or(0.0);
    let source_lat = source.lat.unwrap_or(0.0);
    let region = get_region_from_coords(source_lon, source_lat);

    // 2. Fetch latest weather for the specific region
    let weather = match sqlx::query!(
        r#"
        SELECT wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c
        FROM weather_observations 
        WHERE area_id = $1 AND wind_speed_ms IS NOT NULL AND wind_direction_deg IS NOT NULL
        ORDER BY recorded_at DESC LIMIT 1
        "#,
        region
    ).fetch_optional(&state.pool).await {
        Ok(w) => w,
        Err(e) => {
            error!("Database error fetching weather for region {}: {}", region, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "Database error"}))).into_response();
        }
    };

    let weather = match weather {
        Some(w) => w,
        None => return (StatusCode::OK, AxumJson(json!(null))).into_response(),
    };

    let emission_rate = source.emission_rate_kg_hr;
    
    // REGIME I: Instrument Detector Bound (Shot Noise)
    if emission_rate < 10.0 {
        info!("Signal-to-Noise Ratio too low ({} < 10.0 kg/hr). Shot noise bound reached.", emission_rate);
        return (StatusCode::OK, AxumJson(json!(null))).into_response();
    }

    let ws = weather.wind_speed_ms.unwrap_or(0.0);
    let wd = weather.wind_direction_deg.unwrap_or(0.0);
    let hum = weather.humidity_percent.unwrap_or(0.0);
    let temp = weather.temperature_c.unwrap_or(0.0);

    let mut distance = ws * 3600.0; // 1-hour travel distance

    // REGIME II: Atmospheric Windows & Extinction
    if hum > 85.0 {
        info!("High atmospheric extinction (humidity {}% > 85%). Attenuating distance.", hum);
        distance *= 0.60;
    }

    // Calculate Stability Class using WITA timezone (UTC+8)
    let wita_offset = FixedOffset::east_opt(8 * 3600).expect("Invalid WITA offset");
    let now_wita = Utc::now().with_timezone(&wita_offset);
    let hour_wita = now_wita.hour();
    let is_daytime = hour_wita >= 6 && hour_wita < 18;
    
    let stability_class = get_pasquill_stability_class(ws, is_daytime);
    let mut spread_angle = get_plume_spread_angle(stability_class);

    // REGIME III: Thermal Emissivity Stretch
    let temp_k: f64 = temp + 273.15;
    let baseline_k: f64 = 308.15; // 35C
    if temp_k > baseline_k {
        let scale_factor = (baseline_k / temp_k).powi(4);
        info!("High thermal background (temp {}K > {}K). Dynamic Boltzmann scaling: {}", temp_k, baseline_k, scale_factor);
        spread_angle *= scale_factor;
    }

    // REGIME IV: Optomechanical Smear Limit
    // Default sensor pointing values for Tanager-1 when real telemetry is unavailable.
    // These conservative defaults flag potential MTF degradation.
    let sensor_roll = std::env::var("SENSOR_ROLL_DEG")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(7.0);
    let sensor_pitch = std::env::var("SENSOR_PITCH_DEG")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0);
    let is_smeared = sensor_roll > 6.85 || sensor_pitch > 4.8;
    if is_smeared {
        warn!("Optomechanical smear detected (Roll: {}, Pitch: {}). MTF mismatch.", sensor_roll, sensor_pitch);
    }

    // 2. Terrain-Aware Blocking
    let origin_elev = get_elevation_at_point(source_lon, source_lat).unwrap_or(0.0);
    
    for i in 1..=10 {
        let step_dist = distance * (i as f64 / 10.0);
        let dx = (wd + 180.0).to_radians().sin() * (step_dist / 111320.0);
        let dy = (wd + 180.0).to_radians().cos() * (step_dist / 110540.0);
        let check_lon = source_lon + dx;
        let check_lat = source_lat + dy;
        
        if let Some(elev) = get_elevation_at_point(check_lon, check_lat) {
             if elev > origin_elev + 15.0 {
                 info!("Plume blocked by terrain at {}m elevation (distance: {}m)", elev, step_dist);
                 distance = step_dist; 
                 break;
             }
        }
    }

    // 3. Generate final dispersion polygon
    let record = match sqlx::query_as!(
        PlumePrediction,
        r#"
        WITH plume AS (
            SELECT ST_MakePolygon(
                ST_MakeLine(ARRAY[
                    ST_SetSRID(ST_MakePoint($4::FLOAT8, $5::FLOAT8), 4326)::geometry,
                    ST_Project(ST_SetSRID(ST_MakePoint($4::FLOAT8, $5::FLOAT8), 4326)::geography, $6::FLOAT8, radians($3::FLOAT8 + 180.0::FLOAT8 - $7::FLOAT8))::geometry,
                    ST_Project(ST_SetSRID(ST_MakePoint($4::FLOAT8, $5::FLOAT8), 4326)::geography, $6::FLOAT8, radians($3::FLOAT8 + 180.0::FLOAT8 + $7::FLOAT8))::geometry,
                    ST_SetSRID(ST_MakePoint($4::FLOAT8, $5::FLOAT8), 4326)::geometry
                ])
            ) as geom
        )
        SELECT
            $1::FLOAT8 as "emission_rate_kg_hr!",
            $2::FLOAT8 as "wind_speed_ms!",
            $3::FLOAT8 as "wind_direction_deg!",
            ST_AsGeoJSON(geom) as "plume_line_json!",
            $8::BOOL as "high_uncertainty_smear!",
            ST_Intersects(geom, ST_MakeEnvelope(116.10, -8.67, 116.13, -8.64, 4326)) as "exposure_alert!"
        FROM plume
        "#,
        emission_rate,
        ws,
        wd,
        source_lon,
        source_lat,
        distance,
        spread_angle,
        is_smeared
    )
    .fetch_one(&state.pool)
    .await {
        Ok(r) => Some(r),
        Err(e) => {
            error!("Database error generating plume prediction polygon: {}", e);
            None
        }
    };

    if let Some(ref p) = record {
        if p.exposure_alert {
            let msg = format!(
                "<b>EVACUATION ALERT: Toxic Plume Exposure</b>\n\n<b>Region:</b> {}\n<b>Emission:</b> {:.2} kg/hr\n<b>Wind:</b> {:.2} m/s @ {:.0}deg",
                region, p.emission_rate_kg_hr, p.wind_speed_ms, p.wind_direction_deg
            );
            let client = state.http_client.clone();
            tokio::spawn(async move {
                send_telegram_alert(&client, &msg).await;
            });
        }
    }
     
    (StatusCode::OK, AxumJson(json!(record))).into_response()
}

async fn carbon_mapper_tracker_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(86400)); // Daily
    let api_token = std::env::var("CARBON_MAPPER_TOKEN").expect("CARBON_MAPPER_TOKEN must be set in .env");
    let url = "https://api.carbonmapper.org/api/v1/stac/search";

    loop {
        interval.tick().await;
        info!("Running Carbon Mapper STAC Tracker...");

        // Dynamic date range: from 2024-01-01 to now
        let end_date = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let datetime_range = format!("2024-01-01T00:00:00Z/{}", end_date);

        let payload = serde_json::json!({
            "bbox": [115.40, -9.15, 119.45, -8.00],
            "datetime": datetime_range,
            "limit": 30
        });

        match state.http_client.post(url)
            .header("X-API-KEY", &api_token)
            .json(&payload)
            .send()
            .await {
            Ok(res) => {
                if !res.status().is_success() {
                    error!("Carbon Mapper API error: {}", res.status());
                    continue;
                }

                match res.json::<StacResponse>().await {
                    Ok(stac) => {
                        info!("Carbon Mapper API responded successfully. Found {} features.", stac.features.len());
                        for feature in stac.features {
                            let dt_str = &feature.properties.datetime;
                            let emission_rate = feature.properties.emission_rate_kg_hr;

                            if emission_rate <= 0.0 {
                                continue;
                            }

                            let geom_json = serde_json::to_string(&feature.geometry).unwrap_or_default();

                            let res = sqlx::query!(
                                "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location)
                                 VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)))
                                 ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
                                chrono::DateTime::parse_from_rfc3339(dt_str).unwrap_or_default().with_timezone(&chrono::Utc),
                                emission_rate,
                                geom_json
                            ).execute(&state.pool).await;

                            if let Err(e) = res {
                                error!("DB Error (Carbon Mapper): {}", e);
                            } else {
                                info!("Recorded Carbon Mapper methane plume from {}", dt_str);
                            }
                        }
                    }
                    Err(e) => error!("JSON Parse Error (Carbon Mapper): {}", e),
                }
            }
            Err(e) => error!("Request Error (Carbon Mapper): {}", e),
        }
    }
}

struct Zone {
    name: &'static str,
    bmkg_id: &'static str,
    lat: &'static str,
    lon: &'static str,
}

async fn bmkg_tracker_task(state: Arc<AppState>) {
    // Schema Migration: Add data_source column if it doesn't exist
    match sqlx::query!("ALTER TABLE weather_observations ADD COLUMN IF NOT EXISTS data_source VARCHAR(50) NOT NULL DEFAULT 'Unknown';")
        .execute(&state.pool)
        .await
    {
        Ok(_) => info!("Schema migration: data_source column ensured"),
        Err(e) => warn!("Schema migration warning (data_source): {} -- column may already exist", e),
    }

    let mut interval = time::interval(Duration::from_secs(3600)); // Hourly
    let zones = vec![
        Zone { name: "Lombok Barat", bmkg_id: "52.01.01.2014", lat: "-8.6818", lon: "116.1240" },
        Zone { name: "Lombok Tengah", bmkg_id: "52.02.01.2001", lat: "-8.7167", lon: "116.2667" },
        Zone { name: "Lombok Timur", bmkg_id: "52.03.01.2001", lat: "-8.6500", lon: "116.5333" },
        Zone { name: "Sumbawa Barat", bmkg_id: "52.07.01.1001", lat: "-8.7333", lon: "116.8500" },
        Zone { name: "Kota Bima", bmkg_id: "52.72.01.1001", lat: "-8.4667", lon: "118.7167" },
    ];

    loop {
        interval.tick().await;
        info!("Running Weather Tracker Task for {} zones...", zones.len());

        for zone in &zones {
            let mut success = false;

            // Step A (Primary): BMKG JSON API
            let bmkg_url = format!("https://api.bmkg.go.id/publik/prakiraan-cuaca?adm={}", zone.bmkg_id);
            match state.http_client.get(&bmkg_url).send().await {
                Ok(res) if res.status().is_success() => {
                    if let Ok(bmkg_res) = res.json::<BmkgResponse>().await {
                        if let Some(group) = bmkg_res.data.first() {
                            if let Some(forecast_list) = group.cuaca.first() {
                                if let Some(item) = forecast_list.first() {
                                    let ws_ms = item.ws / 3.6;

                                    let db_res = sqlx::query!(
                                        "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source)
                                         VALUES (NOW(), $1, $2, $3, $4, $5, $6)",
                                        zone.name, ws_ms, item.wd_deg, item.hu, item.t, "BMKG"
                                    ).execute(&state.pool).await;

                                    if let Err(e) = db_res {
                                        error!("DB Error (BMKG) for {}: {}", zone.name, e);
                                    } else {
                                        success = true;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {
                    warn!("BMKG failed for {}, falling back to Open-Meteo", zone.name);
                }
            }

            // Step B (Fallback): Open-Meteo API
            if !success {
                let om_url = format!("https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,wind_speed_10m,wind_direction_10m,relative_humidity_2m&wind_speed_unit=ms", zone.lat, zone.lon);
                match state.http_client.get(&om_url).send().await {
                    Ok(res) if res.status().is_success() => {
                        match res.json::<OpenMeteoResponse>().await {
                            Ok(om_res) => {
                                let cur = om_res.current;
                                let db_res = sqlx::query!(
                                    "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source)
                                     VALUES (NOW(), $1, $2, $3, $4, $5, $6)",
                                    zone.name, cur.wind_speed_10m, cur.wind_direction_10m, cur.relative_humidity_2m, cur.temperature_2m, "Open-Meteo"
                                ).execute(&state.pool).await;

                                if let Err(e) = db_res {
                                    error!("DB Error (Open-Meteo) for {}: {}", zone.name, e);
                                }
                            }
                            Err(e) => error!("JSON Parse Error (Open-Meteo) for {}: {}", zone.name, e),
                        }
                    }
                    Ok(res) => error!("Open-Meteo API error for {}: {}", zone.name, res.status()),
                    Err(e) => error!("Request Error (Open-Meteo) for {}: {}", zone.name, e),
                }
            }
            
            // Respectful delay
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
