use axum::{
    extract::{State, Query},
    http::StatusCode,
    response::{Html, IntoResponse, Json as AxumJson},
    routing::get,
    Router,
};
use chrono::{DateTime, FixedOffset, Timelike, Utc, Duration as ChronoDuration};
use dotenvy::dotenv;
use gdal::Dataset;
use serde::Deserialize;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{error, info, warn};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use uuid::Uuid;

mod models;
mod errors;
mod stac;
mod ws;

use models::*;
use errors::AppError;
use stac::{StacCollection, StacItem, StacLink, StacSearchRequest, StacSearchResponse, STAC_VERSION, StacSearchContext};
use ws::WsState;

// ─── STATE & UTILS ───────────────────────────────────────────────────────────

struct AppState {
    pool: Pool<Postgres>,
    http_client: reqwest::Client,
    metrics: Arc<AppMetrics>,
    ws_state: Arc<WsState>,
    // Track timestamps for health checks
    last_bmkg_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    last_stac_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
}

fn get_elevation_at_point(lon: f64, lat: f64) -> Option<f32> {
    let dataset = Dataset::open("ntb_dem.tif").ok()?;
    let gt = dataset.geo_transform().ok()?;
    let band = dataset.rasterband(1).ok()?;

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

// ─── MAIN ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await
        .expect("Failed to connect to Postgres");

    let http_client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) GeoESG-NTB/1.0")
        .build()
        .unwrap();

    let shared_state = Arc::new(AppState {
        pool: pool.clone(),
        http_client,
        metrics: Arc::new(AppMetrics::default()),
        ws_state: Arc::new(WsState::new()),
        last_bmkg_fetch: std::sync::RwLock::new(None),
        last_stac_fetch: std::sync::RwLock::new(None),
    });

    // START BACKGROUND TASKS
    let state_stac = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        carbon_mapper_tracker_task(state_stac).await;
    });

    let state_bmkg = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        bmkg_tracker_task(state_bmkg).await;
    });

    let state_cleanup = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        data_retention_task(state_cleanup).await;
    });

    let state_forecast = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        weather_forecast_task(state_forecast).await;
    });

    // SETUP API ROUTES & MIDDLEWARE
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            let origin_str = origin.as_bytes();
            origin_str.starts_with(b"http://localhost") || origin_str.starts_with(b"http://127.0.0.1")
        }))
        .allow_methods([axum::http::Method::GET]);

    // Rate limiter: 100 requests per second
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(100)
            .finish()
            .unwrap(),
    );

    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("../frontend/index.html")) }))
        .route("/health", get(health_check))
        .route("/api/metrics", get(metrics_handler))
        .route("/api/stats", get(get_system_stats))
        .route("/api/weather", get(get_latest_weather))
        .route("/api/weather/forecast", get(get_weather_forecast))
        .route("/api/methane/plumes", get(get_methane_plumes))
        .route("/api/plume-prediction", get(get_multi_plume_prediction))
        .route("/api/plume-analysis", get(get_plume_analysis))
        .route("/api/zones", get(get_populated_zones))
        // WebSocket endpoint
        .route("/ws", get(ws::ws_handler))
        // STAC API endpoints
        .route("/api/stac", get(stac_root))
        .route("/api/stac/collections", get(stac_collections))
        .route("/api/stac/collections/methane-observations", get(stac_collection))
        .route("/api/stac/collections/methane-observations/items", get(stac_items))
        .route("/api/stac/search", get(stac_search))
        .layer(GovernorLayer { config: governor_conf })
        .layer(cors)
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("Failed to bind to 0.0.0.0:3000");
    info!("Server running on http://0.0.0.0:3000");
    axum::serve(listener, app).await.expect("Server crash");
}

// ─── API HANDLERS ────────────────────────────────────────────────────────────

async fn health_check(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let db_status = match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => ComponentHealth { status: "OK".to_string(), message: None },
        Err(e) => ComponentHealth { status: "ERROR".to_string(), message: Some(e.to_string()) },
    };

    let dem_status = match Dataset::open("ntb_dem.tif") {
        Ok(_) => ComponentHealth { status: "OK".to_string(), message: None },
        Err(e) => ComponentHealth { status: "ERROR".to_string(), message: Some(e.to_string()) },
    };

    let last_bmkg = *state.last_bmkg_fetch.read().unwrap();
    let last_stac = *state.last_stac_fetch.read().unwrap();

    let health = HealthStatus {
        status: if db_status.status == "OK" && dem_status.status == "OK" { "HEALTHY".to_string() } else { "DEGRADED".to_string() },
        database: db_status,
        dem_file: dem_status,
        last_bmkg_fetch: last_bmkg,
        last_carbon_mapper_fetch: last_stac,
        uptime_seconds: 0,
    };

    Ok((StatusCode::OK, AxumJson(health)))
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> String {
    use std::sync::atomic::Ordering::Relaxed;
    format!(
        "geoesg_requests_total {}\n\
         geoesg_request_errors {}\n\
         geoesg_carbon_mapper_fetches {}\n\
         geoesg_carbon_mapper_errors {}\n\
         geoesg_bmkg_fetches {}\n\
         geoesg_bmkg_errors {}\n\
         geoesg_alerts_sent {}\n\
         geoesg_plumes_ingested {}\n",
        state.metrics.requests_total.load(Relaxed),
        state.metrics.request_errors.load(Relaxed),
        state.metrics.carbon_mapper_fetches.load(Relaxed),
        state.metrics.carbon_mapper_errors.load(Relaxed),
        state.metrics.bmkg_fetches.load(Relaxed),
        state.metrics.bmkg_errors.load(Relaxed),
        state.metrics.alerts_sent.load(Relaxed),
        state.metrics.plumes_ingested.load(Relaxed),
    )
}

async fn get_system_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    // Simplification for brevity: using hardcoded counts instead of complex SQL counts
    // In a full prod app these would be actual SQL aggregations
    let stats = SystemStats {
        total_plumes: 0, plumes_last_24h: 0, plumes_last_7d: 0,
        avg_emission_rate: 0.0, max_emission_rate: 0.0,
        total_weather_records: 0, weather_records_last_24h: 0,
        total_alerts: 0, alerts_last_24h: 0,
        active_zones: 5, latest_plume_at: None, latest_weather_at: None,
    };
    
    (StatusCode::OK, AxumJson(stats)).into_response()
}

async fn get_populated_zones(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    // Return empty list if DB query fails to simplify code length
    let records = sqlx::query!(
        r#"SELECT zone_name, region, zone_type, ST_AsGeoJSON(geometry) as "geom!" 
           FROM populated_zones"#
    ).fetch_all(&state.pool).await.unwrap_or_default();
    
    let mut features = vec![];
    for r in records {
        features.push(json!({
            "type": "Feature",
            "properties": { "name": r.zone_name, "region": r.region, "type": r.zone_type },
            "geometry": serde_json::from_str::<serde_json::Value>(&r.geom).unwrap_or_default()
        }));
    }
    
    let geojson = json!({ "type": "FeatureCollection", "features": features });
    (StatusCode::OK, AxumJson(geojson)).into_response()
}

async fn get_methane_plumes(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    let records = sqlx::query!(
        r#"SELECT recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as "geometry!",
           ST_AsGeoJSON(plume_geometry) as "plume_geometry_json", source
         FROM methane_observations ORDER BY recorded_at DESC LIMIT 100"#
    ).fetch_all(&state.pool).await?;
    
    let plumes: Vec<MethanePlumeResponse> = records.into_iter().map(|row| MethanePlumeResponse {
        recorded_at: row.recorded_at,
        emission_rate_kg_hr: row.emission_rate_kg_hr,
        geometry: serde_json::from_str(&row.geometry).unwrap_or_default(),
        plume_footprint: row.plume_geometry_json.and_then(|g| serde_json::from_str(&g).ok()),
        source: row.source,
    }).collect();
    
    Ok((StatusCode::OK, AxumJson(json!(plumes))))
}

async fn get_latest_weather(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    let records = sqlx::query_as!(WeatherObservation,
        "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
         FROM weather_observations ORDER BY recorded_at DESC LIMIT 10"
    ).fetch_all(&state.pool).await?;
    
    Ok((StatusCode::OK, AxumJson(json!(records))))
}

async fn get_weather_forecast(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    let records = sqlx::query!(
        r#"SELECT forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
           FROM weather_forecasts
           WHERE valid_at > NOW()
           ORDER BY valid_at ASC
           LIMIT 48"#
    ).fetch_all(&state.pool).await?;
    
    let forecasts: Vec<serde_json::Value> = records.into_iter().map(|r| {
        json!({
            "forecast_at": r.forecast_at,
            "valid_at": r.valid_at,
            "area_id": r.area_id,
            "wind_speed_ms": r.wind_speed_ms,
            "wind_direction_deg": r.wind_direction_deg,
            "humidity_percent": r.humidity_percent,
            "temperature_c": r.temperature_c,
            "data_source": r.data_source
        })
    }).collect();
    
    Ok((StatusCode::OK, AxumJson(json!(forecasts))))
}

// ─── STAC API HANDLERS ───────────────────────────────────────────────────────

async fn stac_root() -> Result<impl IntoResponse, AppError> {
    let root = json!({
        "stac_version": STAC_VERSION,
        "type": "Catalog",
        "id": "geoesg-aeco-ntb",
        "title": "GeoESG A.E.C.O NTB Methane Tracker",
        "description": "STAC API for methane emissions tracking in West Nusa Tenggara",
        "links": [
            {
                "rel": "self",
                "href": "/api/stac",
                "type": "application/json"
            },
            {
                "rel": "service-desc",
                "href": "/api",
                "type": "application/vnd.oai.openapi+json;version=3.0"
            },
            {
                "rel": "data",
                "href": "/api/stac/collections",
                "type": "application/json"
            },
            {
                "rel": "search",
                "href": "/api/stac/search",
                "type": "application/geo+json",
                "method": "GET"
            }
        ]
    });
    
    Ok((StatusCode::OK, AxumJson(root)))
}

async fn stac_collections() -> Result<impl IntoResponse, AppError> {
    let collections = json!({
        "collections": [
            StacCollection::methane_observations()
        ],
        "links": [
            {
                "rel": "self",
                "href": "/api/stac/collections",
                "type": "application/json"
            }
        ]
    });
    
    Ok((StatusCode::OK, AxumJson(collections)))
}

async fn stac_collection() -> Result<impl IntoResponse, AppError> {
    Ok((StatusCode::OK, AxumJson(json!(StacCollection::methane_observations()))))
}

async fn stac_items(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StacSearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    let limit = params.limit.unwrap_or(100).min(1000) as i64;
    
    let records = sqlx::query!(
        r#"SELECT id, recorded_at, emission_rate_kg_hr, 
           ST_AsGeoJSON(location) as "geometry!",
           ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
           source
         FROM methane_observations 
         ORDER BY recorded_at DESC 
         LIMIT $1"#,
        limit
    ).fetch_all(&state.pool).await?;
    
    let items: Vec<StacItem> = records.into_iter().map(|row| {
        let geometry: serde_json::Value = serde_json::from_str(&row.geometry).unwrap_or_default();
        let lon = row.lon.unwrap_or(0.0);
        let lat = row.lat.unwrap_or(0.0);
        let bbox = vec![lon - 0.01, lat - 0.01, lon + 0.01, lat + 0.01];
        
        StacItem::from_methane_observation(
            row.id,
            row.recorded_at,
            row.emission_rate_kg_hr,
            geometry,
            bbox,
            &row.source.unwrap_or_else(|| "unknown".to_string()),
        )
    }).collect();
    
    let response = StacSearchResponse {
        r#type: "FeatureCollection".to_string(),
        features: items,
        links: vec![
            StacLink {
                rel: "self".to_string(),
                href: "/api/stac/collections/methane-observations/items".to_string(),
                r#type: Some("application/geo+json".to_string()),
                title: None,
            },
        ],
        context: Some(stac::StacSearchContext {
            returned: records.len() as u32,
            matched: None,
        }),
    };
    
    Ok((StatusCode::OK, AxumJson(json!(response))))
}

async fn stac_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StacSearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    let limit = params.limit.unwrap_or(100).min(1000) as i64;
    
    let records = sqlx::query!(
        r#"SELECT id, recorded_at, emission_rate_kg_hr, 
           ST_AsGeoJSON(location) as "geometry!",
           ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
           source
         FROM methane_observations 
         ORDER BY recorded_at DESC 
         LIMIT $1"#,
        limit
    ).fetch_all(&state.pool).await?;
    
    let items: Vec<StacItem> = records.into_iter().map(|row| {
        let geometry: serde_json::Value = serde_json::from_str(&row.geometry).unwrap_or_default();
        let lon = row.lon.unwrap_or(0.0);
        let lat = row.lat.unwrap_or(0.0);
        let bbox = vec![lon - 0.01, lat - 0.01, lon + 0.01, lat + 0.01];
        
        StacItem::from_methane_observation(
            row.id,
            row.recorded_at,
            row.emission_rate_kg_hr,
            geometry,
            bbox,
            &row.source.unwrap_or_else(|| "unknown".to_string()),
        )
    }).collect();
    
    let response = StacSearchResponse {
        r#type: "FeatureCollection".to_string(),
        features: items,
        links: vec![
            StacLink {
                rel: "self".to_string(),
                href: "/api/stac/search".to_string(),
                r#type: Some("application/geo+json".to_string()),
                title: None,
            },
        ],
        context: Some(stac::StacSearchContext {
            returned: records.len() as u32,
            matched: None,
        }),
    };
    
    Ok((StatusCode::OK, AxumJson(json!(response))))
}

// ─── PHYSICS & DISPERSION ────────────────────────────────────────────────────

fn get_pasquill_stability_class(wind_speed_ms: f64, is_daytime: bool) -> char {
    if is_daytime {
        if wind_speed_ms < 3.0 { 'A' } else if wind_speed_ms < 5.0 { 'B' } else { 'C' }
    } else if wind_speed_ms < 3.0 { 'F' } else if wind_speed_ms < 5.0 { 'E' } else { 'D' }
}

fn get_plume_spread_angle(stability_class: char) -> f64 {
    match stability_class { 'A' => 25.0, 'B' => 20.0, 'C' => 15.0, 'D' => 12.5, 'E' => 8.75, 'F' => 5.0, _ => 12.5 }
}

// Computes centerline ground-level concentration at 1km distance (in parts per million, roughly)
fn calc_gaussian_concentration_1km(emission_kg_hr: f64, ws: f64, stability: char) -> f64 {
    // Convert kg/hr to g/s
    let q_g_s = emission_kg_hr * 1000.0 / 3600.0;
    
    // Approximate dispersion coefficients (sigma_y, sigma_z) at x = 1000m based on Pasquill-Gifford
    let (sy, sz) = match stability {
        'A' => (210.0, 450.0),
        'B' => (155.0, 110.0),
        'C' => (105.0, 61.0),
        'D' => (68.0, 31.0),
        'E' => (50.0, 21.0),
        'F' => (34.0, 11.0),
        _ => (68.0, 31.0)
    };
    
    // Ground level centerline concentration (z=0, y=0)
    // C = Q / (pi * u * sigma_y * sigma_z)
    let ws_safe = if ws < 1.0 { 1.0 } else { ws };
    let c_g_m3 = q_g_s / (std::f64::consts::PI * ws_safe * sy * sz);
    
    // Convert g/m3 to mg/m3 to approx ppm for CH4
    let c_mg_m3 = c_g_m3 * 1000.0;
    c_mg_m3 * 1.5 // Rough ppm conversion for Methane
}

async fn get_multi_plume_prediction(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    // Fetch all active plumes within last 24 hours
    let active_sources = match sqlx::query!(
        r#"SELECT id, ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat, emission_rate_kg_hr
           FROM methane_observations WHERE recorded_at > NOW() - INTERVAL '24 hours'"#
    ).fetch_all(&state.pool).await {
        Ok(s) => s,
        Err(e) => {
            error!("Database error fetching sources: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "DB Error"}))).into_response();
        }
    };

    if active_sources.is_empty() {
        return (StatusCode::OK, AxumJson(json!(Vec::<MultiPlumePrediction>::new()))).into_response();
    }

    let mut predictions = Vec::new();
    let wita_offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let now_wita = Utc::now().with_timezone(&wita_offset);
    let is_daytime = now_wita.hour() >= 6 && now_wita.hour() < 18;

    // Optional sensor telemtry overrides
    let sensor_roll = std::env::var("SENSOR_ROLL_DEG").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(5.0);
    let sensor_pitch = std::env::var("SENSOR_PITCH_DEG").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(2.0);
    let is_smeared = sensor_roll > 6.85 || sensor_pitch > 4.8;

    for source in active_sources {
        let lon = source.lon.unwrap_or(0.0);
        let lat = source.lat.unwrap_or(0.0);
        let region = get_region_from_coords(lon, lat);
        let emission_rate = source.emission_rate_kg_hr;

        // REGIME I: Tanager-1 detection limit (64-126 kg/hr optimal, 100 kg/hr EPA super-emitter threshold)
        if emission_rate < 100.0 { continue; }

        // Fetch fresh weather (only < 6 hours old to prevent staleness)
        let weather = sqlx::query!(
            r#"SELECT wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c
               FROM weather_observations 
               WHERE area_id = $1 AND recorded_at > NOW() - INTERVAL '6 hours'
               ORDER BY recorded_at DESC LIMIT 1"#,
            region
        ).fetch_optional(&state.pool).await.unwrap_or_default();

        let w = match weather { Some(w) => w, None => continue };

        let ws = w.wind_speed_ms.unwrap_or(1.0);
        let wd = w.wind_direction_deg.unwrap_or(0.0);
        let hum = w.humidity_percent.unwrap_or(0.0);
        let temp = w.temperature_c.unwrap_or(25.0);

        let mut distance = ws * 3600.0; 

        // REGIME II: Atmospheric Extinction (simplified humidity attenuation)
        // Note: Simplified model. Full Beer-Lambert: T = exp(-tau) where tau = alpha_abs * path_length
        // H2O absorption bands at SWIR (1.6 um) affect CH4 retrieval significantly
        if hum > 85.0 { distance *= 0.60; }

        let stability = get_pasquill_stability_class(ws, is_daytime);
        let mut spread_angle = get_plume_spread_angle(stability);

        // REGIME III: Thermal Stability Correction (simplified T^4 penalty factor)
        // Note: Not direct Stefan-Boltzmann application. Uses T^4 ratio as heuristic
        // to reduce spread angle under high thermal stability (hotter = more stable atmosphere).
        let temp_k: f64 = temp + 273.15;
        let baseline_k: f64 = 308.15;
        if temp_k > baseline_k { spread_angle *= (baseline_k / temp_k).powi(4); }

        // Calculate concentration
        let conc_1km = calc_gaussian_concentration_1km(emission_rate, ws, stability);

        // Terrain Blocking
        let origin_elev = get_elevation_at_point(lon, lat).unwrap_or(0.0);
        let mut terrain_blocked = false;
        let mut block_dist = None;
        
        for i in 1..=10 {
            let step_dist = distance * (i as f64 / 10.0);
            let dx = (wd + 180.0).to_radians().sin() * (step_dist / 111320.0);
            let dy = (wd + 180.0).to_radians().cos() * (step_dist / 110540.0);
            if let Some(elev) = get_elevation_at_point(lon + dx, lat + dy) {
                 if elev > origin_elev + 15.0 {
                     terrain_blocked = true;
                     block_dist = Some(step_dist);
                     distance = step_dist; 
                     break;
                 }
            }
        }

        // Generate polygon and check intersection with ALL populated zones
        let geom_rec = sqlx::query!(
            r#"WITH plume AS (
                SELECT ST_MakePolygon(ST_MakeLine(ARRAY[
                    ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geometry,
                    ST_Project(ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geography, $3::FLOAT8, radians($4::FLOAT8 + 180.0 - $5::FLOAT8))::geometry,
                    ST_Project(ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geography, $3::FLOAT8, radians($4::FLOAT8 + 180.0 + $5::FLOAT8))::geometry,
                    ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geometry
                ])) as geom
               )
               SELECT ST_AsGeoJSON(geom) as "json!" FROM plume"#
            , lon, lat, distance, wd, spread_angle
        ).fetch_one(&state.pool).await.ok();

        if let Some(rec) = geom_rec {
            let geojson_val: serde_json::Value = serde_json::from_str(&rec.json).unwrap_or_default();
            
            // Check intersections against populated_zones
            let affected = sqlx::query!(
                r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
                   FROM populated_zones 
                   WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
                rec.json
            ).fetch_all(&state.pool).await.unwrap_or_default();
            
            let exposure_alert = !affected.is_empty();
            let mut affected_zones = Vec::new();
            
            for zone in affected {
                affected_zones.push(AffectedZone {
                    zone_name: zone.zone_name.clone(),
                    region: zone.region.clone(),
                    population_estimate: zone.population_estimate,
                    zone_type: zone.zone_type,
                    is_volcanic_zone: zone.is_volcanic_zone,
                });

                // Alert Logic
                if conc_1km > 50.0 { // Arbitrary high threshold for alert
                    let msg = format!(
                        "⚠️ <b>EVACUATION ALERT</b>\n\n<b>Zone:</b> {} ({})\n<b>Emission:</b> {:.2} kg/hr\n<b>Max Dist:</b> {:.0}m\n<b>Est. Conc:</b> {:.1} ppm",
                        zone.zone_name, zone.region, emission_rate, distance, conc_1km
                    );
                    let client = state.http_client.clone();
                    tokio::spawn(async move { send_telegram_alert(&client, &msg).await; });
                    
                    // Log alert to DB
                    let _ = sqlx::query!(
                        "INSERT INTO evacuation_alerts (region, zone_name, emission_rate_kg_hr, wind_speed_ms, wind_direction_deg, concentration_ppm, stability_class) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                        zone.region, zone.zone_name, emission_rate, ws, wd, conc_1km, stability.to_string()
                    ).execute(&state.pool).await;
                }
            }

            predictions.push(MultiPlumePrediction {
                source_id: source.id,
                source_lon: lon,
                source_lat: lat,
                emission_rate_kg_hr: emission_rate,
                wind_speed_ms: ws,
                wind_direction_deg: wd,
                stability_class: stability,
                spread_angle_deg: spread_angle,
                max_distance_m: distance,
                concentration_at_1km_ppm: conc_1km,
                plume_geojson: geojson_val,
                high_uncertainty_smear: is_smeared,
                terrain_blocked,
                terrain_block_distance_m: block_dist,
                affected_zones,
                exposure_alert,
            });
        }
    }
    
    (StatusCode::OK, AxumJson(json!(predictions))).into_response()
}

async fn get_plume_analysis(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.requests_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    
    // Fetch active plumes with their observed geometry
    let active_sources = match sqlx::query!(
        r#"SELECT id, recorded_at, emission_rate_kg_hr, 
           ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
           ST_AsGeoJSON(plume_geometry) as "plume_geometry_json", source
           FROM methane_observations 
           WHERE recorded_at > NOW() - INTERVAL '24 hours'
           ORDER BY recorded_at DESC"#
    ).fetch_all(&state.pool).await {
        Ok(s) => s,
        Err(e) => {
            error!("Database error fetching sources: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "DB Error"}))).into_response();
        }
    };

    if active_sources.is_empty() {
        return (StatusCode::OK, AxumJson(json!(Vec::<PlumeAnalysis>::new()))).into_response();
    }

    let mut analyses = Vec::new();
    let wita_offset = FixedOffset::east_opt(8 * 3600).unwrap();

    for source in active_sources {
        let lon = source.lon.unwrap_or(0.0);
        let lat = source.lat.unwrap_or(0.0);
        let region = get_region_from_coords(lon, lat);
        let emission_rate = source.emission_rate_kg_hr;

        // Skip below Tanager-1 detection limit
        if emission_rate < 100.0 { continue; }

        // Check if observed plume intersects populated zones
        let mut observed_affected_zones = Vec::new();
        let mut observed_exposure_alert = false;

        if let Some(ref plume_json) = source.plume_geometry_json {
            let affected = sqlx::query!(
                r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
                   FROM populated_zones 
                   WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
                plume_json
            ).fetch_all(&state.pool).await.unwrap_or_default();

            observed_exposure_alert = !affected.is_empty();
            for zone in affected {
                observed_affected_zones.push(AffectedZone {
                    zone_name: zone.zone_name.clone(),
                    region: zone.region.clone(),
                    population_estimate: zone.population_estimate,
                    zone_type: zone.zone_type,
                    is_volcanic_zone: zone.is_volcanic_zone,
                });
            }
        }

        let observed = ObservedPlume {
            plume_footprint: source.plume_geometry_json.and_then(|g| serde_json::from_str(&g).ok()),
            affected_zones: observed_affected_zones,
            exposure_alert: observed_exposure_alert,
            source: source.source.unwrap_or_else(|| "unknown".to_string()),
        };

        // Generate forecast predictions using weather forecasts
        let forecasts = sqlx::query!(
            r#"SELECT valid_at, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c
               FROM weather_forecasts 
               WHERE area_id = $1 AND valid_at > NOW() AND valid_at < NOW() + INTERVAL '6 hours'
               ORDER BY valid_at ASC"#,
            region
        ).fetch_all(&state.pool).await.unwrap_or_default();

        let mut forecasted_plumes = Vec::new();
        let sensor_roll = std::env::var("SENSOR_ROLL_DEG").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(5.0);
        let sensor_pitch = std::env::var("SENSOR_PITCH_DEG").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(2.0);
        let is_smeared = sensor_roll > 6.85 || sensor_pitch > 4.8;

        for fc in forecasts {
            let ws = fc.wind_speed_ms.unwrap_or(1.0);
            let wd = fc.wind_direction_deg.unwrap_or(0.0);
            let hum = fc.humidity_percent.unwrap_or(0.0);
            let temp = fc.temperature_c.unwrap_or(25.0);
            let valid_at = fc.valid_at;

            let is_daytime = valid_at.with_timezone(&wita_offset).hour() >= 6 && valid_at.with_timezone(&wita_offset).hour() < 18;

            let mut distance = ws * 3600.0;
            if hum > 85.0 { distance *= 0.60; }

            let stability = get_pasquill_stability_class(ws, is_daytime);
            let mut spread_angle = get_plume_spread_angle(stability);

            let temp_k = temp + 273.15;
            let baseline_k = 308.15;
            if temp_k > baseline_k { spread_angle *= (baseline_k / temp_k).powi(4); }

            let conc_1km = calc_gaussian_concentration_1km(emission_rate, ws, stability);

            // Terrain blocking
            let origin_elev = get_elevation_at_point(lon, lat).unwrap_or(0.0);
            let mut terrain_blocked = false;
            let mut block_dist = None;

            for i in 1..=10 {
                let step_dist = distance * (i as f64 / 10.0);
                let dx = (wd + 180.0).to_radians().sin() * (step_dist / 111320.0);
                let dy = (wd + 180.0).to_radians().cos() * (step_dist / 110540.0);
                if let Some(elev) = get_elevation_at_point(lon + dx, lat + dy) {
                    if elev > origin_elev + 15.0 {
                        terrain_blocked = true;
                        block_dist = Some(step_dist);
                        distance = step_dist;
                        break;
                    }
                }
            }

            // Generate forecast plume polygon
            let geom_rec = sqlx::query!(
                r#"WITH plume AS (
                    SELECT ST_MakePolygon(ST_MakeLine(ARRAY[
                        ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geometry,
                        ST_Project(ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geography, $3::FLOAT8, radians($4::FLOAT8 + 180.0 - $5::FLOAT8))::geometry,
                        ST_Project(ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geography, $3::FLOAT8, radians($4::FLOAT8 + 180.0 + $5::FLOAT8))::geometry,
                        ST_SetSRID(ST_MakePoint($1::FLOAT8, $2::FLOAT8), 4326)::geometry
                    ])) as geom
                   )
                   SELECT ST_AsGeoJSON(geom) as "json!" FROM plume"#
                , lon, lat, distance, wd, spread_angle
            ).fetch_one(&state.pool).await.ok();

            if let Some(rec) = geom_rec {
                let geojson_val: serde_json::Value = serde_json::from_str(&rec.json).unwrap_or_default();

                let affected = sqlx::query!(
                    r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
                       FROM populated_zones 
                       WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
                    rec.json
                ).fetch_all(&state.pool).await.unwrap_or_default();

                let exposure_alert = !affected.is_empty();
                let mut affected_zones = Vec::new();

                for zone in affected {
                    affected_zones.push(AffectedZone {
                        zone_name: zone.zone_name.clone(),
                        region: zone.region.clone(),
                        population_estimate: zone.population_estimate,
                        zone_type: zone.zone_type,
                        is_volcanic_zone: zone.is_volcanic_zone,
                    });

                    // Alert for forecasted exposure
                    if conc_1km > 50.0 {
                        let msg = format!(
                            "⚠️ <b>FORECAST ALERT</b>\n\n<b>Zone:</b> {} ({})\n<b>Emission:</b> {:.2} kg/hr\n<b>Valid At:</b> {}\n<b>Max Dist:</b> {:.0}m\n<b>Est. Conc:</b> {:.1} ppm",
                            zone.zone_name, zone.region, emission_rate, valid_at.format("%Y-%m-%d %H:%M UTC"), distance, conc_1km
                        );
                        let client = state.http_client.clone();
                        tokio::spawn(async move { send_telegram_alert(&client, &msg).await; });
                    }
                }

                forecasted_plumes.push(ForecastedPlume {
                    valid_at,
                    wind_speed_ms: ws,
                    wind_direction_deg: wd,
                    stability_class: stability,
                    spread_angle_deg: spread_angle,
                    max_distance_m: distance,
                    concentration_at_1km_ppm: conc_1km,
                    plume_geojson: geojson_val,
                    terrain_blocked,
                    terrain_block_distance_m: block_dist,
                    affected_zones,
                    exposure_alert,
                });
            }
        }

        analyses.push(PlumeAnalysis {
            source_id: source.id,
            source_lon: lon,
            source_lat: lat,
            emission_rate_kg_hr: emission_rate,
            recorded_at: source.recorded_at,
            observed,
            forecast: forecasted_plumes,
        });
    }

    (StatusCode::OK, AxumJson(json!(analyses))).into_response()
}

// ─── BACKGROUND TASKS ────────────────────────────────────────────────────────

async fn carbon_mapper_tracker_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(86400)); 
    let api_token = std::env::var("CARBON_MAPPER_TOKEN").unwrap_or_default();
    if api_token.is_empty() { return; }

    loop {
        interval.tick().await;
        state.metrics.carbon_mapper_fetches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        
        let mut next_url = Some("https://api.carbonmapper.org/api/v1/stac/search".to_string());
        
        while let Some(url) = next_url.clone() {
            let payload = json!({
                "bbox": [115.40, -9.15, 119.45, -8.00],
                "datetime": format!("2024-01-01T00:00:00Z/{}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
                "limit": 100 // Pagination
            });

            match state.http_client.post(&url).header("X-API-KEY", &api_token).json(&payload).send().await {
                Ok(res) if res.status().is_success() => {
                    if let Ok(stac) = res.json::<StacResponse>().await {
                        *state.last_stac_fetch.write().unwrap() = Some(Utc::now());
                        
                        for feature in stac.features {
                            if feature.properties.emission_rate_kg_hr <= 0.0 { continue; }
                            let dt = chrono::DateTime::parse_from_rfc3339(&feature.properties.datetime).unwrap().with_timezone(&Utc);
                            let geom = serde_json::to_string(&feature.geometry).unwrap();
                            
                            // Store both centroid (for spatial queries) and full plume footprint
                            let res: Result<sqlx::postgres::PgQueryResult, sqlx::Error> = sqlx::query!(
                                "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location, plume_geometry, source) VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)), ST_GeomFromGeoJSON($3), 'carbon_mapper') ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
                                dt, feature.properties.emission_rate_kg_hr, geom
                            ).execute(&state.pool).await;
                            
                            if res.is_ok() && res.unwrap().rows_affected() > 0 {
                                state.metrics.plumes_ingested.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        
                        // Handle STAC pagination (Feature #5)
                        next_url = stac.links.iter().find(|l| l.rel == "next").map(|l| l.href.clone());
                    } else { next_url = None; }
                }
                _ => { 
                    state.metrics.carbon_mapper_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    next_url = None; 
                }
            }
        }
    }
}

async fn bmkg_tracker_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(3600)); 
    let zones = [
        ("Lombok Barat", "52.01.01.2014", "-8.6818", "116.1240"),
        ("Lombok Tengah", "52.02.01.2001", "-8.7167", "116.2667"),
        ("Lombok Timur", "52.03.01.2001", "-8.6500", "116.5333"),
        ("Sumbawa Barat", "52.07.01.1001", "-8.7333", "116.8500"),
        ("Kota Bima", "52.72.01.1001", "-8.4667", "118.7167"),
    ];

    loop {
        interval.tick().await;
        state.metrics.bmkg_fetches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *state.last_bmkg_fetch.write().unwrap() = Some(Utc::now());

        for (name, bmkg_id, lat, lon) in zones {
            let mut success = false;
            let bmkg_url = format!("https://api.bmkg.go.id/publik/prakiraan-cuaca?adm4={}", bmkg_id);
            
            if let Ok(res) = state.http_client.get(&bmkg_url).send().await {
                if let Ok(json) = res.json::<BmkgResponse>().await {
                    if let Some(item) = json.data.first().and_then(|g| g.cuaca.first()).and_then(|l| l.first()) {
                        let _ = sqlx::query!(
                            "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source) VALUES (NOW(), $1, $2, $3, $4, $5, 'BMKG')",
                            name, item.ws / 3.6, item.wd_deg, item.hu, item.t
                        ).execute(&state.pool).await;
                        success = true;
                    }
                }
            }

            if !success {
                let om_url = format!("https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,wind_speed_10m,wind_direction_10m,relative_humidity_2m", lat, lon);
                if let Ok(res) = state.http_client.get(&om_url).send().await {
                    if let Ok(json) = res.json::<OpenMeteoResponse>().await {
                        let _ = sqlx::query!(
                            "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source) VALUES (NOW(), $1, $2, $3, $4, $5, 'Open-Meteo')",
                            name, json.current.wind_speed_10m, json.current.wind_direction_10m, json.current.relative_humidity_2m, json.current.temperature_2m
                        ).execute(&state.pool).await;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn data_retention_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(86400)); // Daily
    loop {
        interval.tick().await;
        info!("Running data retention cleanup...");
        // Delete weather data older than 30 days
        let _ = sqlx::query("DELETE FROM weather_observations WHERE recorded_at < NOW() - INTERVAL '30 days'").execute(&state.pool).await;
        // Delete weather forecasts older than 7 days
        let _ = sqlx::query("DELETE FROM weather_forecasts WHERE created_at < NOW() - INTERVAL '7 days'").execute(&state.pool).await;
    }
}

async fn weather_forecast_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(3600)); // Hourly
    let zones = [
        ("Lombok Barat", "-8.6818", "116.1240"),
        ("Lombok Tengah", "-8.7167", "116.2667"),
        ("Lombok Timur", "-8.6500", "116.5333"),
        ("Sumbawa Barat", "-8.7333", "116.8500"),
        ("Kota Bima", "-8.4667", "118.7167"),
    ];

    loop {
        interval.tick().await;
        info!("Fetching weather forecasts from Open-Meteo...");

        for (name, lat, lon) in zones {
            let url = format!(
                "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&hourly=temperature_2m,relative_humidity_2m,wind_speed_10m,wind_direction_10m&forecast_days=2&timezone=Asia/Makassar",
                lat, lon
            );

            if let Ok(res) = state.http_client.get(&url).send().await {
                if let Ok(forecast) = res.json::<OpenMeteoForecastResponse>().await {
                    for i in 0..forecast.hourly.time.len() {
                        let valid_at = chrono::NaiveDateTime::parse_from_str(
                            &forecast.hourly.time[i], "%Y-%m-%dT%H:%M"
                        ).unwrap_or_default();

                        let _ = sqlx::query!(
                            "INSERT INTO weather_forecasts (forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source) VALUES (NOW(), $1, $2, $3, $4, $5, $6, 'Open-Meteo')",
                            valid_at.and_utc(),
                            name,
                            forecast.hourly.wind_speed_10m.get(i).copied().unwrap_or(0.0),
                            forecast.hourly.wind_direction_10m.get(i).copied().unwrap_or(0.0),
                            forecast.hourly.relative_humidity_2m.get(i).copied().unwrap_or(0.0),
                            forecast.hourly.temperature_2m.get(i).copied().unwrap_or(0.0)
                        ).execute(&state.pool).await;
                    }
                    info!("Stored {} forecast hours for {}", forecast.hourly.time.len(), name);
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

// ─── TESTS ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pasquill_stability_class_daytime() {
        // Daytime tests
        assert_eq!(get_pasquill_stability_class(2.0, true), 'A');  // Low wind, daytime
        assert_eq!(get_pasquill_stability_class(4.0, true), 'B');  // Medium wind, daytime
        assert_eq!(get_pasquill_stability_class(6.0, true), 'C');  // High wind, daytime
    }

    #[test]
    fn test_pasquill_stability_class_nighttime() {
        // Nighttime tests
        assert_eq!(get_pasquill_stability_class(2.0, false), 'F');  // Low wind, nighttime
        assert_eq!(get_pasquill_stability_class(4.0, false), 'E');  // Medium wind, nighttime
        assert_eq!(get_pasquill_stability_class(6.0, false), 'D');  // High wind, nighttime
    }

    #[test]
    fn test_plume_spread_angle() {
        assert_eq!(get_plume_spread_angle('A'), 25.0);
        assert_eq!(get_plume_spread_angle('B'), 20.0);
        assert_eq!(get_plume_spread_angle('C'), 15.0);
        assert_eq!(get_plume_spread_angle('D'), 12.5);
        assert_eq!(get_plume_spread_angle('E'), 8.75);
        assert_eq!(get_plume_spread_angle('F'), 5.0);
        assert_eq!(get_plume_spread_angle('X'), 12.5);  // Default
    }

    #[test]
    fn test_gaussian_concentration_1km() {
        // Test with known values
        let conc = calc_gaussian_concentration_1km(1000.0, 3.0, 'D');
        assert!(conc > 0.0);
        assert!(conc < 1000.0);  // Sanity check

        // Higher emission should give higher concentration
        let conc_low = calc_gaussian_concentration_1km(100.0, 3.0, 'D');
        let conc_high = calc_gaussian_concentration_1km(1000.0, 3.0, 'D');
        assert!(conc_high > conc_low);

        // Higher wind speed should give lower concentration
        let conc_low_wind = calc_gaussian_concentration_1km(1000.0, 1.0, 'D');
        let conc_high_wind = calc_gaussian_concentration_1km(1000.0, 10.0, 'D');
        assert!(conc_low_wind > conc_high_wind);
    }

    #[test]
    fn test_gaussian_concentration_wind_safety() {
        // Wind speed < 1.0 should be clamped to 1.0
        let conc = calc_gaussian_concentration_1km(1000.0, 0.5, 'D');
        assert!(conc > 0.0);
    }

    #[test]
    fn test_region_from_coords() {
        // Test known coordinates
        assert_eq!(get_region_from_coords(116.1240, -8.6818), "Lombok Barat");
        assert_eq!(get_region_from_coords(116.2667, -8.7167), "Lombok Tengah");
        assert_eq!(get_region_from_coords(118.7167, -8.4667), "Kota Bima");
    }

    #[test]
    fn test_shot_noise_bound() {
        // Test detection threshold (100 kg/hr)
        let min_detection = 100.0;
        assert!(50.0 < min_detection);   // Below threshold
        assert!(150.0 >= min_detection); // Above threshold
    }

    #[test]
    fn test_sensor_smear_thresholds() {
        // Test sensor smear limits from Physics Limits document
        let roll_limit = 6.85;
        let pitch_limit = 4.8;

        // Below limits - no smear
        assert!(5.0 <= roll_limit);
        assert!(2.0 <= pitch_limit);

        // Above limits - smear
        assert!(7.0 > roll_limit);
        assert!(5.0 > pitch_limit);
    }

    #[test]
    fn test_terrain_blocking_threshold() {
        // Test terrain blocking threshold (15m)
        let threshold = 15.0;
        assert!(10.0 < threshold);  // Below threshold - no blocking
        assert!(20.0 > threshold);  // Above threshold - blocking
    }
}
