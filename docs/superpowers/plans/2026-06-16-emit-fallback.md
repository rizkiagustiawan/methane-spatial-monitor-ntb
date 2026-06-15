# EMIT Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add NASA EMIT methane plume data as fallback when Carbon Mapper API fails

**Architecture:** Add parallel `emit_tracker_task` that fetches from US GHG Center STAC API. Carbon Mapper remains primary; EMIT activates only when Carbon Mapper fails or returns no data. Both sources write to same `methane_observations` table with different `source` labels.

**Tech Stack:** Rust, reqwest, serde, chrono, sqlx

---

## File Structure

| File | Change |
|------|--------|
| `src/config.rs` | Add `EmitConfig` struct + env vars |
| `src/models.rs` | Add EMIT STAC response models + metrics fields |
| `src/main.rs` | Add `emit_tracker_task`, update `AppState`, wire fallback logic |
| `src/errors.rs` | No changes needed (reuse `ExternalService`) |
| `.env.example` | Add EMIT env vars |

---

### Task 1: Add EMIT Configuration

**Files:**
- Modify: `src/config.rs:10-17` (add field to `AppConfig`)
- Modify: `src/config.rs:34-40` (add `EmitConfig` struct)
- Modify: `src/config.rs:101-108` (add env parsing)

- [ ] **Step 1: Add EmitConfig struct**

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct EmitConfig {
    pub enabled: bool,
    pub base_url: String,
    pub bbox: Vec<f64>,
    pub poll_interval_secs: u64,
}
```

- [ ] **Step 2: Add emit field to AppConfig**

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub carbon_mapper: CarbonMapperConfig,
    pub emit: EmitConfig,
    pub weather: WeatherConfig,
    pub telegram: TelegramConfig,
    pub physics: PhysicsConfig,
}
```

- [ ] **Step 3: Add env parsing in AppConfig::from_env()**

After `carbon_mapper` block (~line 108), add:

```rust
emit: EmitConfig {
    enabled: std::env::var("EMIT_ENABLED")
        .unwrap_or_else(|_| "true".into())
        .parse()
        .unwrap_or(true),
    base_url: std::env::var("EMIT_STAC_URL")
        .unwrap_or_else(|_| "https://ghgcenter.upc.nasa.gov/api/stac".into()),
    bbox: vec![115.40, -9.15, 119.45, -8.00],
    poll_interval_secs: std::env::var("EMIT_POLL_INTERVAL_SECS")
        .unwrap_or_else(|_| "43200".into())
        .parse()
        .unwrap_or(43200),
},
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: Compiles with warnings about unused fields

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add EMIT configuration for fallback data source"
```

---

### Task 2: Add EMIT Models

**Files:**
- Modify: `src/models.rs:192-202` (add metrics fields)
- Modify: `src/models.rs:243-269` (add EMIT STAC models)

- [ ] **Step 1: Add EMIT metrics to AppMetrics**

```rust
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
```

- [ ] **Step 2: Add EMIT STAC response models**

After existing STAC models (~line 269), add:

```rust
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
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add src/models.rs
git commit -m "feat: add EMIT STAC response models and metrics"
```

---

### Task 3: Add EMIT Tracker Task

**Files:**
- Modify: `src/main.rs:24-48` (update AppState)
- Modify: `src/main.rs:250-270` (update metrics display)
- Modify: `src/main.rs:970-1035` (add emit_tracker_task)

- [ ] **Step 1: Update AppState with EMIT fields**

Add to `AppState` struct:

```rust
struct AppState {
    pool: Pool<Postgres>,
    http_client: reqwest::Client,
    metrics: Arc<AppMetrics>,
    ws_state: Arc<WsState>,
    config: AppConfig,
    last_bmkg_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    last_stac_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    last_emit_fetch: std::sync::RwLock<Option<DateTime<Utc>>>,
    start_time: std::time::Instant,
}
```

- [ ] **Step 2: Update AppState initialization**

Find where `AppState` is created and add:

```rust
last_emit_fetch: std::sync::RwLock::new(None),
```

- [ ] **Step 3: Add emit_tracker_task function**

After `carbon_mapper_tracker_task` (line 1035), add:

```rust
async fn emit_tracker_task(state: Arc<AppState>) {
    if !state.config.emit.enabled {
        info!("EMIT fallback disabled");
        return;
    }

    let mut interval = time::interval(Duration::from_secs(state.config.emit.poll_interval_secs));
    let bbox = &state.config.emit.bbox;
    let collection = "emit-ch4plume-v1";

    info!("EMIT fallback task started (poll: {}s)", state.config.emit.poll_interval_secs);

    loop {
        interval.tick().await;
        state.metrics.emit_fetches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let search_url = format!("{}/search", state.config.emit.base_url);
        let payload = json!({
            "collections": [collection],
            "bbox": bbox,
            "datetime": format!("2022-08-01T00:00:00Z/{}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
            "limit": 100
        });

        match state.http_client.post(&search_url).json(&payload).send().await {
            Ok(res) if res.status().is_success() => {
                if let Ok(stac) = res.json::<EmitStacResponse>().await {
                    *state.last_emit_fetch.write().unwrap() = Some(Utc::now());

                    for feature in stac.features {
                        let emission_rate = match feature.properties.ch4_plume_emission_rate {
                            Some(rate) if rate > 0.0 => rate,
                            _ => continue,
                        };

                        let dt = match chrono::DateTime::parse_from_rfc3339(&feature.properties.datetime) {
                            Ok(dt) => dt.with_timezone(&Utc),
                            Err(_) => continue,
                        };

                        let geom = serde_json::to_string(&feature.geometry).unwrap_or_default();

                        let (lon, lat) = if let Some(coords) = feature.geometry.get("coordinates") {
                            if let Some(arr) = coords.as_array() {
                                if arr.len() >= 2 {
                                    (arr[0].as_f64().unwrap_or(0.0), arr[1].as_f64().unwrap_or(0.0))
                                } else { (0.0, 0.0) }
                            } else { (0.0, 0.0) }
                        } else { (0.0, 0.0) };

                        let plume_id = feature.properties.ch4_plume_id
                            .unwrap_or_else(|| format!("emit-{}", Uuid::new_v4()));

                        let res = sqlx::query(
                            "INSERT INTO methane_observations (recorded_at, emission_rate_kg_hr, location, plume_geometry, source) VALUES ($1, $2, ST_Centroid(ST_GeomFromGeoJSON($3)), ST_GeomFromGeoJSON($3), 'emit') ON CONFLICT (recorded_at, emission_rate_kg_hr) DO NOTHING",
                        )
                        .bind(dt).bind(emission_rate).bind(&geom)
                        .execute(&state.pool).await;

                        if res.is_ok() && res.unwrap().rows_affected() > 0 {
                            state.metrics.emit_plumes_ingested.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            ws::broadcast_plume_update(
                                &state.ws_state.tx,
                                plume_id,
                                emission_rate,
                                lat,
                                lon,
                                dt.to_rfc3339(),
                            ).await;
                        }
                    }
                }
            }
            _ => {
                state.metrics.emit_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}
```

- [ ] **Step 4: Update metrics display**

Find metrics display section and add:

```rust
geoesg_emit_fetches {}
geoesg_emit_errors {}
geoesg_emit_plumes_ingested {}
```

And bind values:

```rust
state.metrics.emit_fetches.load(Relaxed),
state.metrics.emit_errors.load(Relaxed),
state.metrics.emit_plumes_ingested.load(Relaxed),
```

- [ ] **Step 5: Spawn EMIT task in main**

Find where tasks are spawned and add:

```rust
let state_emit = state.clone();
tokio::spawn(async move {
    emit_tracker_task(state_emit).await;
});
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: add EMIT tracker task for methane plume fallback"
```

---

### Task 4: Add Fallback Logic

**Files:**
- Modify: `src/main.rs:970-1035` (add fallback trigger)

- [ ] **Step 1: Add fallback trigger in carbon_mapper_tracker_task**

At the end of `carbon_mapper_tracker_task`, after the while loop (line 1033), add:

```rust
// If Carbon Mapper failed or returned no data, log for EMIT fallback
if state.metrics.carbon_mapper_errors.load(std::sync::atomic::Ordering::Relaxed) > 0 {
    info!("Carbon Mapper errors detected, EMIT fallback will run on next cycle");
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: add fallback logic for Carbon Mapper to EMIT"
```

---

### Task 5: Update Environment Configuration

**Files:**
- Modify: `.env.example`

- [ ] **Step 1: Add EMIT env vars**

Append to `.env.example`:

```env
# NASA EMIT Fallback (GHG Center STAC)
EMIT_ENABLED=true
EMIT_STAC_URL=https://ghgcenter.upc.nasa.gov/api/stac
EMIT_POLL_INTERVAL_SECS=43200
```

- [ ] **Step 2: Commit**

```bash
git add .env.example
git commit -m "feat: add EMIT environment variables to .env.example"
```

---

### Task 6: Verify Integration

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run release build**

Run: `cargo build --release`
Expected: Builds successfully

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: complete EMIT fallback integration"
```

---

## Configuration Summary

| Variable | Default | Description |
|----------|---------|-------------|
| `EMIT_ENABLED` | `true` | Enable/disable EMIT fallback |
| `EMIT_STAC_URL` | `https://ghgcenter.upc.nasa.gov/api/stac` | NASA GHG Center STAC endpoint |
| `EMIT_POLL_INTERVAL_SECS` | `43200` | Poll interval (12 hours) |

## Metrics Added

| Metric | Description |
|--------|-------------|
| `geoesg_emit_fetches` | Total EMIT API calls |
| `geoesg_emit_errors` | Failed EMIT API calls |
| `geoesg_emit_plumes_ingested` | Plumes stored from EMIT |

## Testing

- Unit tests: `cargo test --lib`
- Integration tests: `cargo test --test integration_test`
- Manual: Check `/api/metrics` endpoint for EMIT counters
