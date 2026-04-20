use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::geometry::Geometry;
use crate::spec::OutputField;

pub type Tags = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ElementKind {
    Node,
    Way,
}

impl ElementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Way => "way",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Feature {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: ElementKind,
    pub tags: Tags,
    pub geometry: Geometry,
}

impl Feature {
    pub fn to_value_with_fields(&self, fields: &[OutputField]) -> Value {
        let mut object = Map::new();

        for field in fields {
            match field {
                OutputField::Id => {
                    object.insert("id".to_owned(), Value::from(self.id));
                }
                OutputField::Type => {
                    object.insert("type".to_owned(), Value::from(self.kind.as_str()));
                }
                OutputField::Tags => {
                    object.insert(
                        "tags".to_owned(),
                        serde_json::to_value(&self.tags).expect("tags serialize"),
                    );
                }
                OutputField::Geometry => {
                    object.insert(
                        "geometry".to_owned(),
                        serde_json::to_value(&self.geometry).expect("geometry serialize"),
                    );
                }
            }
        }

        Value::Object(object)
    }
}

#[cfg(test)]
mod tests {
    use crate::geometry::Geometry;

    use super::*;

    #[test]
    fn filters_output_fields() {
        let feature = Feature {
            id: 123,
            kind: ElementKind::Node,
            tags: Tags::from([("name".to_owned(), "Test".to_owned())]),
            geometry: Geometry::Point {
                coordinates: [8.7, 48.9],
            },
        };

        let value = feature.to_value_with_fields(&[OutputField::Id, OutputField::Geometry]);
        assert_eq!(value["id"], 123);
        assert!(value.get("geometry").is_some());
        assert!(value.get("type").is_none());
        assert!(value.get("tags").is_none());
    }
}
