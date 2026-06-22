-- Enable PostGIS
CREATE EXTENSION IF NOT EXISTS postgis;

-- Table for Tanager-1 Methane Observations (PostGIS focus)
CREATE TABLE methane_observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recorded_at TIMESTAMPTZ NOT NULL,
    emission_rate_kg_hr DOUBLE PRECISION NOT NULL,
    location GEOMETRY(Point, 4326) NOT NULL,
    plume_geometry GEOMETRY(Geometry, 4326),
    source TEXT DEFAULT 'carbon_mapper',
    metadata JSONB,
    CONSTRAINT uq_methane_source UNIQUE (recorded_at, emission_rate_kg_hr)
);

CREATE INDEX idx_methane_location ON methane_observations USING GIST(location);
CREATE INDEX idx_methane_plume_geom ON methane_observations USING GIST(plume_geometry);
CREATE INDEX idx_methane_time ON methane_observations (recorded_at DESC);

-- Table for BMKG Weather Data
CREATE TABLE weather_observations (
    recorded_at TIMESTAMPTZ NOT NULL,
    area_id TEXT NOT NULL,
    wind_speed_ms DOUBLE PRECISION,
    wind_direction_deg DOUBLE PRECISION,
    humidity_percent DOUBLE PRECISION,
    temperature_c DOUBLE PRECISION,
    cloud_cover_percent DOUBLE PRECISION DEFAULT 50.0,
    data_source VARCHAR(50) NOT NULL DEFAULT 'Unknown'
);

CREATE INDEX idx_weather_area_time ON weather_observations (area_id, recorded_at DESC);

-- Table for Weather Forecasts (for plume dispersion prediction)
CREATE TABLE weather_forecasts (
    id SERIAL PRIMARY KEY,
    forecast_at TIMESTAMPTZ NOT NULL,
    valid_at TIMESTAMPTZ NOT NULL,
    area_id TEXT NOT NULL,
    wind_speed_ms DOUBLE PRECISION,
    wind_direction_deg DOUBLE PRECISION,
    humidity_percent DOUBLE PRECISION,
    temperature_c DOUBLE PRECISION,
    data_source VARCHAR(50) NOT NULL DEFAULT 'Open-Meteo',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_forecast_area_time ON weather_forecasts (area_id, valid_at DESC);

-- Populated zones for exposure alert checks (Feature #8)
CREATE TABLE populated_zones (
    id SERIAL PRIMARY KEY,
    zone_name TEXT NOT NULL,
    region TEXT NOT NULL,
    population_estimate INTEGER,
    zone_type VARCHAR(50) NOT NULL DEFAULT 'residential',
    geometry GEOMETRY(Polygon, 4326) NOT NULL,
    is_volcanic_zone BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_populated_zones_geom ON populated_zones USING GIST(geometry);

-- Seed NTB populated zones with approximate boundaries
-- Mataram city center
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Kota Mataram', 'Lombok Barat', 486715, 'urban',
 ST_GeomFromText('POLYGON((116.05 -8.62, 116.17 -8.62, 116.17 -8.58, 116.05 -8.58, 116.05 -8.62))', 4326));

-- Praya (Lombok Tengah)
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Praya', 'Lombok Tengah', 120000, 'urban',
 ST_GeomFromText('POLYGON((116.25 -8.74, 116.30 -8.74, 116.30 -8.70, 116.25 -8.70, 116.25 -8.74))', 4326));

-- Selong (Lombok Timur)
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Selong', 'Lombok Timur', 85000, 'urban',
 ST_GeomFromText('POLYGON((116.52 -8.67, 116.57 -8.67, 116.57 -8.63, 116.52 -8.63, 116.52 -8.67))', 4326));

-- Sumbawa Besar
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Sumbawa Besar', 'Sumbawa Barat', 65000, 'urban',
 ST_GeomFromText('POLYGON((117.40 -8.52, 117.44 -8.52, 117.44 -8.48, 117.40 -8.48, 117.40 -8.52))', 4326));

-- Bima city
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Kota Bima', 'Kota Bima', 155000, 'urban',
 ST_GeomFromText('POLYGON((118.70 -8.48, 118.76 -8.48, 118.76 -8.44, 118.70 -8.44, 118.70 -8.48))', 4326));

-- Senaru (near Rinjani)
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Senaru', 'Lombok Utara', 8000, 'rural',
 ST_GeomFromText('POLYGON((116.38 -8.32, 116.42 -8.32, 116.42 -8.28, 116.38 -8.28, 116.38 -8.32))', 4326));

-- Sembalun (near Rinjani)
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, geometry) VALUES
('Sembalun', 'Lombok Timur', 12000, 'rural',
 ST_GeomFromText('POLYGON((116.48 -8.34, 116.52 -8.34, 116.52 -8.30, 116.48 -8.30, 116.48 -8.34))', 4326));

-- Rinjani volcanic exclusion zone (Feature #20)
INSERT INTO populated_zones (zone_name, region, population_estimate, zone_type, is_volcanic_zone, geometry) VALUES
('Gunung Rinjani Caldera', 'Lombok Utara', 0, 'volcanic_exclusion', TRUE,
 ST_Buffer(ST_SetSRID(ST_MakePoint(116.4550, -8.4117), 4326)::geography, 5000)::geometry);

-- Evacuation alerts log (Feature #6)
CREATE TABLE evacuation_alerts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    triggered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    region TEXT NOT NULL,
    zone_name TEXT,
    emission_rate_kg_hr DOUBLE PRECISION NOT NULL,
    wind_speed_ms DOUBLE PRECISION NOT NULL,
    wind_direction_deg DOUBLE PRECISION NOT NULL,
    concentration_ppm DOUBLE PRECISION,
    plume_polygon GEOMETRY(Polygon, 4326),
    affected_population INTEGER,
    stability_class CHAR(1),
    telegram_sent BOOLEAN NOT NULL DEFAULT FALSE,
    acknowledged_at TIMESTAMPTZ,
    notes TEXT
);

CREATE INDEX idx_alerts_time ON evacuation_alerts (triggered_at DESC);
CREATE INDEX idx_alerts_region ON evacuation_alerts (region, triggered_at DESC);

-- Plume prediction history for temporal animation (Feature #19)
CREATE TABLE plume_predictions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    predicted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    source_observation_id UUID REFERENCES methane_observations(id),
    emission_rate_kg_hr DOUBLE PRECISION NOT NULL,
    wind_speed_ms DOUBLE PRECISION NOT NULL,
    wind_direction_deg DOUBLE PRECISION NOT NULL,
    stability_class CHAR(1) NOT NULL,
    spread_angle_deg DOUBLE PRECISION NOT NULL,
    max_distance_m DOUBLE PRECISION NOT NULL,
    concentration_at_1km_ppm DOUBLE PRECISION,
    plume_polygon GEOMETRY(Polygon, 4326),
    high_uncertainty_smear BOOLEAN NOT NULL DEFAULT FALSE,
    terrain_blocked BOOLEAN NOT NULL DEFAULT FALSE,
    terrain_block_distance_m DOUBLE PRECISION
);

CREATE INDEX idx_predictions_time ON plume_predictions (predicted_at DESC);
CREATE INDEX idx_predictions_geom ON plume_predictions USING GIST(plume_polygon);

-- Sentinel-5P Macro Overpasses (Feature: Early Warning)
CREATE TABLE s5p_overpasses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scene_id TEXT UNIQUE NOT NULL,
    start_datetime TIMESTAMPTZ NOT NULL,
    end_datetime TIMESTAMPTZ NOT NULL,
    orbit_number INTEGER,
    footprint GEOMETRY(Polygon, 4326),
    netcdf_download_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_s5p_time ON s5p_overpasses (start_datetime DESC);
