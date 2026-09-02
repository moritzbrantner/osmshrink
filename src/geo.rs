use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::error::Result;
use crate::geometry::Geometry;
use crate::model::Feature;

pub type GeoProperties = Map<String, Value>;
pub type GeoMetadata = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GeoFeatureId {
    String(String),
    Number(Number),
}

impl From<geojson::feature::Id> for GeoFeatureId {
    fn from(value: geojson::feature::Id) -> Self {
        match value {
            geojson::feature::Id::String(value) => Self::String(value),
            geojson::feature::Id::Number(value) => Self::Number(value),
        }
    }
}

impl From<GeoFeatureId> for geojson::feature::Id {
    fn from(value: GeoFeatureId) -> Self {
        match value {
            GeoFeatureId::String(value) => Self::String(value),
            GeoFeatureId::Number(value) => Self::Number(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoFeature {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<GeoFeatureId>,
    #[serde(default)]
    pub properties: GeoProperties,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<geojson::Geometry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<geojson::Bbox>,
    #[serde(default)]
    pub metadata: GeoMetadata,
}

impl GeoFeature {
    pub fn from_osm(feature: &Feature) -> Self {
        let mut properties = GeoProperties::new();
        properties.insert("osm_id".to_owned(), Value::from(feature.id));
        properties.insert(
            "osm_type".to_owned(),
            Value::from(feature.kind.as_str()),
        );
        for (key, value) in &feature.tags {
            properties.insert(key.clone(), Value::from(value.clone()));
        }

        Self {
            id: Some(GeoFeatureId::String(format!(
                "{}/{}",
                feature.kind.as_str(),
                feature.id
            ))),
            properties,
            geometry: Some(osm_geometry_to_geojson(&feature.geometry)),
            bbox: None,
            metadata: GeoMetadata::new(),
        }
    }
}

impl From<&Feature> for GeoFeature {
    fn from(feature: &Feature) -> Self {
        Self::from_osm(feature)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoDataset {
    #[serde(default)]
    pub features: Vec<GeoFeature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<geojson::Bbox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crs: Option<String>,
    #[serde(default)]
    pub metadata: GeoMetadata,
}

pub trait GeoFeatureReader {
    fn next_feature(&mut self) -> Result<Option<GeoFeature>>;
}

pub trait GeoFeatureWriter {
    fn write_feature(&mut self, feature: &GeoFeature) -> Result<()>;

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

pub fn pipe_features<R, W>(reader: &mut R, writer: &mut W) -> Result<usize>
where
    R: GeoFeatureReader,
    W: GeoFeatureWriter,
{
    let mut count = 0;
    while let Some(feature) = reader.next_feature()? {
        writer.write_feature(&feature)?;
        count += 1;
    }
    writer.finish()?;
    Ok(count)
}

fn osm_geometry_to_geojson(geometry: &Geometry) -> geojson::Geometry {
    match geometry {
        Geometry::Point { coordinates } => geojson::Geometry::new_point(position(*coordinates)),
        Geometry::LineString { coordinates } => {
            geojson::Geometry::new_line_string(coordinates.iter().copied().map(position))
        }
        Geometry::Polygon { coordinates } => geojson::Geometry::new_polygon(
            coordinates
                .iter()
                .map(|ring| ring.iter().copied().map(position).collect::<Vec<_>>()),
        ),
        Geometry::MultiPolygon { coordinates } => {
            geojson::Geometry::new_multi_polygon(coordinates.iter().map(|polygon| {
                polygon
                    .iter()
                    .map(|ring| ring.iter().copied().map(position).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            }))
        }
    }
}

fn position(coordinate: [f64; 2]) -> geojson::Position {
    vec![coordinate[0], coordinate[1]].into()
}

#[cfg(test)]
mod tests {
    use crate::geometry::Geometry;
    use crate::model::{ElementKind, Tags};

    use super::*;

    #[test]
    fn converts_osm_feature_to_neutral_feature() {
        let feature = Feature {
            id: 42,
            kind: ElementKind::Way,
            tags: Tags::from([
                ("highway".to_owned(), "residential".to_owned()),
                ("lanes".to_owned(), "2".to_owned()),
            ]),
            geometry: Geometry::LineString {
                coordinates: vec![[8.7, 48.9], [8.8, 49.0]],
            },
        };

        let geo = GeoFeature::from_osm(&feature);
        assert_eq!(
            geo.id,
            Some(GeoFeatureId::String("way/42".to_owned()))
        );
        assert_eq!(geo.properties["osm_id"], 42);
        assert_eq!(geo.properties["osm_type"], "way");
        assert_eq!(geo.properties["highway"], "residential");
        assert!(matches!(
            geo.geometry.as_ref().map(|geometry| &geometry.value),
            Some(geojson::GeometryValue::LineString { .. })
        ));
    }

    struct VecReader {
        features: std::vec::IntoIter<GeoFeature>,
    }

    impl GeoFeatureReader for VecReader {
        fn next_feature(&mut self) -> Result<Option<GeoFeature>> {
            Ok(self.features.next())
        }
    }

    #[derive(Default)]
    struct VecWriter {
        features: Vec<GeoFeature>,
        finished: bool,
    }

    impl GeoFeatureWriter for VecWriter {
        fn write_feature(&mut self, feature: &GeoFeature) -> Result<()> {
            self.features.push(feature.clone());
            Ok(())
        }

        fn finish(&mut self) -> Result<()> {
            self.finished = true;
            Ok(())
        }
    }

    #[test]
    fn pipes_features_through_adapter_seam() {
        let feature = GeoFeature {
            id: Some(GeoFeatureId::String("a".to_owned())),
            properties: GeoProperties::new(),
            geometry: None,
            bbox: None,
            metadata: GeoMetadata::new(),
        };
        let mut reader = VecReader {
            features: vec![feature.clone()].into_iter(),
        };
        let mut writer = VecWriter::default();

        let count = pipe_features(&mut reader, &mut writer).unwrap();
        assert_eq!(count, 1);
        assert_eq!(writer.features, vec![feature]);
        assert!(writer.finished);
    }
}
