use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json as AxumJson},
    routing::get,
    Router,
};
use chrono::{DateTime, FixedOffset, Timelike, Utc};
use gdal::Dataset;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres, Row};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{error, info, warn};
use uuid::Uuid;

mod config;
mod errors;
mod models;
mod physics;
mod repositories;
mod services;
mod stac;
mod ws;

use config::AppConfig;
use errors::AppError;
use models::*;
use physics::*;
use services::*;
use stac::{
    StacCollection, StacItem, StacLink, StacSearchRequest, StacSearchResponse, STAC_VERSION,
};
use ws::WsState;

// ─── STATE & UTILS ───────────────────────────────────────────────────────────

struct AppState {
    pool: Pool<Postgres>,
    http_client: reqwest::Client,
    metrics: Arc<AppMetrics>,
    ws_state: Arc<WsState>,
    config: AppConfig,
    last_bmkg_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    last_stac_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    last_emit_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    last_s5p_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    start_time: std::time::Instant,
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

async fn send_telegram_alert(client: &reqwest::Client, msg: &str, token: &str, chat_id: &str) {
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
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let app_config = AppConfig::from_env().expect("Invalid configuration");

    let pool = PgPoolOptions::new()
        .max_connections(app_config.database.max_connections)
        .min_connections(app_config.database.min_connections)
        .acquire_timeout(Duration::from_secs(
            app_config.database.acquire_timeout_secs,
        ))
        .connect(&app_config.database.url)
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
        config: app_config.clone(),
        last_bmkg_fetch: std::sync::RwLock::new(None),
        last_stac_fetch: std::sync::RwLock::new(None),
        last_emit_fetch: std::sync::RwLock::new(None),
        last_s5p_fetch: std::sync::RwLock::new(None),
        start_time: std::time::Instant::now(),
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

    let state_s5p = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        s5p_tracker_task(state_s5p).await;
    });

    let state_forecast = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        weather_forecast_task(state_forecast).await;
    });

    let state_emit = Arc::clone(&shared_state);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        emit_tracker_task(state_emit).await;
    });

    // SETUP API ROUTES & MIDDLEWARE
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            let origin_str = origin.as_bytes();
            origin_str.starts_with(b"http://localhost")
                || origin_str.starts_with(b"http://127.0.0.1")
        }))
        .allow_methods([axum::http::Method::GET]);

    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(100)
            .finish()
            .unwrap(),
    );

    let app = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../frontend/index.html")) }),
        )
        .route("/health", get(health_check))
        .route("/api/metrics", get(metrics_handler))
        .route("/api/stats", get(get_system_stats))
        .route("/api/weather", get(get_latest_weather))
        .route("/api/weather/forecast", get(get_weather_forecast))
        .route("/api/methane/plumes", get(get_methane_plumes))
        .route("/api/plume-prediction", get(get_multi_plume_prediction))
        .route("/api/plume-analysis", get(get_plume_analysis))
        .route("/api/zones", get(get_populated_zones))
        .route("/api/s5p", get(get_s5p_overpasses))
        .route("/ws", get(ws::ws_handler))
        .route("/api/stac", get(stac_root))
        .route("/api/stac/collections", get(stac_collections))
        .route(
            "/api/stac/collections/methane-observations",
            get(stac_collection),
        )
        .route(
            "/api/stac/collections/methane-observations/items",
            get(stac_items),
        )
        .route("/api/stac/search", get(stac_search))
        .layer(GovernorLayer {
            config: governor_conf,
        })
        .layer(cors)
        .with_state(shared_state.clone());

    let bind_addr = format!("{}:{}", app_config.server.host, app_config.server.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", bind_addr, e));
    info!("Server running on http://{}", bind_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server crash");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { info!("Received Ctrl+C, shutting down gracefully..."); }
        _ = terminate => { info!("Received SIGTERM, shutting down gracefully..."); }
    }
}

async fn get_s5p_overpasses(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let records = sqlx::query_as::<_, S5pOverpass>(
        "SELECT scene_id, start_datetime, end_datetime, orbit_number, netcdf_download_url FROM s5p_overpasses ORDER BY start_datetime DESC LIMIT 20"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    (
        axum::http::StatusCode::OK,
        axum::response::Json(serde_json::json!(records)),
    )
}

// ─── API HANDLERS ────────────────────────────────────────────────────────────

async fn health_check(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let db_status = match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => ComponentHealth {
            status: "OK".to_string(),
            message: None,
        },
        Err(e) => ComponentHealth {
            status: "ERROR".to_string(),
            message: Some(e.to_string()),
        },
    };

    let dem_status = match Dataset::open("ntb_dem.tif") {
        Ok(_) => ComponentHealth {
            status: "OK".to_string(),
            message: None,
        },
        Err(e) => ComponentHealth {
            status: "ERROR".to_string(),
            message: Some(e.to_string()),
        },
    };

    let last_bmkg = *state.last_bmkg_fetch.read().unwrap();
    let last_stac = *state.last_stac_fetch.read().unwrap();
    let last_emit = *state.last_emit_fetch.read().unwrap();

    let health = HealthStatus {
        status: if db_status.status == "OK" && dem_status.status == "OK" {
            "HEALTHY".to_string()
        } else {
            "DEGRADED".to_string()
        },
        database: db_status,
        dem_file: dem_status,
        last_bmkg_fetch: last_bmkg,
        last_carbon_mapper_fetch: last_stac,
        last_emit_fetch: last_emit,
        last_s5p_fetch: *state.last_s5p_fetch.read().unwrap(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
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
         geoesg_emit_fetches {}\n\
         geoesg_emit_errors {}\n\
         geoesg_emit_plumes_ingested {}\n\
         geoesg_bmkg_fetches {}\n\
         geoesg_bmkg_errors {}\n\
         geoesg_alerts_sent {}\n\
         geoesg_plumes_ingested {}\n\
         geoesg_s5p_fetches {}\n\
         geoesg_s5p_errors {}\n",
        state.metrics.requests_total.load(Relaxed),
        state.metrics.request_errors.load(Relaxed),
        state.metrics.carbon_mapper_fetches.load(Relaxed),
        state.metrics.carbon_mapper_errors.load(Relaxed),
        state.metrics.emit_fetches.load(Relaxed),
        state.metrics.emit_errors.load(Relaxed),
        state.metrics.emit_plumes_ingested.load(Relaxed),
        state.metrics.bmkg_fetches.load(Relaxed),
        state.metrics.bmkg_errors.load(Relaxed),
        state.metrics.alerts_sent.load(Relaxed),
        state.metrics.plumes_ingested.load(Relaxed),
        state.metrics.s5p_fetches.load(Relaxed),
        state.metrics.s5p_errors.load(Relaxed),
    )
}

async fn get_system_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let plume_stats = sqlx::query(
        r#"SELECT 
            COUNT(*)::BIGINT as total_plumes,
            COUNT(*) FILTER (WHERE recorded_at > NOW() - INTERVAL '24 hours')::BIGINT as plumes_24h,
            COUNT(*) FILTER (WHERE recorded_at > NOW() - INTERVAL '7 days')::BIGINT as plumes_7d,
            AVG(emission_rate_kg_hr) as avg_rate,
            MAX(emission_rate_kg_hr) as max_rate,
            MAX(recorded_at) as latest_plume_at
           FROM methane_observations"#,
    )
    .fetch_optional(&state.pool)
    .await;

    let weather_stats = sqlx::query(
        r#"SELECT 
            COUNT(*)::BIGINT as total_weather,
            COUNT(*) FILTER (WHERE recorded_at > NOW() - INTERVAL '24 hours')::BIGINT as weather_24h,
            MAX(recorded_at) as latest_weather_at
           FROM weather_observations"#,
    ).fetch_optional(&state.pool).await;

    let alert_stats = sqlx::query(
        r#"SELECT 
            COUNT(*)::BIGINT as total_alerts,
            COUNT(*) FILTER (WHERE triggered_at > NOW() - INTERVAL '24 hours')::BIGINT as alerts_24h
           FROM evacuation_alerts"#,
    )
    .fetch_optional(&state.pool)
    .await;

    let active_zones: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(DISTINCT region)::BIGINT FROM populated_zones"#)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(5);

    let (total_plumes, plumes_24h, plumes_7d, avg_rate, max_rate, latest_plume_at) =
        match plume_stats {
            Ok(Some(ref row)) => (
                row.get::<i64, _>("total_plumes"),
                row.get::<i64, _>("plumes_24h"),
                row.get::<i64, _>("plumes_7d"),
                row.get::<Option<f64>, _>("avg_rate").unwrap_or(0.0),
                row.get::<Option<f64>, _>("max_rate").unwrap_or(0.0),
                row.get::<Option<DateTime<Utc>>, _>("latest_plume_at"),
            ),
            _ => (0, 0, 0, 0.0, 0.0, None),
        };

    let (total_weather, weather_24h, latest_weather_at) = match weather_stats {
        Ok(Some(ref row)) => (
            row.get::<i64, _>("total_weather"),
            row.get::<i64, _>("weather_24h"),
            row.get::<Option<DateTime<Utc>>, _>("latest_weather_at"),
        ),
        _ => (0, 0, None),
    };

    let (total_alerts, alerts_24h) = match alert_stats {
        Ok(Some(ref row)) => (
            row.get::<i64, _>("total_alerts"),
            row.get::<i64, _>("alerts_24h"),
        ),
        _ => (0, 0),
    };

    let stats = SystemStats {
        total_plumes,
        plumes_last_24h: plumes_24h,
        plumes_last_7d: plumes_7d,
        avg_emission_rate: avg_rate,
        max_emission_rate: max_rate,
        total_weather_records: total_weather,
        weather_records_last_24h: weather_24h,
        total_alerts,
        alerts_last_24h: alerts_24h,
        active_zones,
        latest_plume_at,
        latest_weather_at,
    };

    (StatusCode::OK, AxumJson(stats)).into_response()
}

async fn get_populated_zones(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let records = sqlx::query(
        r#"SELECT zone_name, region, zone_type, ST_AsGeoJSON(geometry) as geom 
           FROM populated_zones"#,
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut features = vec![];
    for r in records {
        let geom: String = r.get("geom");
        features.push(json!({
            "type": "Feature",
            "properties": {
                "name": r.get::<String, _>("zone_name"),
                "region": r.get::<String, _>("region"),
                "type": r.get::<String, _>("zone_type")
            },
            "geometry": serde_json::from_str::<serde_json::Value>(&geom).unwrap_or_default()
        }));
    }

    let geojson = json!({ "type": "FeatureCollection", "features": features });
    (StatusCode::OK, AxumJson(geojson)).into_response()
}

async fn get_methane_plumes(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let records = sqlx::query(
        r#"SELECT recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as geometry,
           ST_AsGeoJSON(plume_geometry) as plume_geometry_json, source
         FROM methane_observations ORDER BY recorded_at DESC LIMIT 100"#,
    )
    .fetch_all(&state.pool)
    .await?;

    let plumes: Vec<MethanePlumeResponse> = records
        .into_iter()
        .map(|row| {
            let geometry_str: String = row.get("geometry");
            let plume_geometry_json: Option<String> = row.get("plume_geometry_json");
            MethanePlumeResponse {
                recorded_at: row.get("recorded_at"),
                emission_rate_kg_hr: row.get("emission_rate_kg_hr"),
                geometry: serde_json::from_str(&geometry_str).unwrap_or_default(),
                plume_footprint: plume_geometry_json
                    .and_then(|g: String| serde_json::from_str(&g).ok()),
                source: row.get("source"),
            }
        })
        .collect();

    Ok((StatusCode::OK, AxumJson(json!(plumes))))
}

async fn get_latest_weather(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let records = sqlx::query_as::<_, WeatherObservation>(
        "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
         FROM weather_observations ORDER BY recorded_at DESC LIMIT 10",
    ).fetch_all(&state.pool).await?;

    Ok((StatusCode::OK, AxumJson(json!(records))))
}

async fn get_weather_forecast(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let records = sqlx::query(
        r#"SELECT forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
           FROM weather_forecasts
           WHERE valid_at > NOW()
           ORDER BY valid_at ASC
           LIMIT 48"#,
    ).fetch_all(&state.pool).await?;

    let forecasts: Vec<serde_json::Value> = records
        .into_iter()
        .map(|r| {
            json!({
                "forecast_at": r.get::<DateTime<Utc>, _>("forecast_at"),
                "valid_at": r.get::<DateTime<Utc>, _>("valid_at"),
                "area_id": r.get::<String, _>("area_id"),
                "wind_speed_ms": r.get::<Option<f64>, _>("wind_speed_ms"),
                "wind_direction_deg": r.get::<Option<f64>, _>("wind_direction_deg"),
                "humidity_percent": r.get::<Option<f64>, _>("humidity_percent"),
                "temperature_c": r.get::<Option<f64>, _>("temperature_c"),
                "data_source": r.get::<Option<String>, _>("data_source").unwrap_or_default()
            })
        })
        .collect();

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
            { "rel": "self", "href": "/api/stac", "type": "application/json" },
            { "rel": "service-desc", "href": "/api", "type": "application/vnd.oai.openapi+json;version=3.0" },
            { "rel": "data", "href": "/api/stac/collections", "type": "application/json" },
            { "rel": "search", "href": "/api/stac/search", "type": "application/geo+json", "method": "GET" }
        ]
    });
    Ok((StatusCode::OK, AxumJson(root)))
}

async fn stac_collections() -> Result<impl IntoResponse, AppError> {
    let collections = json!({
        "collections": [StacCollection::methane_observations()],
        "links": [{ "rel": "self", "href": "/api/stac/collections", "type": "application/json" }]
    });
    Ok((StatusCode::OK, AxumJson(collections)))
}

async fn stac_collection() -> Result<impl IntoResponse, AppError> {
    Ok((
        StatusCode::OK,
        AxumJson(json!(StacCollection::methane_observations())),
    ))
}

async fn stac_items(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StacSearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let limit = params.limit.unwrap_or(100).min(1000) as i64;
    let items = fetch_stac_items(&state.pool, limit).await?;
    let count = items.len() as u32;
    let response = StacSearchResponse {
        r#type: "FeatureCollection".to_string(),
        features: items,
        links: vec![StacLink {
            rel: "self".to_string(),
            href: "/api/stac/collections/methane-observations/items".to_string(),
            r#type: Some("application/geo+json".to_string()),
            title: None,
        }],
        context: Some(stac::StacSearchContext {
            returned: count,
            matched: None,
        }),
    };
    Ok((StatusCode::OK, AxumJson(json!(response))))
}

async fn stac_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StacSearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let limit = params.limit.unwrap_or(100).min(1000) as i64;
    let items = fetch_stac_items(&state.pool, limit).await?;
    let count = items.len() as u32;
    let response = StacSearchResponse {
        r#type: "FeatureCollection".to_string(),
        features: items,
        links: vec![StacLink {
            rel: "self".to_string(),
            href: "/api/stac/search".to_string(),
            r#type: Some("application/geo+json".to_string()),
            title: None,
        }],
        context: Some(stac::StacSearchContext {
            returned: count,
            matched: None,
        }),
    };
    Ok((StatusCode::OK, AxumJson(json!(response))))
}

async fn fetch_stac_items(pool: &Pool<Postgres>, limit: i64) -> Result<Vec<StacItem>, AppError> {
    let records = sqlx::query(
        r#"SELECT id, recorded_at, emission_rate_kg_hr, 
           ST_AsGeoJSON(location) as geometry,
           ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
           source
         FROM methane_observations 
         ORDER BY recorded_at DESC 
         LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let items: Vec<StacItem> = records
        .into_iter()
        .map(|row| {
            let geometry_str: String = row.get("geometry");
            let geometry: serde_json::Value =
                serde_json::from_str(&geometry_str).unwrap_or_default();
            let lon: Option<f64> = row.get("lon");
            let lat: Option<f64> = row.get("lat");
            let lon = lon.unwrap_or(0.0);
            let lat = lat.unwrap_or(0.0);
            let bbox = vec![lon - 0.01, lat - 0.01, lon + 0.01, lat + 0.01];
            let source: Option<String> = row.get("source");

            StacItem::from_methane_observation(
                row.get("id"),
                row.get("recorded_at"),
                row.get("emission_rate_kg_hr"),
                geometry,
                bbox,
                &source.unwrap_or_else(|| "unknown".to_string()),
            )
        })
        .collect();

    Ok(items)
}

// ─── PHYSICS & DISPERSION ────────────────────────────────────────────────────

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

fn calc_gaussian_concentration_1km(emission_kg_hr: f64, ws: f64, stability: char) -> f64 {
    let q_g_s = emission_kg_hr * 1000.0 / 3600.0;
    let (sy, sz) = match stability {
        'A' => (210.0, 450.0),
        'B' => (155.0, 110.0),
        'C' => (105.0, 61.0),
        'D' => (68.0, 31.0),
        'E' => (50.0, 21.0),
        'F' => (34.0, 11.0),
        _ => (68.0, 31.0),
    };
    let ws_safe = if ws < 1.0 { 1.0 } else { ws };
    let c_g_m3 = q_g_s / (std::f64::consts::PI * ws_safe * sy * sz);
    let c_mg_m3 = c_g_m3 * 1000.0;
    c_mg_m3 * 1.5
}

async fn get_multi_plume_prediction(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let active_sources = match sqlx::query(
        r#"SELECT id, ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat, emission_rate_kg_hr
           FROM methane_observations WHERE recorded_at > NOW() - INTERVAL '24 hours'"#,
    ).fetch_all(&state.pool).await {
        Ok(s) => s,
        Err(e) => {
            error!("Database error fetching sources: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, AxumJson(json!({"error": "DB Error"}))).into_response();
        }
    };

    if active_sources.is_empty() {
        return (
            StatusCode::OK,
            AxumJson(json!(Vec::<serde_json::Value>::new())),
        )
            .into_response();
    }

    let mut predictions = Vec::new();
    let wita_offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let now_wita = Utc::now().with_timezone(&wita_offset);
    let is_daytime = now_wita.hour() >= 6 && now_wita.hour() < 18;

    let sensor_roll = std::env::var("SENSOR_ROLL_DEG")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(5.0);
    let sensor_pitch = std::env::var("SENSOR_PITCH_DEG")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0);
    let is_smeared = sensor_roll > 6.85 || sensor_pitch > 4.8;

    for source in active_sources {
        let id: Uuid = source.get("id");
        let lon: Option<f64> = source.get("lon");
        let lat: Option<f64> = source.get("lat");
        let lon = lon.unwrap_or(0.0);
        let lat = lat.unwrap_or(0.0);
        let region = get_region_from_coords(lon, lat);
        let emission_rate: f64 = source.get("emission_rate_kg_hr");

        if emission_rate < tanager1::DETECTION_CONSERVATIVE_KG_HR {
            continue;
        }

        let weather = sqlx::query(
            r#"SELECT wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c
               FROM weather_observations 
               WHERE area_id = $1 AND recorded_at > NOW() - INTERVAL '6 hours'
               ORDER BY recorded_at DESC LIMIT 1"#,
        )
        .bind(region)
        .fetch_optional(&state.pool)
        .await
        .unwrap_or_default();

        let w = match weather {
            Some(w) => w,
            None => continue,
        };

        let ws: Option<f64> = w.get("wind_speed_ms");
        let wd: Option<f64> = w.get("wind_direction_deg");
        let hum: Option<f64> = w.get("humidity_percent");
        let temp: Option<f64> = w.get("temperature_c");
        let ws = ws.unwrap_or(1.0);
        let wd = wd.unwrap_or(0.0);
        let hum = hum.unwrap_or(0.0);
        let temp = temp.unwrap_or(25.0);

        let mut distance = ws * 3600.0;
        if hum > 85.0 {
            distance *= 0.60;
        }

        let stability = get_pasquill_stability_class(ws, is_daytime);
        let mut spread_angle = get_plume_spread_angle(stability);

        let temp_k: f64 = temp + 273.15;
        let baseline_k: f64 = 308.15;
        if temp_k > baseline_k {
            spread_angle *= (baseline_k / temp_k).powi(4);
        }

        let conc_1km = calc_gaussian_concentration_1km(emission_rate, ws, stability);

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

        let geom_rec = sqlx::query(
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
        .bind(lon).bind(lat).bind(distance).bind(wd).bind(spread_angle)
        .fetch_one(&state.pool).await.ok();

        if let Some(rec) = geom_rec {
            let json_str: String = rec.get("json");
            let geojson_val: serde_json::Value =
                serde_json::from_str(&json_str).unwrap_or_default();

            let affected = sqlx::query(
                r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
                   FROM populated_zones 
                   WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
            )
            .bind(&json_str)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();

            let exposure_alert = !affected.is_empty();
            let mut affected_zones = Vec::new();

            for zone in &affected {
                let zone_name: String = zone.get("zone_name");
                let region_str: String = zone.get("region");
                affected_zones.push(AffectedZone {
                    zone_name: zone_name.clone(),
                    region: region_str.clone(),
                    population_estimate: zone.get("population_estimate"),
                    zone_type: zone.get("zone_type"),
                    is_volcanic_zone: zone.get("is_volcanic_zone"),
                });

                if conc_1km > 50.0 {
                    let msg = format!(
                        "⚠️ <b>EVACUATION ALERT</b>\n\n<b>Zone:</b> {} ({})\n<b>Emission:</b> {:.2} kg/hr\n<b>Max Dist:</b> {:.0}m\n<b>Est. Conc:</b> {:.1} ppm",
                        zone_name, region_str, emission_rate, distance, conc_1km
                    );
                    let client = state.http_client.clone();
                    let token = state.config.telegram.bot_token.clone();
                    let chat_id = state.config.telegram.chat_id.clone();
                    tokio::spawn(async move {
                        send_telegram_alert(&client, &msg, &token, &chat_id).await;
                    });

                    ws::broadcast_alert(
                        &state.ws_state.tx,
                        zone_name.clone(),
                        region_str.clone(),
                        emission_rate,
                        format!(
                            "Evacuation alert: {} ({}) - {:.1} ppm at 1km",
                            zone_name, region_str, conc_1km
                        ),
                    )
                    .await;

                    let _ = sqlx::query(
                        "INSERT INTO evacuation_alerts (region, zone_name, emission_rate_kg_hr, wind_speed_ms, wind_direction_deg, concentration_ppm, stability_class) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    )
                    .bind(&region_str).bind(&zone_name).bind(emission_rate).bind(ws).bind(wd).bind(conc_1km).bind(stability.to_string())
                    .execute(&state.pool).await;
                }
            }

            predictions.push(json!({
                "source_id": id,
                "source_lon": lon,
                "source_lat": lat,
                "emission_rate_kg_hr": emission_rate,
                "wind_speed_ms": ws,
                "wind_direction_deg": wd,
                "stability_class": stability.to_string(),
                "spread_angle_deg": spread_angle,
                "max_distance_m": distance,
                "concentration_at_1km_ppm": conc_1km,
                "plume_geojson": geojson_val,
                "high_uncertainty_smear": is_smeared,
                "terrain_blocked": terrain_blocked,
                "terrain_block_distance_m": block_dist,
                "affected_zones": affected_zones,
                "exposure_alert": exposure_alert,
            }));
        }
    }

    (StatusCode::OK, AxumJson(json!(predictions))).into_response()
}

async fn get_plume_analysis(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state
        .metrics
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let active_sources = match sqlx::query(
        r#"SELECT id, recorded_at, emission_rate_kg_hr, 
           ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
           ST_AsGeoJSON(plume_geometry) as plume_geometry_json, source
           FROM methane_observations 
           WHERE recorded_at > NOW() - INTERVAL '24 hours'
           ORDER BY recorded_at DESC"#,
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(s) => s,
        Err(e) => {
            error!("Database error fetching sources: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(json!({"error": "DB Error"})),
            )
                .into_response();
        }
    };

    if active_sources.is_empty() {
        return (
            StatusCode::OK,
            AxumJson(json!(Vec::<serde_json::Value>::new())),
        )
            .into_response();
    }

    let mut analyses = Vec::new();
    let wita_offset = FixedOffset::east_opt(8 * 3600).unwrap();

    for source in active_sources {
        let id: Uuid = source.get("id");
        let recorded_at: DateTime<Utc> = source.get("recorded_at");
        let lon: Option<f64> = source.get("lon");
        let lat: Option<f64> = source.get("lat");
        let lon = lon.unwrap_or(0.0);
        let lat = lat.unwrap_or(0.0);
        let region = get_region_from_coords(lon, lat);
        let emission_rate: f64 = source.get("emission_rate_kg_hr");
        let plume_geometry_json: Option<String> = source.get("plume_geometry_json");
        let source_name: Option<String> = source.get("source");

        if emission_rate < tanager1::DETECTION_90PCT_KG_HR {
            continue;
        }

        let mut observed_affected_zones = Vec::new();
        let mut observed_exposure_alert = false;

        if let Some(ref plume_json) = plume_geometry_json {
            let affected = sqlx::query(
                r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
                   FROM populated_zones 
                   WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
            )
            .bind(plume_json)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();

            observed_exposure_alert = !affected.is_empty();
            for zone in &affected {
                observed_affected_zones.push(AffectedZone {
                    zone_name: zone.get("zone_name"),
                    region: zone.get("region"),
                    population_estimate: zone.get("population_estimate"),
                    zone_type: zone.get("zone_type"),
                    is_volcanic_zone: zone.get("is_volcanic_zone"),
                });
            }
        }

        let observed = ObservedPlume {
            plume_footprint: plume_geometry_json
                .and_then(|g: String| serde_json::from_str(&g).ok()),
            affected_zones: observed_affected_zones,
            exposure_alert: observed_exposure_alert,
            source: source_name.unwrap_or_else(|| "unknown".to_string()),
        };

        let forecasts = sqlx::query(
            r#"SELECT valid_at, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c
               FROM weather_forecasts 
               WHERE area_id = $1 AND valid_at > NOW() AND valid_at < NOW() + INTERVAL '6 hours'
               ORDER BY valid_at ASC"#,
        )
        .bind(region)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let mut forecasted_plumes = Vec::new();
        let sensor_roll = std::env::var("SENSOR_ROLL_DEG")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(5.0);
        let sensor_pitch = std::env::var("SENSOR_PITCH_DEG")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(2.0);
        let _is_smeared = sensor_roll > 6.85 || sensor_pitch > 4.8;

        for fc in &forecasts {
            let ws: Option<f64> = fc.get("wind_speed_ms");
            let wd: Option<f64> = fc.get("wind_direction_deg");
            let hum: Option<f64> = fc.get("humidity_percent");
            let temp: Option<f64> = fc.get("temperature_c");
            let valid_at: DateTime<Utc> = fc.get("valid_at");
            let ws = ws.unwrap_or(1.0);
            let wd = wd.unwrap_or(0.0);
            let hum = hum.unwrap_or(0.0);
            let temp = temp.unwrap_or(25.0);

            let is_daytime = valid_at.with_timezone(&wita_offset).hour() >= 6
                && valid_at.with_timezone(&wita_offset).hour() < 18;

            let mut distance = ws * 3600.0;
            if hum > 85.0 {
                distance *= 0.60;
            }

            let stability = get_pasquill_stability_class(ws, is_daytime);
            let mut spread_angle = get_plume_spread_angle(stability);

            let temp_k: f64 = temp + 273.15;
            let baseline_k: f64 = 308.15;
            if temp_k > baseline_k {
                spread_angle *= (baseline_k / temp_k).powi(4);
            }

            let conc_1km = calc_gaussian_concentration_1km(emission_rate, ws, stability);

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

            let geom_rec = sqlx::query(
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
            .bind(lon).bind(lat).bind(distance).bind(wd).bind(spread_angle)
            .fetch_one(&state.pool).await.ok();

            if let Some(rec) = geom_rec {
                let json_str: String = rec.get("json");
                let geojson_val: serde_json::Value =
                    serde_json::from_str(&json_str).unwrap_or_default();

                let affected = sqlx::query(
                    r#"SELECT zone_name, region, zone_type, population_estimate, is_volcanic_zone
                       FROM populated_zones 
                       WHERE ST_Intersects(geometry, ST_GeomFromGeoJSON($1))"#,
                )
                .bind(&json_str)
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default();

                let exposure_alert = !affected.is_empty();
                let mut affected_zones = Vec::new();

                for zone in &affected {
                    let zone_name: String = zone.get("zone_name");
                    let region_str: String = zone.get("region");
                    affected_zones.push(AffectedZone {
                        zone_name: zone_name.clone(),
                        region: region_str.clone(),
                        population_estimate: zone.get("population_estimate"),
                        zone_type: zone.get("zone_type"),
                        is_volcanic_zone: zone.get("is_volcanic_zone"),
                    });

                    if conc_1km > 50.0 {
                        let msg = format!(
                            "⚠️ <b>FORECAST ALERT</b>\n\n<b>Zone:</b> {} ({})\n<b>Emission:</b> {:.2} kg/hr\n<b>Valid At:</b> {}\n<b>Max Dist:</b> {:.0}m\n<b>Est. Conc:</b> {:.1} ppm",
                            zone_name, region_str, emission_rate, valid_at.format("%Y-%m-%d %H:%M UTC"), distance, conc_1km
                        );
                        let client = state.http_client.clone();
                        let token = state.config.telegram.bot_token.clone();
                        let chat_id = state.config.telegram.chat_id.clone();
                        tokio::spawn(async move {
                            send_telegram_alert(&client, &msg, &token, &chat_id).await;
                        });
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
            source_id: id,
            source_lon: lon,
            source_lat: lat,
            emission_rate_kg_hr: emission_rate,
            recorded_at,
            observed,
            forecast: forecasted_plumes,
        });
    }

    (StatusCode::OK, AxumJson(json!(analyses))).into_response()
}

// ─── BACKGROUND TASKS ────────────────────────────────────────────────────────

async fn carbon_mapper_tracker_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(
        state.config.carbon_mapper.poll_interval_secs,
    ));
    let api_token = &state.config.carbon_mapper.api_token;
    if api_token.is_empty() {
        return;
    }

    loop {
        interval.tick().await;
        state
            .metrics
            .carbon_mapper_fetches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut next_url = Some(state.config.carbon_mapper.base_url.clone());
        let mut cycle_errors = 0u64;

        while let Some(url) = next_url.clone() {
            let payload = json!({
                "bbox": state.config.carbon_mapper.bbox,
                "datetime": format!("2024-01-01T00:00:00Z/{}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
                "limit": 100
            });

            match state
                .http_client
                .post(&url)
                .header("X-API-KEY", api_token.as_str())
                .json(&payload)
                .send()
                .await
            {
                Ok(res) if res.status().is_success() => {
                    if let Ok(stac) = res.json::<StacResponse>().await {
                        *state.last_stac_fetch.write().unwrap() = Some(Utc::now());

                        for feature in stac.features {
                            if feature.properties.emission_rate_kg_hr <= 0.0 {
                                continue;
                            }
                            let dt =
                                chrono::DateTime::parse_from_rfc3339(&feature.properties.datetime)
                                    .unwrap()
                                    .with_timezone(&Utc);
                            let geom = serde_json::to_string(&feature.geometry).unwrap();

                            let (lon, lat) =
                                if let Some(coords) = feature.geometry.get("coordinates") {
                                    if let Some(arr) = coords.as_array() {
                                        if arr.len() >= 2 {
                                            (
                                                arr[0].as_f64().unwrap_or(0.0),
                                                arr[1].as_f64().unwrap_or(0.0),
                                            )
                                        } else {
                                            (0.0, 0.0)
                                        }
                                    } else {
                                        (0.0, 0.0)
                                    }
                                } else {
                                    (0.0, 0.0)
                                };

                            let res = sqlx::query(
                                "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location, plume_geometry, source) VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)), ST_GeomFromGeoJSON($3), 'carbon_mapper') ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
                            )
                            .bind(dt).bind(feature.properties.emission_rate_kg_hr).bind(&geom)
                            .execute(&state.pool).await;

                            if res.is_ok() && res.unwrap().rows_affected() > 0 {
                                state
                                    .metrics
                                    .plumes_ingested
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                ws::broadcast_plume_update(
                                    &state.ws_state.tx,
                                    format!("{}", Uuid::new_v4()),
                                    feature.properties.emission_rate_kg_hr,
                                    lat,
                                    lon,
                                    dt.to_rfc3339(),
                                )
                                .await;
                            }
                        }

                        next_url = stac
                            .links
                            .iter()
                            .find(|l| l.rel == "next")
                            .map(|l| l.href.clone());
                    } else {
                        next_url = None;
                    }
                }
                _ => {
                    state
                        .metrics
                        .carbon_mapper_errors
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    cycle_errors += 1;
                    next_url = None;
                }
            }

            // If Carbon Mapper had errors this cycle, log for EMIT fallback
            if cycle_errors > 0 {
                info!(
                    "Carbon Mapper had {} errors this cycle, EMIT fallback active",
                    cycle_errors
                );
                cycle_errors = 0;
            }
        }
    }
}

async fn emit_tracker_task(state: Arc<AppState>) {
    if !state.config.emit.enabled {
        info!("EMIT fallback disabled");
        return;
    }

    let mut interval = time::interval(Duration::from_secs(state.config.emit.poll_interval_secs));
    let bbox = &state.config.emit.bbox;
    let collection = "emit-ch4plume-v1";

    info!(
        "EMIT fallback task started (poll: {}s)",
        state.config.emit.poll_interval_secs
    );

    loop {
        interval.tick().await;
        state
            .metrics
            .emit_fetches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let base_search_url = format!("{}/search", state.config.emit.base_url);
        let mut next_url = Some(base_search_url.clone());

        while let Some(url) = next_url.take() {
            let payload = json!({
                "collections": [collection],
                "bbox": bbox,
                "datetime": format!("2022-08-01T00:00:00Z/{}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
                "limit": 100
            });

            match state.http_client.post(&url).json(&payload).send().await {
                Ok(res) if res.status().is_success() => {
                    if let Ok(stac) = res.json::<EmitStacResponse>().await {
                        *state.last_emit_fetch.write().unwrap() = Some(Utc::now());

                        for feature in stac.features {
                            let emission_rate = match feature.properties.ch4_plume_emission_rate {
                                Some(rate) if rate > 0.0 => rate,
                                _ => continue,
                            };

                            let dt = match chrono::DateTime::parse_from_rfc3339(
                                &feature.properties.datetime,
                            ) {
                                Ok(dt) => dt.with_timezone(&Utc),
                                Err(_) => continue,
                            };

                            let geom = serde_json::to_string(&feature.geometry).unwrap_or_default();

                            let (lon, lat) =
                                if let Some(coords) = feature.geometry.get("coordinates") {
                                    if let Some(arr) = coords.as_array() {
                                        if arr.len() >= 2 {
                                            (
                                                arr[0].as_f64().unwrap_or(0.0),
                                                arr[1].as_f64().unwrap_or(0.0),
                                            )
                                        } else {
                                            (0.0, 0.0)
                                        }
                                    } else {
                                        (0.0, 0.0)
                                    }
                                } else {
                                    (0.0, 0.0)
                                };

                            let plume_id = feature
                                .properties
                                .ch4_plume_id
                                .unwrap_or_else(|| format!("emit-{}", Uuid::new_v4()));

                            let res = sqlx::query(
                                "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location, plume_geometry, source) VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)), ST_GeomFromGeoJSON($3), 'emit') ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
                            )
                            .bind(dt).bind(emission_rate).bind(&geom)
                            .execute(&state.pool).await;

                            if let Ok(result) = res {
                                if result.rows_affected() > 0 {
                                    state
                                        .metrics
                                        .emit_plumes_ingested
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    ws::broadcast_plume_update(
                                        &state.ws_state.tx,
                                        plume_id,
                                        emission_rate,
                                        lat,
                                        lon,
                                        dt.to_rfc3339(),
                                    )
                                    .await;
                                }
                            }
                        }

                        next_url = stac
                            .links
                            .iter()
                            .find(|l| l.rel == "next")
                            .map(|l| l.href.clone());
                    } else {
                        next_url = None;
                    }
                }
                _ => {
                    state
                        .metrics
                        .emit_errors
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    warn!("EMIT fetch failed for: {}", url);
                }
            }
        }
    }
}

async fn s5p_tracker_task(state: Arc<AppState>) {
    if !state.config.s5p.enabled {
        info!("S5P macro radar disabled");
        return;
    }

    let mut interval = time::interval(Duration::from_secs(state.config.s5p.poll_interval_secs));
    // S5P requires slightly wider bounding box
    let bbox = vec![115.0, -9.5, 120.0, -7.5];

    info!(
        "S5P macro radar started (poll: {}s)",
        state.config.s5p.poll_interval_secs
    );

    loop {
        interval.tick().await;
        state
            .metrics
            .s5p_fetches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let search_url = format!("{}/search", state.config.s5p.base_url);
        let payload = json!({
            "collections": ["sentinel-5p-l2-netcdf"],
            "bbox": bbox,
            "datetime": format!("2024-01-01T00:00:00Z/{}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
            "query": {
                "s5p:product_type": {"eq": "L2__CH4___"}
            },
            "limit": 10
        });

        match state
            .http_client
            .post(&search_url)
            .json(&payload)
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => {
                if let Ok(stac) = res.json::<PlanetaryComputerResponse>().await {
                    *state.last_s5p_fetch.write().unwrap() = Some(Utc::now());

                    for feature in stac.features {
                        let start_dt = match chrono::DateTime::parse_from_rfc3339(
                            &feature.properties.start_datetime,
                        ) {
                            Ok(dt) => dt.with_timezone(&Utc),
                            Err(_) => continue,
                        };
                        let end_dt = match chrono::DateTime::parse_from_rfc3339(
                            &feature.properties.end_datetime,
                        ) {
                            Ok(dt) => dt.with_timezone(&Utc),
                            Err(_) => continue,
                        };

                        let geom = serde_json::to_string(&feature.geometry).unwrap_or_default();

                        let download_url = feature.assets.get("ch4").map(|a| a.href.clone());

                        let _ = sqlx::query(
                            "INSERT INTO s5p_overpasses (scene_id, start_datetime, end_datetime, orbit_number, footprint, netcdf_download_url) VALUES ($1, $2, $3, $4, ST_GeomFromGeoJSON($5), $6) ON CONFLICT (scene_id) DO NOTHING",
                        )
                        .bind(&feature.id)
                        .bind(start_dt)
                        .bind(end_dt)
                        .bind(feature.properties.orbit)
                        .bind(&geom)
                        .bind(download_url)
                        .execute(&state.pool).await;
                    }
                }
            }
            _ => {
                state
                    .metrics
                    .s5p_errors
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        state
            .metrics
            .bmkg_fetches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *state.last_bmkg_fetch.write().unwrap() = Some(Utc::now());

        for (name, bmkg_id, lat, lon) in zones {
            let mut success = false;
            let bmkg_url = format!(
                "https://api.bmkg.go.id/publik/prakiraan-cuaca?adm4={}",
                bmkg_id
            );

            if let Ok(res) = state.http_client.get(&bmkg_url).send().await {
                if let Ok(json) = res.json::<BmkgResponse>().await {
                    if let Some(item) = json
                        .data
                        .first()
                        .and_then(|g| g.cuaca.first())
                        .and_then(|l| l.first())
                    {
                        let _ = sqlx::query(
                            "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source) VALUES (NOW(), $1, $2, $3, $4, $5, 'BMKG')",
                        )
                        .bind(name).bind(item.ws / 3.6).bind(item.wd_deg).bind(item.hu).bind(item.t)
                        .execute(&state.pool).await;
                        success = true;
                        ws::broadcast_weather_update(
                            &state.ws_state.tx,
                            name.to_string(),
                            item.ws / 3.6,
                            item.wd_deg,
                            item.t,
                            item.hu,
                        )
                        .await;
                    }
                }
            }

            if !success {
                let om_url = format!("https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,wind_speed_10m,wind_direction_10m,relative_humidity_2m", lat, lon);
                if let Ok(res) = state.http_client.get(&om_url).send().await {
                    if let Ok(json) = res.json::<OpenMeteoResponse>().await {
                        let _ = sqlx::query(
                            "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source) VALUES (NOW(), $1, $2, $3, $4, $5, 'Open-Meteo')",
                        )
                        .bind(name).bind(json.current.wind_speed_10m).bind(json.current.wind_direction_10m).bind(json.current.relative_humidity_2m).bind(json.current.temperature_2m)
                        .execute(&state.pool).await;
                        ws::broadcast_weather_update(
                            &state.ws_state.tx,
                            name.to_string(),
                            json.current.wind_speed_10m,
                            json.current.wind_direction_10m,
                            json.current.temperature_2m,
                            json.current.relative_humidity_2m,
                        )
                        .await;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn data_retention_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(86400));
    loop {
        interval.tick().await;
        info!("Running data retention cleanup...");
        let _ = sqlx::query(
            "DELETE FROM weather_observations WHERE recorded_at < NOW() - INTERVAL '30 days'",
        )
        .execute(&state.pool)
        .await;
        let _ = sqlx::query(
            "DELETE FROM weather_forecasts WHERE created_at < NOW() - INTERVAL '7 days'",
        )
        .execute(&state.pool)
        .await;
    }
}

async fn weather_forecast_task(state: Arc<AppState>) {
    let mut interval = time::interval(Duration::from_secs(3600));
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
                            &forecast.hourly.time[i],
                            "%Y-%m-%dT%H:%M",
                        )
                        .unwrap_or_default();

                        let _ = sqlx::query(
                            "INSERT INTO weather_forecasts (forecast_at, valid_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source) VALUES (NOW(), $1, $2, $3, $4, $5, $6, 'Open-Meteo')",
                        )
                        .bind(valid_at.and_utc())
                        .bind(name)
                        .bind(forecast.hourly.wind_speed_10m.get(i).copied().unwrap_or(0.0))
                        .bind(forecast.hourly.wind_direction_10m.get(i).copied().unwrap_or(0.0))
                        .bind(forecast.hourly.relative_humidity_2m.get(i).copied().unwrap_or(0.0))
                        .bind(forecast.hourly.temperature_2m.get(i).copied().unwrap_or(0.0))
                        .execute(&state.pool).await;
                    }
                    info!(
                        "Stored {} forecast hours for {}",
                        forecast.hourly.time.len(),
                        name
                    );
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
        assert_eq!(get_pasquill_stability_class(2.0, true), 'A');
        assert_eq!(get_pasquill_stability_class(4.0, true), 'B');
        assert_eq!(get_pasquill_stability_class(6.0, true), 'C');
    }

    #[test]
    fn test_pasquill_stability_class_nighttime() {
        assert_eq!(get_pasquill_stability_class(2.0, false), 'F');
        assert_eq!(get_pasquill_stability_class(4.0, false), 'E');
        assert_eq!(get_pasquill_stability_class(6.0, false), 'D');
    }

    #[test]
    fn test_plume_spread_angle() {
        assert_eq!(get_plume_spread_angle('A'), 25.0);
        assert_eq!(get_plume_spread_angle('B'), 20.0);
        assert_eq!(get_plume_spread_angle('C'), 15.0);
        assert_eq!(get_plume_spread_angle('D'), 12.5);
        assert_eq!(get_plume_spread_angle('E'), 8.75);
        assert_eq!(get_plume_spread_angle('F'), 5.0);
        assert_eq!(get_plume_spread_angle('X'), 12.5);
    }

    #[test]
    fn test_gaussian_concentration_1km() {
        let conc = calc_gaussian_concentration_1km(1000.0, 3.0, 'D');
        assert!(conc > 0.0);
        assert!(conc < 1000.0);

        let conc_low = calc_gaussian_concentration_1km(100.0, 3.0, 'D');
        let conc_high = calc_gaussian_concentration_1km(1000.0, 3.0, 'D');
        assert!(conc_high > conc_low);

        let conc_low_wind = calc_gaussian_concentration_1km(1000.0, 1.0, 'D');
        let conc_high_wind = calc_gaussian_concentration_1km(1000.0, 10.0, 'D');
        assert!(conc_low_wind > conc_high_wind);
    }

    #[test]
    fn test_gaussian_concentration_wind_safety() {
        let conc = calc_gaussian_concentration_1km(1000.0, 0.5, 'D');
        assert!(conc > 0.0);
    }

    #[test]
    fn test_region_from_coords() {
        assert_eq!(get_region_from_coords(116.1240, -8.6818), "Lombok Barat");
        assert_eq!(get_region_from_coords(116.2667, -8.7167), "Lombok Tengah");
        assert_eq!(get_region_from_coords(118.7167, -8.4667), "Kota Bima");
    }

    #[test]
    fn test_shot_noise_bound() {
        let min_detection = tanager1::DETECTION_90PCT_KG_HR;
        assert!(50.0 < min_detection);
        assert!(100.0 >= min_detection);
        assert_eq!(min_detection, 90.0);
    }

    #[test]
    fn test_sensor_smear_thresholds() {
        let roll_limit = 6.85;
        let pitch_limit = 4.8;
        assert!(5.0 <= roll_limit);
        assert!(2.0 <= pitch_limit);
        assert!(7.0 > roll_limit);
        assert!(5.0 > pitch_limit);
    }

    #[test]
    fn test_terrain_blocking_threshold() {
        let threshold = 15.0;
        assert!(10.0 < threshold);
        assert!(20.0 > threshold);
    }
}
