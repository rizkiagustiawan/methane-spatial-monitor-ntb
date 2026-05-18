# NTB Methane Tracker - GeoESG A.E.C.O

Advanced real-time methane emission tracking and dispersion prediction system for **TPA Regional Kebon Kongok, Lombok Barat, NTB**.

## Tech Stack
- **Backend:** Rust (Axum, Tokio, SQLx)
- **Frontend:** HTML5, Leaflet.js, CartoDB Dark Matter
- **Database:** PostgreSQL + PostGIS (Spatial Intelligence)
- **Data Sources:** 
  - **BMKG:** Real-time weather API (Wind & Temp).
  - **Carbon Mapper:** STAC API for high-resolution methane plume detection.
  - **Open-Meteo:** Secondary fallback for regional weather data.

## Key Features
1. **Interactive Dashboard:**
   - Real-time visualization of methane plumes on a dark-themed geographic map.
   - Live prediction of gas dispersion footprints.
2. **Gaussian Plume Dispersion:**
   - Scientifically rigorous 1-hour dispersion modeling.
   - Uses Pasquill-Gifford Class D stability approximation to generate 2D polygon footprints.
3. **Automated Data Pipeline:**
   - Hourly weather syncing from BMKG.
   - Automated STAC metadata tracking for methane observations.
4. **Spatial API:**
   - `GET /`: Serves the interactive map dashboard.
   - `GET /api/methane/plumes`: Returns detected plumes as GeoJSON.
   - `GET /api/plume-prediction`: Returns the 2D Gaussian dispersion footprint.

## Quick Start
1. **Infrastructure:**
   ```bash
   docker compose up -d
   ```
2. **Configuration:**
   Add required tokens to your `.env`:
   ```env
   DATABASE_URL=postgres://geo_admin:geo_secure_password@localhost:5432/geoesg_aeco
   CARBON_MAPPER_TOKEN=your_token_here
   ```
3. **Launch:**
   ```bash
   cargo run
   ```
   Access the dashboard at: **http://localhost:3000**

## Technical Methodology
- **Dispersion:** Triangular wedge calculation using `ST_Project` and `ST_MakePolygon` based on a 12.5-degree dispersion angle.
- **Backend Architecture:** Asynchronous task spawning for background data tracking without blocking the API server.
