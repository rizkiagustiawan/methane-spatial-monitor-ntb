use axum::{extract::State, response::Html, routing::get, Json, Router};
use dotenvy::dotenv;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{error, info};
use tower_http::cors::{Any, CorsLayer};

mod models;
use models::*;

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

    let shared_pool = Arc::new(pool);

    // Spawn Background Tasks
    let pool_stac = Arc::clone(&shared_pool);
    tokio::spawn(async move {
        carbon_mapper_tracker_task(pool_stac).await;
    });

    let pool_bmkg = Arc::clone(&shared_pool);
    tokio::spawn(async move {
        bmkg_tracker_task(pool_bmkg).await;
    });

    // API Routes
    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("../frontend/index.html")) }))
        .route("/health", get(|| async { "OK" }))
        .route("/api/weather", get(get_latest_weather))
        .route("/api/methane", get(get_latest_methane))
        .route("/api/methane/plumes", get(get_methane_plumes))
        .route("/api/plume-prediction", get(get_plume_prediction))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(shared_pool);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Server running on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn get_methane_plumes(
    State(pool): State<Arc<Pool<Postgres>>>,
) -> Json<Vec<MethanePlumeResponse>> {
    let records = sqlx::query!(
        r#"SELECT recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as "geometry!"
         FROM methane_observations
         ORDER BY recorded_at DESC
         LIMIT 100"#
    )
    .fetch_all(&*pool)
    .await
    .unwrap_or_default();

    let plumes = records
        .into_iter()
        .map(|row| MethanePlumeResponse {
            recorded_at: row.recorded_at,
            emission_rate_kg_hr: row.emission_rate_kg_hr,
            geometry: serde_json::from_str(&row.geometry).unwrap_or_default(),
        })
        .collect();

    Json(plumes)
}

async fn get_latest_weather(
    State(pool): State<Arc<Pool<Postgres>>>,
) -> Json<Vec<WeatherObservation>> {
    let records = sqlx::query_as!(
        WeatherObservation,
        "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source
         FROM weather_observations
         ORDER BY recorded_at DESC
         LIMIT 10"
    )
    .fetch_all(&*pool)
    .await
    .unwrap_or_default();

    Json(records)
}

async fn get_latest_methane(
    State(pool): State<Arc<Pool<Postgres>>>,
) -> Json<Vec<MethaneObservation>> {
    let records = sqlx::query_as!(
        MethaneObservation,
        r#"SELECT id, recorded_at, emission_rate_kg_hr, ST_AsGeoJSON(location) as "location_json!", 0.0::FLOAT8 as "total_green_area_hectares!"
         FROM methane_observations
         ORDER BY recorded_at DESC
         LIMIT 10"#
    )
    .fetch_all(&*pool)
    .await
    .unwrap_or_default();

    Json(records)
}

async fn get_plume_prediction(
    State(pool): State<Arc<Pool<Postgres>>>,
) -> Json<Option<PlumePrediction>> {
    let record = sqlx::query_as!(
        PlumePrediction,
        r#"
        WITH latest_weather AS (
            SELECT wind_speed_ms, wind_direction_deg FROM weather_observations 
            WHERE wind_speed_ms IS NOT NULL AND wind_direction_deg IS NOT NULL
            ORDER BY recorded_at DESC LIMIT 1
        ),
        latest_methane AS (
            SELECT location, emission_rate_kg_hr FROM methane_observations ORDER BY recorded_at DESC LIMIT 1
        ),
        plume_vars AS (
            SELECT
                m.emission_rate_kg_hr,
                w.wind_speed_ms,
                w.wind_direction_deg,
                m.location::geography as origin,
                (w.wind_speed_ms * 3600.0) as distance_1hr, -- x-axis (downwind distance in 1 hour)
                radians(w.wind_direction_deg + 180.0) as blow_to_rad, -- wind direction vector
                radians(12.5) as spread_rad -- y-axis spread (sigma_y equivalent for Class D stability)
            FROM latest_methane m CROSS JOIN latest_weather w
        )
        SELECT
            emission_rate_kg_hr,
            wind_speed_ms as "wind_speed_ms!",
            wind_direction_deg as "wind_direction_deg!",
            ST_AsGeoJSON(
                ST_MakePolygon(
                    ST_MakeLine(ARRAY[
                        origin::geometry,
                        ST_Project(origin, distance_1hr, blow_to_rad - spread_rad)::geometry,
                        ST_Project(origin, distance_1hr, blow_to_rad + spread_rad)::geometry,
                        origin::geometry
                    ])
                )
            ) as "plume_line_json!"
        FROM plume_vars
        "#
    )
    .fetch_optional(&*pool)
    .await
    .unwrap_or_default();
     
    Json(record)
}

async fn carbon_mapper_tracker_task(pool: Arc<Pool<Postgres>>) {
    let mut interval = time::interval(Duration::from_secs(86400)); // Daily
    let api_token = std::env::var("CARBON_MAPPER_TOKEN").expect("CARBON_MAPPER_TOKEN must be set in .env");
    let client = reqwest::Client::new();
    let url = "https://api.carbonmapper.org/api/v1/stac/search";

    loop {
        interval.tick().await;
        info!("Running Carbon Mapper STAC Tracker...");

        let payload = serde_json::json!({
            "bbox": [115.40, -9.15, 119.45, -8.00],
            "datetime": "2024-01-01T00:00:00Z/2026-05-17T00:00:00Z",
            "limit": 30
        });

        match client.post(url)
            .header("X-API-KEY", &api_token) // Carbon Mapper typically uses X-API-KEY
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
                        // for array check:
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
                                 ON CONFLICT DO NOTHING",
                                chrono::DateTime::parse_from_rfc3339(dt_str).unwrap_or_default().with_timezone(&chrono::Utc),
                                emission_rate,
                                geom_json
                            ).execute(&*pool).await;

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

async fn bmkg_tracker_task(pool: Arc<Pool<Postgres>>) {
    // Schema Migration: Add data_source column if it doesn't exist
    let _ = sqlx::query!("ALTER TABLE weather_observations ADD COLUMN IF NOT EXISTS data_source VARCHAR(50) NOT NULL DEFAULT 'Unknown';")
        .execute(&*pool)
        .await;

    let mut interval = time::interval(Duration::from_secs(3600)); // Hourly
    let zones = vec![
        Zone { name: "Lombok Barat", bmkg_id: "52.01.01.2014", lat: "-8.6818", lon: "116.1240" },
        Zone { name: "Lombok Tengah", bmkg_id: "52.02.01.2001", lat: "-8.7167", lon: "116.2667" },
        Zone { name: "Lombok Timur", bmkg_id: "52.03.01.2001", lat: "-8.6500", lon: "116.5333" },
        Zone { name: "Sumbawa Barat", bmkg_id: "52.07.01.1001", lat: "-8.7333", lon: "116.8500" },
        Zone { name: "Kota Bima", bmkg_id: "52.72.01.1001", lat: "-8.4667", lon: "118.7167" },
    ];
    
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .unwrap_or_default();

    loop {
        interval.tick().await;
        info!("Running Weather Tracker Task for {} zones...", zones.len());

        for zone in &zones {
            let mut success = false;

            // Step A (Primary): BMKG JSON API
            let bmkg_url = format!("https://api.bmkg.go.id/publik/prakiraan-cuaca?adm={}", zone.bmkg_id);
            match client.get(&bmkg_url).send().await {
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
                                    ).execute(&*pool).await;

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
                    tracing::warn!("BMKG failed for {}, falling back to Open-Meteo", zone.name);
                }
            }

            // Step B (Fallback): Open-Meteo API
            if !success {
                let om_url = format!("https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,wind_speed_10m,wind_direction_10m,relative_humidity_2m&wind_speed_unit=ms", zone.lat, zone.lon);
                match client.get(&om_url).send().await {
                    Ok(res) if res.status().is_success() => {
                        match res.json::<OpenMeteoResponse>().await {
                            Ok(om_res) => {
                                let cur = om_res.current;
                                let db_res = sqlx::query!(
                                    "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c, data_source)
                                     VALUES (NOW(), $1, $2, $3, $4, $5, $6)",
                                    zone.name, cur.wind_speed_10m, cur.wind_direction_10m, cur.relative_humidity_2m, cur.temperature_2m, "Open-Meteo"
                                ).execute(&*pool).await;

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
