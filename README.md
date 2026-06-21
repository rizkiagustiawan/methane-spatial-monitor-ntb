# NTB Methane Tracker - Geo ESG A.E.C.O

Advanced real-time methane emission tracking, Gaussian dispersion prediction, and exposure alerting system for **West Nusa Tenggara (NTB)**, rigorously bound by the physics limits of remote sensing.

## Tech Stack
- **Backend:** Rust (Axum 0.7, Tokio, SQLx, GDAL)
- **Frontend:** HTML5, Leaflet.js, CartoDB Dark Matter
- **Database:** PostgreSQL + PostGIS (Spatial Intelligence)
- **Data Sources:** 
  - **Carbon Mapper:** STAC API (Tanager-1 instrument) for high-resolution methane plumes.
  - **NASA EMIT:** GHG Center STAC API for methane plume fallback (ISS-based).
  - **Sentinel-5P:** Microsoft Planetary Computer STAC API for macro-scale early warning.
  - **BMKG:** Real-time weather API (Wind, Temp, Humidity, Cloud Cover) across 110 NTB zones.
  - **Open-Meteo:** Secondary fallback for regional weather data + forecast (110 zones batch).
  - **SRTM/DEM:** 30m Digital Elevation Model for terrain-aware tracking.

## Key Features

### 1. **Observed vs Forecast Architecture**
- **Observed Plumes:** Direct satellite footprint from Carbon Mapper Tanager-1 (30m resolution).
- **Forecasted Dispersion:** Gaussian plume prediction using weather forecasts (6 hours ahead).
- Separate visualization layers for observed and forecasted plumes.

### 2. **Physics-Bound Dispersion Engine:**
- **Gaussian Plume Modeling:** Calculates ground-level center-line concentrations (ppm) using precise Pasquill-Gifford stability classes mapped to WITA timezone.
- **Terrain Blocking:** Ray-casts along the plume trajectory over the DEM raster. Truncates dispersion if terrain rises >15m relative to the emission source.
- **Atmospheric Extinction:** Beer-Lambert Law for CH4 at 2200nm (HITRAN Database).
- **Humidity Attenuation:** Water vapor absorption above 85% humidity threshold.
- **Wind Uncertainty Propagation:** ±1.5 m/s error propagation (Conrad & Johnson, 2026).
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
- **EMIT Fallback:** NASA EMIT STAC API as fallback when Carbon Mapper fails.
- **Sentinel-5P:** Macro-scale early warning via Microsoft Planetary Computer.
- **AI Data Fusion:** Fills temporal gaps when high-res satellites are occluded by clouds, crossing S5P macro detections with historical point-source data.
- Weather forecast integration (Open-Meteo, 2-day hourly forecast).
- Triggers real-time **Telegram Evacuation Alerts** when toxic concentration footprints intersect mapped populated areas (e.g., Kota Mataram, Sembalun).
- Features automated 30-day weather data retention policies.

### 6. **Resilient Architecture:**
- Typed error handling with `thiserror` (8 error types).
- Enforced API rate-limiting (100 req/sec) via `tower-governor`.
- Real-time `/api/metrics` (Prometheus-ready) and `/api/stats` endpoints.
- Graceful shutdown support.
- Concurrent BMKG requests (10 parallel) for faster data ingestion.
- Batch Open-Meteo requests (110 zones in 1 API call).

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
├── main.rs         # Central routing, physics algorithms, async background workers
├── models.rs       # Type-safe structs mapping to PostGIS geometries and JSON payloads
├── errors.rs       # Typed error handling with thiserror
├── physics.rs      # Physics constants, HITRAN data, Gaussian plume formulas
├── services.rs     # Business logic layer
├── repositories.rs # Database access layer
├── stac.rs         # STAC 1.0.0 data model and API handlers
├── ws.rs           # WebSocket real-time communication
├── config.rs       # Configuration with validation
└── lib.rs          # Module exports

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
| GET | `/api/plume-prediction` | Multi-plume prediction |
| GET | `/api/zones` | Populated zones (GeoJSON) |
| GET | `/api/s5p` | Sentinel-5P overpasses |
| GET | `/api/fusion` | Data Fusion Anomalies (S5P + High-Res gaps) |

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
| Humidity Attenuation | 85% threshold | HITRAN Database |
| Geolocation Accuracy | 50m CE90 | Carbon Mapper Product Guide |
| Wind Uncertainty | ±1.5 m/s | Conrad & Johnson (2026) |
| Wind Shear Coefficient | 0.17 | Vollrath et al. (2026) |
| CH4 Absorption (2200nm) | 1.0e-21 cm²/molecule | HITRAN Database |

## Data Sources & Coverage

### BMKG Weather Data (110 zones)
| Kab/Kota | Kecamatan | Kode adm4 |
|----------|-----------|-----------|
| Lombok Barat | 8 | 52.01.01.2014 - 52.01.08.2001 |
| Lombok Tengah | 12 | 52.02.01.1001 - 52.02.12.2001 |
| Lombok Timur | 20 | 52.03.01.2001 - 52.03.20.2001 |
| Lombok Utara | 5 | 52.08.01.2001 - 52.08.05.2001 |
| Sumbawa Barat | 6 | 52.07.01.2002 - 52.07.06.2001 |
| Sumbawa | 24 | 52.04.02.2001 - 52.04.28.2001 |
| Dompu | 8 | 52.05.01.1001 - 52.05.08.2003 |
| Bima | 16 | 52.06.01.2005 - 52.06.18.2001 |
| Kota Mataram | 6 | 52.71.01.1001 - 52.71.06.1001 |
| Kota Bima | 5 | 52.72.01.1001 - 52.72.05.1001 |

### Open-Meteo Weather Data
- **Coverage:** 110 zones (same as BMKG)
- **Batch Request:** All zones in 1 API call (bypass rate limits)
- **Forecast:** 2-day hourly forecast
- **Timezone:** Asia/Makassar (WITA)

### Satellite Data Sources
| Source | Instrument | Resolution | Coverage |
|--------|------------|------------|----------|
| Carbon Mapper | Tanager-1 | 30m GSD | STAC API |
| NASA EMIT | ISS-based | 60m GSD | GHG Center STAC |
| Sentinel-5P | TROPOMI | 7x5.5 km² | Planetary Computer |

## Riset & Referensi

### Paper yang Divalidasi (2024-2026)
| Paper | Tahun | Jurnal | Topik |
|-------|-------|--------|-------|
| Vollrath et al. | 2026 | AMT | Wind shear coefficient, power law |
| Wietzel et al. | 2025 | AMT | PG classification, wind uncertainty |
| Suzuki | 2025 | J. Nuclear Sci. Tech. | Cloud cover untuk PG classification |
| Li et al. | 2026 | Environments | Gaussian Plume, dispersion coefficients |
| Conrad & Johnson | 2026 | AMT | Wind uncertainty propagation |
| MethaneSAT | 2026 | ACP | Detection limits, mass-balance |
| Gao et al. | 2026 | ACP | Terrain effects on Gaussian Plume |
| Batur et al. | 2026 | Progress in Nuclear Energy | Terrain-modified Gaussian Plume |

### Physics References
| Reference | Topik |
|-----------|-------|
| Turner (1970) | Pasquill-Gifford stability classes |
| ISC3 Manual | Gaussian Plume Model |
| Briggs (1973) | Dispersion coefficients |
| HITRAN Database | CH4 absorption at 2200nm |
| Carbon Mapper Product Guide v1.1.6 | Tanager-1 specifications |
| Radiative Transfer Theory | Beer-Lambert Law |

## Environment Variables

```env
# Database
DATABASE_URL=postgres://geo_admin:password@localhost:5432/geoesg_aeco
POSTGRES_USER=geo_admin
POSTGRES_PASSWORD=your_password
POSTGRES_DB=geoesg_aeco

# Carbon Mapper STAC API
CARBON_MAPPER_TOKEN=your_token

# NASA EMIT Fallback (GHG Center STAC)
EMIT_ENABLED=true
EMIT_STAC_URL=https://ghgcenter.upc.nasa.gov/api/stac
EMIT_BBOX=115.40,-9.15,119.45,-8.00
EMIT_POLL_INTERVAL_SECS=43200

# Sentinel-5P (Microsoft Planetary Computer)
S5P_ENABLED=true
S5P_STAC_URL=https://planetarycomputer.microsoft.com/api/stac/v1
S5P_POLL_INTERVAL_SECS=21600

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
- **NASA EMIT** for methane plume data (ISS-based)
- **Microsoft Planetary Computer** for Sentinel-5P data
- **BMKG** for weather data across NTB
- **Open-Meteo** for weather forecast API
- **ESA Copernicus** for Earth observation standards
- **HITRAN Database** for CH4 absorption data
- **Turner (1970)** for Pasquill-Gifford stability classes
- **Briggs (1973)** for dispersion coefficients
- **Vollrath et al. (2026)** for wind shear coefficient
- **Conrad & Johnson (2026)** for wind uncertainty propagation
