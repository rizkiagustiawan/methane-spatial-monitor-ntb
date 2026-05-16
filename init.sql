-- Enable PostGIS and TimescaleDB
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Table for Tanager-1 Methane Observations (PostGIS focus)
CREATE TABLE methane_observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recorded_at TIMESTAMPTZ NOT NULL,
    emission_rate_kg_hr DOUBLE PRECISION NOT NULL,
    location GEOMETRY(Point, 4326) NOT NULL,
    metadata JSONB
);

CREATE INDEX idx_methane_location ON methane_observations USING GIST(location);
CREATE INDEX idx_methane_time ON methane_observations (recorded_at DESC);

-- Table for BMKG Weather Data (TimescaleDB Hypertable focus)
CREATE TABLE weather_observations (
    recorded_at TIMESTAMPTZ NOT NULL,
    area_id TEXT NOT NULL,
    wind_speed_ms DOUBLE PRECISION,
    wind_direction_deg DOUBLE PRECISION,
    humidity_percent DOUBLE PRECISION,
    temperature_c DOUBLE PRECISION
);

-- Convert to Hypertable for optimized time-series queries
SELECT create_hypertable('weather_observations', 'recorded_at');

CREATE INDEX idx_weather_area_time ON weather_observations (area_id, recorded_at DESC);
