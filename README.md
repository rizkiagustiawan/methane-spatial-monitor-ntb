# NTB Methane Tracker - Geo ESG A.E.C.O

Advanced real-time methane emission tracking, Gaussian dispersion prediction, and exposure alerting system for **West Nusa Tenggara (NTB)**, rigorously bound by the physics limits of remote sensing.

## Tech Stack
- **Backend:** Rust (Axum, Tokio, SQLx, GDAL)
- **Frontend:** HTML5, Leaflet.js, CartoDB Dark Matter
- **Database:** PostgreSQL + PostGIS (Spatial Intelligence)
- **Data Sources:** 
  - **Carbon Mapper:** STAC API (Tanager-1 instrument) for high-resolution methane plumes.
  - **BMKG:** Real-time weather API (Wind, Temp, Humidity) across 10 NTB regions.
  - **Open-Meteo:** Secondary fallback for regional weather data.
  - **SRTM/DEM:** 30m Digital Elevation Model for terrain-aware tracking.

## Key Features
1. **Interactive Dashboard:**
   - Real-time visualization of methane plumes, dynamic dispersion footprints, and vulnerable populated zones.
   - Built-in metrics panel, dynamic legend, and flashing exposure warning banners.
2. **Physics-Bound Dispersion Engine:**
   - **Gaussian Plume Modeling:** Calculates ground-level center-line concentrations (ppm) using precise Pasquill-Gifford stability classes mapped to WITA timezone.
   - **Terrain Blocking:** Ray-casts along the plume trajectory over the DEM raster. Truncates dispersion if terrain rises >15m relative to the emission source.
   - **Thermodynamic Extinction:** Implements 40% distance attenuation in high humidity (>85%) and applies Stefan-Boltzmann T⁴ scaling during high thermal backgrounds.
   - **Optomechanical Limits:** Flags MTF degradation (smear) based on Tanager-1 satellite roll/pitch thresholds.
3. **Automated Data Pipeline & Alerts:**
   - Background tasks handle paginated STAC ingestion and multi-source weather fetching.
   - Triggers real-time **Telegram Evacuation Alerts** when toxic concentration footprints intersect mapped populated areas (e.g., Kota Mataram, Sembalun).
   - Features automated 30-day weather data retention policies.
4. **Resilient Architecture:**
   - Enforced API rate-limiting (100 req/sec) via `tower-governor`.
   - Real-time `/api/metrics` (Prometheus-ready) and `/api/stats` endpoints.

## Quick Start
1. **Infrastructure:**
   ```bash
   # Use the template to create your config
   cp .env.example .env
   # Add your passwords and API tokens (Carbon Mapper, Telegram) to .env
   
   # Start the PostGIS database
   docker compose up -d
   ```
2. **Launch:**
   ```bash
   cargo run --release
   ```
   Access the dashboard at: **http://localhost:3000**

## Project Architecture
- `src/main.rs`: Central routing, physics algorithms, and async background workers.
- `src/models.rs`: Type-safe structs mapping to PostGIS geometries and JSON payloads.
- `init.sql`: Relational schema including `populated_zones`, `evacuation_alerts`, and `plume_predictions`.
- `frontend/index.html`: Lightweight, map-centric dashboard with 60-second polling.
