use axum::{extract::State, routing::get, Json, Router};
use dotenvy::dotenv;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{error, info};

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
        stac_tracker_task(pool_stac).await;
    });

    let pool_bmkg = Arc::clone(&shared_pool);
    tokio::spawn(async move {
        bmkg_tracker_task(pool_bmkg).await;
    });

    // API Routes
    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/api/weather", get(get_latest_weather))
        .route("/api/methane", get(get_latest_methane))
        .route("/api/plume-prediction", get(get_plume_prediction))
        .with_state(shared_pool);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Server running on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn get_latest_weather(
    State(pool): State<Arc<Pool<Postgres>>>,
) -> Json<Vec<WeatherObservation>> {
    let records = sqlx::query_as!(
        WeatherObservation,
        "SELECT recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c
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
        )
        SELECT
            m.emission_rate_kg_hr,
            w.wind_speed_ms as "wind_speed_ms!",
            w.wind_direction_deg as "wind_direction_deg!",
            ST_AsGeoJSON(
                ST_MakeLine(
                    m.location::geometry,
                    ST_Project(
                        m.location::geography,
                        (w.wind_speed_ms * 3600.0), 
                        radians(w.wind_direction_deg + 180.0)
                    )::geometry
                )
            ) as "plume_line_json!"
        FROM latest_methane m CROSS JOIN latest_weather w
        "#
    )
    .fetch_optional(&*pool)
    .await
    .unwrap_or_default();
     
    Json(record)
}

async fn stac_tracker_task(pool: Arc<Pool<Postgres>>) {
    let mut interval = time::interval(Duration::from_secs(86400)); // Daily
    let client = reqwest::Client::new();
    let url = "https://earth-search.aws.element84.com/v1/search";

    loop {
        interval.tick().await;
        info!("Running Sentinel-2 STAC Tracker (Element 84)...");

        let payload = serde_json::json!({
            "bbox": [116.08, -8.66, 116.10, -8.63],
            "collections": ["sentinel-2-l2a"],
            "limit": 1
        });

        match client.post(url)
            .json(&payload)
            .send()
            .await {
            Ok(res) => {
                if !res.status().is_success() {
                    error!("STAC API error: {}", res.status());
                    continue;
                }

                match res.json::<StacResponse>().await {
                    Ok(stac) => {
                        if let Some(feature) = stac.features.first() {
                            let dt_str = &feature.properties.datetime;
                            let cloud = feature.properties.cloud_cover;
                            let geom_json = serde_json::to_string(&feature.geometry).unwrap_or_default();

                            let res = sqlx::query!(
                                "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location)
                                 VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)))
                                 ON CONFLICT DO NOTHING",
                                chrono::DateTime::parse_from_rfc3339(dt_str).unwrap_or_default().with_timezone(&chrono::Utc),
                                cloud,
                                geom_json
                            ).execute(&*pool).await;

                            if let Err(e) = res {
                                error!("DB Error (STAC): {}", e);
                            }
                        }
                    }
                    Err(e) => error!("JSON Parse Error (STAC): {}", e),
                }
            }
            Err(e) => error!("Request Error (STAC): {}", e),
        }
    }
}

async fn bmkg_tracker_task(pool: Arc<Pool<Postgres>>) {
    let mut interval = time::interval(Duration::from_secs(3600)); // Hourly
    let url = "https://api.bmkg.go.id/publik/prakiraan-cuaca?adm4=52.01.01.2014";
    
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .unwrap_or_default();

    loop {
        interval.tick().await;
        info!("Running BMKG Weather Tracker...");

        match client.get(url).send().await {
            Ok(res) => {
                if !res.status().is_success() {
                    error!("BMKG API error: {}", res.status());
                    continue;
                }

                match res.json::<BmkgResponse>().await {
                    Ok(bmkg_res) => {
                        if let Some(group) = bmkg_res.data.first() {
                            if let Some(forecast_list) = group.cuaca.first() {
                                if let Some(item) = forecast_list.first() {
                                    let ws_ms = item.ws / 3.6;
                                    let area_id = "52.01.01.2014";

                                    let res = sqlx::query!(
                                        "INSERT INTO weather_observations (recorded_at, area_id, wind_speed_ms, wind_direction_deg, humidity_percent, temperature_c)
                                         VALUES (NOW(), $1, $2, $3, $4, $5)",
                                        area_id, ws_ms, item.wd_deg, item.hu, item.t
                                    ).execute(&*pool).await;

                                    if let Err(e) = res {
                                        error!("DB Error (BMKG): {}", e);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => error!("JSON Parse Error (BMKG): {}", e),
                }
            }
            Err(e) => error!("Request Error (BMKG): {}", e),
        }
    }
}
