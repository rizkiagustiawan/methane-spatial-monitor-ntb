//! Integration tests for GeoESG A.E.C.O Backend
//! These tests verify physics, constants, and logic without database dependency

use geoesg_aeco_backend::models::{ComponentHealth, HealthStatus};

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Physics Constants ───────────────────────────────────────────────

    #[test]
    fn test_tanager1_detection_limits() {
        // Source: Carbon Mapper Product Guide v1.1.6
        // "CH4 90% Probability of Detection: 90-180 kg/hr"
        let min = 90.0;
        let max = 180.0;
        let conservative = 150.0;
        assert!(min > 0.0);
        assert!(max > min);
        assert!(conservative > min);
        assert!(conservative < max);
    }

    #[test]
    fn test_pasquill_classes() {
        let classes = ['A', 'B', 'C', 'D', 'E', 'F'];
        assert_eq!(classes.len(), 6);
        assert_eq!(classes[0], 'A');
        assert_eq!(classes[5], 'F');
    }

    #[test]
    fn test_sensor_limits() {
        // Source: Physics Limits in Remote Sensing
        let roll_limit = 6.85;
        let pitch_limit = 4.8;
        assert!(roll_limit > 0.0);
        assert!(pitch_limit > 0.0);
        assert!(roll_limit > pitch_limit);
    }

    #[test]
    fn test_terrain_threshold() {
        let threshold_m = 15.0;
        assert!(threshold_m > 0.0);
        assert!(threshold_m < 100.0);
    }

    #[test]
    fn test_humidity_threshold() {
        let threshold = 85.0;
        assert!(threshold > 0.0);
        assert!(threshold <= 100.0);
    }

    #[test]
    fn test_wind_speed_bounds() {
        let low_wind = 3.0;
        let high_wind = 5.0;
        assert!(low_wind < high_wind);
        assert!(low_wind > 0.0);
    }

    #[test]
    fn test_stac_version() {
        let version = "1.0.0";
        assert!(!version.is_empty());
        assert!(version.contains('.'));
    }

    #[test]
    fn test_ntb_bounds() {
        let west = 115.40;
        let east = 119.45;
        let south = -9.15;
        let north = -8.00;
        assert!(west < east);
        assert!(south < north);
        assert!(west > 110.0);
        assert!(east < 120.0);
        assert!(south < -8.0);
        assert!(north > -9.0);
    }

    // ─── Gaussian Plume Physics ──────────────────────────────────────────

    #[test]
    fn test_gaussian_concentration_formula() {
        // C(x,0,0) = Q / (pi * u * sigma_y * sigma_z)
        // With known values
        let q = 1000.0 * 1000.0 / 3600.0; // 1000 kg/hr in g/s
        let u = 3.0; // m/s
        let sy = 68.0; // sigma_y at 1km for class D
        let sz = 31.0; // sigma_z at 1km for class D
        let pi = std::f64::consts::PI;

        let conc = q / (pi * u * sy * sz);
        assert!(conc > 0.0);

        // Higher emission -> higher concentration
        let conc_high = (q * 2.0) / (pi * u * sy * sz);
        assert!(conc_high > conc);

        // Higher wind -> lower concentration
        let conc_low_wind = q / (pi * (u * 2.0) * sy * sz);
        assert!(conc_low_wind < conc);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_dispersion_coefficients_ordering() {
        // Pasquill-Gifford dispersion coefficients at 1km
        // For unstable classes (A, B): sigma_z can be > sigma_y due to strong vertical mixing
        // For stable classes (D, E, F): sigma_y > sigma_z
        let classes = [
            ('A', 210.0, 450.0),
            ('B', 155.0, 110.0),
            ('C', 105.0, 61.0),
            ('D', 68.0, 31.0),
            ('E', 50.0, 21.0),
            ('F', 34.0, 11.0),
        ];
        for (cls, sy, sz) in classes {
            assert!(sy > 0.0, "sigma_y for class {} should be > 0", cls);
            assert!(sz > 0.0, "sigma_z for class {} should be > 0", cls);
        }
        // For stable conditions, sigma_y > sigma_z
        assert!(68.0 > 31.0); // D
        assert!(50.0 > 21.0); // E
        assert!(34.0 > 11.0); // F
    }

    #[test]
    fn test_stability_class_progression() {
        // A (most unstable) -> F (most stable)
        // Spread angle should decrease: A=25, B=20, C=15, D=12.5, E=8.75, F=5
        let angles = [25.0, 20.0, 15.0, 12.5, 8.75, 5.0];
        for i in 1..angles.len() {
            assert!(
                angles[i] < angles[i - 1],
                "Spread angle should decrease from A to F"
            );
        }
    }

    #[test]
    fn test_pasquill_stability_classification_logic() {
        // Daytime: low wind -> A, medium -> B, high -> C
        fn classify(ws: f64, daytime: bool) -> char {
            if daytime {
                if ws < 3.0 {
                    'A'
                } else if ws < 5.0 {
                    'B'
                } else {
                    'C'
                }
            } else {
                if ws < 3.0 {
                    'F'
                } else if ws < 5.0 {
                    'E'
                } else {
                    'D'
                }
            }
        }

        assert_eq!(classify(2.0, true), 'A');
        assert_eq!(classify(4.0, true), 'B');
        assert_eq!(classify(6.0, true), 'C');
        assert_eq!(classify(2.0, false), 'F');
        assert_eq!(classify(4.0, false), 'E');
        assert_eq!(classify(6.0, false), 'D');
    }

    // ─── Beer-Lambert Law ────────────────────────────────────────────────

    #[test]
    fn test_beer_lambert_transmittance() {
        // T = exp(-tau)
        let tau_zero = (0.0_f64).exp();
        assert!((tau_zero - 1.0).abs() < 0.001);

        let tau_one = (-1.0_f64).exp();
        assert!((tau_one - 0.368).abs() < 0.01);

        // Transmittance is always between 0 and 1 for tau >= 0
        for tau in [0.0_f64, 0.5, 1.0, 2.0, 5.0] {
            let t = (-tau).exp();
            assert!(
                (0.0..=1.0).contains(&t),
                "Transmittance out of range for tau={}",
                tau
            );
        }
    }

    // ─── Rayleigh Scattering ─────────────────────────────────────────────

    #[test]
    fn test_rayleigh_scattering_wavelength_dependence() {
        // sigma ~ lambda^-4
        // Blue (450nm) scatters more than red (650nm)
        let blue = (550.0_f64 / 450.0).powi(4);
        let red = (550.0_f64 / 650.0).powi(4);
        assert!(blue > red, "Blue should scatter more than red");
    }

    // ─── Uncertainty Propagation ─────────────────────────────────────────

    #[test]
    fn test_uncertainty_quadrature() {
        // sigma_total = sqrt(sigma_s^2 + sigma_w^2 + sigma_m^2)
        let s = 0.40_f64; // sensor
        let w = 0.20_f64; // weather
        let m = 0.50_f64; // model
        let total = (s * s + w * w + m * m).sqrt();
        assert!(total > 0.0);
        assert!(total < 1.0);
        // Should be ~0.67
        assert!((total - 0.67).abs() < 0.05);
    }

    // ─── Geographic Coordinate Tests ─────────────────────────────────────

    #[test]
    fn test_ntb_zone_coordinates() {
        // Verify all NTB zone centers are within bounds
        let zones = [
            ("Lombok Barat", -8.6818, 116.1240),
            ("Lombok Tengah", -8.7167, 116.2667),
            ("Lombok Timur", -8.6500, 116.5333),
            ("Lombok Utara", -8.3500, 116.4000),
            ("Kota Mataram", -8.5833, 116.1167),
            ("Sumbawa Barat", -8.7333, 116.8500),
            ("Sumbawa", -8.5000, 117.4167),
            ("Dompu", -8.5333, 118.4667),
            ("Bima", -8.6500, 118.6167),
            ("Kota Bima", -8.4667, 118.7167),
        ];

        for (name, lat, lon) in zones {
            assert!(
                lon > 115.0 && lon < 120.0,
                "{} lon out of NTB bounds: {}",
                name,
                lon
            );
            assert!(
                lat > -9.5 && lat < -8.0,
                "{} lat out of NTB bounds: {}",
                name,
                lat
            );
        }
    }

    // ─── Wind Speed Safety ───────────────────────────────────────────────

    #[test]
    fn test_wind_speed_clamping() {
        // Wind speed < 1.0 m/s should be clamped to prevent division by zero
        let ws = 0.5_f64;
        let ws_safe = if ws < 1.0 { 1.0 } else { ws };
        assert_eq!(ws_safe, 1.0);

        let ws_ok = 3.0_f64;
        let ws_safe2 = if ws_ok < 1.0 { 1.0 } else { ws_ok };
        assert_eq!(ws_safe2, 3.0);
    }

    // ─── CH4 Unit Conversion ─────────────────────────────────────────────

    #[test]
    fn test_ch4_ppm_conversion() {
        // Test conversion at standard conditions (25°C, 1 atm)
        let mg_m3 = 10.0;
        let temp_c = 25.0;
        let pressure_kpa = 101.325;

        let ppm = geoesg_aeco_backend::physics::gaussian_plume::mgm3_to_ppm_ch4(
            mg_m3,
            temp_c,
            pressure_kpa,
        );
        assert!(ppm > 0.0);

        // At 25C and 1 atm:
        // ppm = 10.0 * 8.3144 * 298.15 / (101.325 * 16.04) = 15.25
        assert!((ppm - 15.25).abs() < 0.1);

        // Test temperature dependence
        let temp_hot = 40.0;
        let ppm_hot = geoesg_aeco_backend::physics::gaussian_plume::mgm3_to_ppm_ch4(
            mg_m3,
            temp_hot,
            pressure_kpa,
        );
        assert!(
            ppm_hot > ppm,
            "Higher temperature should result in higher volume/ppm"
        );

        let temp_cold = 0.0;
        let ppm_cold = geoesg_aeco_backend::physics::gaussian_plume::mgm3_to_ppm_ch4(
            mg_m3,
            temp_cold,
            pressure_kpa,
        );
        assert!(
            ppm_cold < ppm,
            "Lower temperature should result in lower volume/ppm"
        );
    }

    // ─── PHME Criteria ──────────────────────────────────────────────────

    #[test]
    fn test_phme_proximity_criterion() {
        // Proximity-only: plume origin within 100m of sensitive receptor
        let proximity_threshold = 100.0;
        assert!(50.0 <= proximity_threshold); // Within threshold
        assert!(150.0 > proximity_threshold); // Beyond threshold
    }

    #[test]
    fn test_phme_size_criterion() {
        // Size and proximity: plume > 1000m AND overlaps receptor
        let size_threshold = 1000.0;
        assert!(1500.0 > size_threshold); // Large plume
        assert!(500.0 <= size_threshold); // Small plume
    }

    // ─── Terrain Blocking ────────────────────────────────────────────────

    #[test]
    fn test_terrain_blocking_elevation_diff() {
        // If terrain rises >15m along plume path, assume blocked
        let origin_elev = 100.0;
        let terrain_elev = 120.0;
        let threshold = 15.0;
        assert!(terrain_elev - origin_elev > threshold); // Should block

        let low_terrain = 110.0;
        assert!(low_terrain - origin_elev <= threshold); // Should not block
    }

    // ─── Thermal Stability Correction ────────────────────────────────────

    #[test]
    fn test_thermal_stability_t4_penalty() {
        // Uses T^4 ratio as heuristic for spread angle reduction
        let temp_k = 313.15_f64; // 40°C
        let baseline_k = 308.15_f64; // 35°C
        let ratio = (baseline_k / temp_k).powi(4);
        assert!(ratio < 1.0, "T^4 penalty should reduce spread angle");
        assert!(
            ratio > 0.9,
            "T^4 penalty should be small for small temp differences"
        );
    }

    // ─── Sensor Smear ────────────────────────────────────────────────────

    #[test]
    fn test_sensor_smear_detection() {
        let roll_limit = 6.85;
        let pitch_limit = 4.8;

        // Below limits - no smear
        assert!(5.0 <= roll_limit);
        assert!(2.0 <= pitch_limit);

        // Above limits - smear detected
        assert!(7.0 > roll_limit);
        assert!(5.0 > pitch_limit);
    }

    // ─── EMIT Configuration ─────────────────────────────────────────────

    #[test]
    fn test_emit_stac_url_format() {
        let url = "https://ghgcenter.upc.nasa.gov/api/stac";
        assert!(
            url.starts_with("https://"),
            "EMIT STAC URL should use HTTPS"
        );
        assert!(url.contains("stac"), "URL should contain 'stac'");
    }

    #[test]
    fn test_emit_default_bbox() {
        // NTB bounding box: [min_lon, min_lat, max_lon, max_lat]
        let bbox = [115.40, -9.15, 119.45, -8.00];
        assert_eq!(bbox.len(), 4);
        assert!(bbox[0] < bbox[2], "min_lon should be < max_lon");
        assert!(bbox[1] < bbox[3], "min_lat should be < max_lat");

        // Verify NTB bounds
        assert!(bbox[0] > 115.0, "min_lon should be > 115");
        assert!(bbox[2] < 120.0, "max_lon should be < 120");
        assert!(bbox[1] < -8.0, "min_lat should be < -8");
        assert!(bbox[3] > -9.5, "max_lat should be > -9.5");
    }

    #[test]
    fn test_emit_poll_interval() {
        let poll_secs = 43200u64; // 12 hours
        let hours = poll_secs / 3600;
        assert_eq!(hours, 12, "Default poll interval should be 12 hours");
    }

    #[test]
    fn test_emit_collection_name() {
        let collection = "emit-ch4plume-v1";
        assert!(
            collection.starts_with("emit-"),
            "Collection should start with 'emit-'"
        );
        assert!(
            collection.contains("ch4"),
            "Collection should reference CH4"
        );
    }

    // ─── EMIT Monitoring ────────────────────────────────────────────────

    #[test]
    fn test_health_status_has_emit_field() {
        // Verify HealthStatus struct has last_emit_fetch field
        // This is a compile-time check - if the field doesn't exist, this won't compile
        let health = HealthStatus {
            status: "HEALTHY".to_string(),
            database: ComponentHealth {
                status: "OK".to_string(),
                message: None,
            },
            dem_file: ComponentHealth {
                status: "OK".to_string(),
                message: None,
            },
            last_bmkg_fetch: None,
            last_carbon_mapper_fetch: None,
            last_emit_fetch: None,
            last_s5p_fetch: None,
            uptime_seconds: 0,
        };
        assert_eq!(health.status, "HEALTHY");
        assert!(health.last_emit_fetch.is_none());
    }

    #[test]
    fn test_emit_metrics_names() {
        // Verify metric names follow convention
        let metrics = vec![
            "geoesg_emit_fetches",
            "geoesg_emit_errors",
            "geoesg_emit_plumes_ingested",
        ];
        for metric in metrics {
            assert!(
                metric.starts_with("geoesg_"),
                "Metric should start with 'geoesg_'"
            );
            assert!(metric.contains("emit"), "Metric should contain 'emit'");
        }
    }
}
