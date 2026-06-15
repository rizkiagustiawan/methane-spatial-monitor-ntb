# NTB Methane Tracker - Geo ESG A.E.C.O

Advanced real-time methane emission tracking, Gaussian dispersion prediction, and exposure alerting system for **West Nusa Tenggara (NTB)**, rigorously bound by the physics limits of remote sensing.

## Tech Stack
- **Backend:** Rust (Axum 0.7, Tokio, SQLx, GDAL)
- **Frontend:** HTML5, Leaflet.js, CartoDB Dark Matter
- **Database:** PostgreSQL + PostGIS (Spatial Intelligence)
- **Data Sources:** 
  - **Carbon Mapper:** STAC API (Tanager-1 instrument) for high-resolution methane plumes.
  - **NASA EMIT:** GHG Center STAC API for methane plume fallback (ISS-based).
  - **BMKG:** Real-time weather API (Wind, Temp, Humidity) across 10 NTB regions.
  - **Open-Meteo:** Secondary fallback for regional weather data + forecast.
  - **SRTM/DEM:** 30m Digital Elevation Model for terrain-aware tracking.

## Key Features

### 1. **Observed vs Forecast Architecture**
- **Observed Plumes:** Direct satellite footprint from Carbon Mapper Tanager-1 (30m resolution).
- **Forecasted Dispersion:** Gaussian plume prediction using weather forecasts (6 hours ahead).
- Separate visualization layers for observed and forecasted plumes.

### 2. **Physics-Bound Dispersion Engine:**
- **Gaussian Plume Modeling:** Calculates ground-level center-line concentrations (ppm) using precise Pasquill-Gifford stability classes mapped to WITA timezone.
- **Terrain Blocking:** Ray-casts along the plume trajectory over the DEM raster. Truncates dispersion if terrain rises >15m relative to the emission source.
- **Atmospheric Extinction:** Simplified humidity attenuation model for high humidity (>85%) conditions.
- **Thermal Stability Correction:** T⁴ penalty factor for high thermal backgrounds.
- **Optomechanical Limits:** Flags MTF degradation (smear) based on Tanager-1 satellite roll/pitch thresholds (6.85°/4.8°).

### 3. **STAC API (SpatioTemporal Asset Catalog)**
- Full STAC 1.0.0 compliance for Earth Observation data interoperability.
- Endpoints: `/api/stac`, `/api/stac/collections`, `/api/stac/search`.
- Compatible with STAC ecosystem tools and libraries.

### 4. **Real-time WebSocket**
- Live plume updates, weather notifications, and alerts.
- Endpoint: `/ws`
- Supports subscribe/unsubscribe to specific regions.

### 5. **Automated Data Pipeline & Alerts:**
- Background tasks handle paginated STAC ingestion and multi-source weather fetching.
- Weather forecast integration (Open-Meteo, 2-day hourly forecast).
- Triggers real-time **Telegram Evacuation Alerts** when toxic concentration footprints intersect mapped populated areas (e.g., Kota Mataram, Sembalun).
- Features automated 30-day weather data retention policies.

### 6. **Resilient Architecture:**
- Typed error handling with `thiserror` (8 error types).
- Enforced API rate-limiting (100 req/sec) via `tower-governor`.
- Real-time `/api/metrics` (Prometheus-ready) and `/api/stats` endpoints.
- Graceful shutdown support.

## Quick Start

### 1. **Infrastructure:**
```bash
# Use the template to create your config
cp .env.example .env
# Add your passwords and API tokens (Carbon Mapper, Telegram) to .env

# Start the PostGIS database
docker compose up -d
```

### 2. **Launch:**
```bash
cargo run --release
```
Access the dashboard at: **http://localhost:3000**

### 3. **Run Tests:**
```bash
cargo test
```

## Project Architecture

```
src/
├── main.rs       # Central routing, physics algorithms, async background workers
├── models.rs     # Type-safe structs mapping to PostGIS geometries and JSON payloads
├── errors.rs     # Typed error handling with thiserror
├── stac.rs       # STAC 1.0.0 data model and API handlers
└── ws.rs         # WebSocket real-time communication

tests/
└── integration_test.rs  # Integration tests

frontend/
└── index.html    # Map-centric dashboard with observed/forecast layers

init.sql          # Database schema (PostGIS)
openapi.yaml      # API documentation (OpenAPI 3.0.3)
```

## API Endpoints

### Core API
| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | System health check |
| GET | `/api/metrics` | Prometheus metrics |
| GET | `/api/stats` | System statistics |
| GET | `/api/weather` | Latest weather observations |
| GET | `/api/weather/forecast` | Weather forecasts (2 days) |
| GET | `/api/methane/plumes` | Recent methane plumes |
| GET | `/api/plume-analysis` | Plume analysis (observed + forecast) |
| GET | `/api/zones` | Populated zones (GeoJSON) |

### STAC API (SpatioTemporal Asset Catalog)
| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/stac` | STAC catalog root |
| GET | `/api/stac/collections` | List collections |
| GET | `/api/stac/collections/methane-observations` | Collection detail |
| GET | `/api/stac/collections/methane-observations/items` | Collection items |
| GET | `/api/stac/search` | STAC search |

### WebSocket
| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/ws` | WebSocket connection for real-time updates |

## Detection Thresholds

Based on Tanager-1 satellite specifications (Source: Carbon Mapper Product Guide v1.1.6):
- **90% Probability of Detection:** 90-180 kg/hr (3 m/s wind, 35° SZA, 25% albedo, 30m GSD)
- **Conservative Threshold:** 150 kg/hr (for reliable detection)
- **Spatial Resolution:** 30m GSD
- **Geolocation Accuracy:** 50m (CE90)
- **Spectral Range:** 400-2500nm (5nm sampling, 5.5nm FWHM)
- **SNR @ 2200nm:** 310-655

## Physics Constraints

| Parameter | Value | Source |
|-----------|-------|--------|
| Detection Limit (90%) | 90-180 kg/hr | Carbon Mapper Product Guide |
| Detection Limit (Conservative) | 150 kg/hr | Carbon Mapper Product Guide |
| Sensor Roll Limit | 6.85° | Physics Limits document |
| Sensor Pitch Limit | 4.8° | Physics Limits document |
| Terrain Blocking Threshold | 15m | Simplified terrain model |
| Humidity Attenuation | 85% threshold | Simplified atmospheric model |
| Geolocation Accuracy | 50m CE90 | Carbon Mapper Product Guide |

## Environment Variables

```env
# Database
DATABASE_URL=postgres://geo_admin:password@localhost:5432/geoesg_aeco
POSTGRES_USER=geo_admin
POSTGRES_PASSWORD=your_password
POSTGRES_DB=geoesg_aeco

# Carbon Mapper STAC API
CARBON_MAPPER_TOKEN=your_token

# Telegram Alerts
TELEGRAM_BOT_TOKEN=your_bot_token
TELEGRAM_CHAT_ID=your_chat_id

# Sensor Telemetry (optional)
SENSOR_ROLL_DEG=5.0
SENSOR_PITCH_DEG=2.0
```

## License

Proprietary - GeoESG A.E.C.O

## Acknowledgments

- **Carbon Mapper** for Tanager-1 satellite data
- **BMKG** for weather data
- **Open-Meteo** for weather forecast API
- **ESA Copernicus** for Earth observation standards
