# GeoESG A.E.C.O - Methane Plume & Dispersion Tracking

Backend system for real-time methane emission tracking and dispersion prediction at **TPA Regional Kebon Kongok, Lombok Barat**.

## Tech Stack
- **Language:** Rust (Axum, Tokio, SQLx)
- **Database:** PostgreSQL + PostGIS + TimescaleDB
- **Data Sources:** 
  - **BMKG:** Real-time wind speed and direction (JSON API).
  - **Sentinel-2 (Element 84):** Satellite imagery metadata over Lombok Barat.
- **Infrastucture:** Docker Compose

## Features
1. **Automated Data Ingestion:**
   - Hourly weather updates from BMKG.
   - Daily STAC metadata tracking for Sentinel-2.
2. **Spatial Analytics:**
   - Real-time plume dispersion prediction using PostGIS `ST_Project` and `ST_MakeLine`.
3. **REST API:**
   - `GET /api/weather`: Latest weather observations.
   - `GET /api/methane`: Latest methane/cloud metrics with GeoJSON locations.
   - `GET /api/plume-prediction`: Projected 1-hour methane trajectory line.

## Setup
1. **Start Database:**
   ```bash
   docker compose up -d
   ```
2. **Configure Environment:**
   Create `.env` file:
   ```env
   DATABASE_URL=postgres://geo_admin:geo_secure_password@localhost:5432/geoesg_aeco
   ```
3. **Run Application:**
   ```bash
   cargo run
   ```

## Target Location
- **Center Point:** -8.645458, 116.091589 (TPA Kebon Kongok)
- **BBOX:** `[116.08, -8.66, 116.10, -8.63]`
