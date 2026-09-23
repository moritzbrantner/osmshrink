use std::path::Path;

use crate::error::{OsmshrinkError, Result};
use crate::geo::{GeoFeature, GeoFeatureId};
use crate::model::Feature;

pub(crate) fn parse_ndjson_feature(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<GeoFeature> {
    if let Ok(feature) = serde_json::from_str::<geojson::Feature>(line) {
        return Ok(feature_from_geojson(feature));
    }
    if let Ok(feature) = serde_json::from_str::<Feature>(line) {
        return Ok(GeoFeature::from_osm(&feature));
    }
    if let Ok(feature) = serde_json::from_str::<GeoFeature>(line) {
        return Ok(feature);
    }

    Err(OsmshrinkError::ParseGeoData {
        path: path.to_path_buf(),
        format: "NDJSON",
        details: format!("line {line_number} is not a supported feature record"),
    })
}

pub(crate) fn feature_from_geojson(feature: geojson::Feature) -> GeoFeature {
    GeoFeature {
        id: feature.id.map(GeoFeatureId::from),
        properties: feature.properties.unwrap_or_default(),
        geometry: feature.geometry,
        bbox: feature.bbox,
        metadata: feature.foreign_members.unwrap_or_default(),
    }
}
