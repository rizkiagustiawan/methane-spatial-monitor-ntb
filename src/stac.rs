use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// STAC Specification Version
pub const STAC_VERSION: &str = "1.0.0";

/// STAC Item - The core unit of STAC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacItem {
    #[serde(default = "default_type")]
    pub r#type: String,
    pub id: String,
    pub geometry: Value,
    pub bbox: Vec<f64>,
    pub properties: StacProperties,
    pub assets: std::collections::HashMap<String, StacAsset>,
    #[serde(default)]
    pub links: Vec<StacLink>,
    #[serde(default)]
    pub collection: Option<String>,
}

fn default_type() -> String {
    "Feature".to_string()
}

/// STAC Item Properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacProperties {
    pub datetime: DateTime<Utc>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

/// STAC Asset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacAsset {
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

/// STAC Link
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacLink {
    pub rel: String,
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// STAC Collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacCollection {
    #[serde(default = "default_collection_type")]
    pub r#type: String,
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub license: String,
    pub extent: StacExtent,
    #[serde(default)]
    pub links: Vec<StacLink>,
    #[serde(default)]
    pub assets: std::collections::HashMap<String, StacAsset>,
}

fn default_collection_type() -> String {
    "Collection".to_string()
}

/// STAC Extent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacExtent {
    pub spatial: StacSpatialExtent,
    pub temporal: StacTemporalExtent,
}

/// STAC Spatial Extent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacSpatialExtent {
    pub bbox: Vec<Vec<f64>>,
}

/// STAC Temporal Extent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacTemporalExtent {
    pub interval: Vec<Vec<Option<DateTime<Utc>>>>,
}

/// STAC Search Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacSearchRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intersects: Option<Value>,
}

/// STAC Search Response (ItemCollection)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacSearchResponse {
    #[serde(default = "default_type")]
    pub r#type: String,
    pub features: Vec<StacItem>,
    #[serde(default)]
    pub links: Vec<StacLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<StacSearchContext>,
}

/// STAC Search Context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacSearchContext {
    pub returned: u32,
    pub matched: Option<u32>,
}

/// Methane-specific STAC extensions
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethaneStacProperties {
    pub datetime: DateTime<Utc>,
    pub emission_rate_kg_hr: f64,
    pub emission_rate_uncertainty_kg_hr: Option<f64>,
    pub source_type: Option<String>,  // oil_gas, waste, coal, agriculture
    pub instrument: Option<String>,   // tanager-1, emit, tropomi
    pub processing_level: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

impl StacItem {
    /// Create a new STAC Item for a methane observation
    pub fn from_methane_observation(
        id: Uuid,
        recorded_at: DateTime<Utc>,
        emission_rate_kg_hr: f64,
        geometry: Value,
        bbox: Vec<f64>,
        source: &str,
    ) -> Self {
        let mut properties = std::collections::HashMap::new();
        properties.insert("emission_rate_kg_hr".to_string(), 
            serde_json::json!(emission_rate_kg_hr));
        properties.insert("source".to_string(), 
            serde_json::json!(source));
        properties.insert("instrument".to_string(), 
            serde_json::json!("tanager-1"));
        properties.insert("processing_level".to_string(), 
            serde_json::json!("L2"));

        StacItem {
            r#type: "Feature".to_string(),
            id: id.to_string(),
            geometry,
            bbox,
            properties: StacProperties {
                datetime: recorded_at,
                extra: properties,
            },
            assets: std::collections::HashMap::new(),
            links: vec![
                StacLink {
                    rel: "self".to_string(),
                    href: format!("/api/stac/items/{}", id),
                    r#type: Some("application/geo+json".to_string()),
                    title: None,
                },
                StacLink {
                    rel: "collection".to_string(),
                    href: "/api/stac/collections/methane-observations".to_string(),
                    r#type: Some("application/json".to_string()),
                    title: Some("Methane Observations".to_string()),
                },
            ],
            collection: Some("methane-observations".to_string()),
        }
    }
}

impl StacCollection {
    /// Create the methane observations collection
    pub fn methane_observations() -> Self {
        StacCollection {
            r#type: "Collection".to_string(),
            id: "methane-observations".to_string(),
            title: Some("NTB Methane Observations".to_string()),
            description: Some("Methane emission observations from Carbon Mapper Tanager-1 satellite for West Nusa Tenggara region".to_string()),
            license: "proprietary".to_string(),
            extent: StacExtent {
                spatial: StacSpatialExtent {
                    bbox: vec![vec![115.40, -9.15, 119.45, -8.00]],
                },
                temporal: StacTemporalExtent {
                    interval: vec![vec![
                        Some(DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap().with_timezone(&Utc)),
                        None,
                    ]],
                },
            },
            links: vec![
                StacLink {
                    rel: "self".to_string(),
                    href: "/api/stac/collections/methane-observations".to_string(),
                    r#type: Some("application/json".to_string()),
                    title: None,
                },
                StacLink {
                    rel: "root".to_string(),
                    href: "/api/stac".to_string(),
                    r#type: Some("application/json".to_string()),
                    title: None,
                },
                StacLink {
                    rel: "items".to_string(),
                    href: "/api/stac/collections/methane-observations/items".to_string(),
                    r#type: Some("application/geo+json".to_string()),
                    title: None,
                },
            ],
            assets: std::collections::HashMap::new(),
        }
    }
}

impl StacSearchResponse {
    /// Create an empty search response
    #[allow(dead_code)]
    pub fn empty() -> Self {
        StacSearchResponse {
            r#type: "FeatureCollection".to_string(),
            features: vec![],
            links: vec![],
            context: Some(StacSearchContext {
                returned: 0,
                matched: Some(0),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_stac_item_serialization() {
        let item = StacItem::from_methane_observation(
            Uuid::new_v4(),
            Utc::now(),
            150.0,
            json!({"type": "Point", "coordinates": [116.12, -8.68]}),
            vec![116.11, -8.69, 116.13, -8.67],
            "carbon_mapper",
        );

        let serialized = serde_json::to_string(&item).unwrap();
        assert!(serialized.contains("\"type\":\"Feature\""));
        assert!(serialized.contains("emission_rate_kg_hr"));
    }

    #[test]
    fn test_stac_item_from_methane_observation() {
        let id = Uuid::new_v4();
        let recorded_at = Utc::now();
        let emission_rate = 200.0;
        let geometry = json!({"type": "Point", "coordinates": [116.12, -8.68]});
        let bbox = vec![116.11, -8.69, 116.13, -8.67];

        let item = StacItem::from_methane_observation(
            id,
            recorded_at,
            emission_rate,
            geometry.clone(),
            bbox.clone(),
            "carbon_mapper",
        );

        assert_eq!(item.id, id.to_string());
        assert_eq!(item.geometry, geometry);
        assert_eq!(item.bbox, bbox);
        assert!(item.properties.extra.contains_key("emission_rate_kg_hr"));
        assert!(item.properties.extra.contains_key("source"));
    }

    #[test]
    fn test_stac_collection_methane_observations() {
        let collection = StacCollection::methane_observations();

        assert_eq!(collection.id, "methane-observations");
        assert_eq!(collection.r#type, "Collection");
        assert!(collection.title.is_some());
        assert!(collection.description.is_some());
        assert_eq!(collection.license, "proprietary");
        
        // Check spatial extent covers NTB
        let bbox = &collection.extent.spatial.bbox[0];
        assert!(bbox[0] < 116.0);  // West
        assert!(bbox[2] > 119.0);  // East
        assert!(bbox[1] < -9.0);   // South
        assert!(bbox[3] >= -8.0);  // North
    }

    #[test]
    fn test_stac_search_response_empty() {
        let response = StacSearchResponse::empty();

        assert_eq!(response.r#type, "FeatureCollection");
        assert!(response.features.is_empty());
        assert!(response.context.is_some());
        assert_eq!(response.context.unwrap().returned, 0);
    }

    #[test]
    fn test_stac_link_structure() {
        let link = StacLink {
            rel: "self".to_string(),
            href: "/api/stac/items/123".to_string(),
            r#type: Some("application/geo+json".to_string()),
            title: None,
        };

        assert_eq!(link.rel, "self");
        assert_eq!(link.href, "/api/stac/items/123");
        assert!(link.r#type.is_some());
        assert!(link.title.is_none());
    }

    #[test]
    fn test_stac_asset_structure() {
        let asset = StacAsset {
            href: "s3://bucket/data.tif".to_string(),
            title: Some("Methane Plume Data".to_string()),
            description: Some("Tanager-1 methane observation".to_string()),
            r#type: Some("image/tiff".to_string()),
            roles: Some(vec!["data".to_string()]),
        };

        assert_eq!(asset.href, "s3://bucket/data.tif");
        assert!(asset.title.is_some());
        assert!(asset.roles.is_some());
    }

    #[test]
    fn test_stac_version_constant() {
        assert_eq!(STAC_VERSION, "1.0.0");
    }
}
