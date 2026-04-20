use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::{OsmshrinkError, Result};
use crate::geometry::Geometry;
use crate::model::Feature;
use crate::spec::{OutputField, OutputFormat};

pub struct OutputWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    format: OutputFormat,
    first_json_item: bool,
    fields: Vec<OutputField>,
}

impl OutputWriter {
    pub fn create(path: &Path, format: OutputFormat) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| OsmshrinkError::WriteFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let file = File::create(path).map_err(|source| OsmshrinkError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;
        let mut writer = BufWriter::new(file);
        match format {
            OutputFormat::Json => {
                writer
                    .write_all(b"[\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: path.to_path_buf(),
                        source,
                    })?;
            }
            OutputFormat::Geojson => {
                writer
                    .write_all(b"{\"type\":\"FeatureCollection\",\"features\":[\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: path.to_path_buf(),
                        source,
                    })?;
            }
            OutputFormat::Ndjson => {}
        }

        Ok(Self {
            path: path.to_path_buf(),
            writer,
            format,
            first_json_item: true,
            fields: OutputField::defaults(),
        })
    }

    pub fn set_fields(&mut self, fields: Vec<OutputField>) {
        self.fields = fields;
    }

    pub fn write_feature(&mut self, feature: &Feature) -> Result<()> {
        let value = feature.to_value_with_fields(&self.fields);

        match self.format {
            OutputFormat::Ndjson => {
                serde_json::to_writer(&mut self.writer, &value).map_err(|source| {
                    OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source: std::io::Error::other(source),
                    }
                })?;
                self.writer
                    .write_all(b"\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source,
                    })?;
            }
            OutputFormat::Json => {
                if !self.first_json_item {
                    self.writer
                        .write_all(b",\n")
                        .map_err(|source| OsmshrinkError::WriteFile {
                            path: self.path.clone(),
                            source,
                        })?;
                }
                serde_json::to_writer_pretty(&mut self.writer, &value).map_err(|source| {
                    OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source: std::io::Error::other(source),
                    }
                })?;
                self.first_json_item = false;
            }
            OutputFormat::Geojson => {
                if !self.first_json_item {
                    self.writer
                        .write_all(b",\n")
                        .map_err(|source| OsmshrinkError::WriteFile {
                            path: self.path.clone(),
                            source,
                        })?;
                }
                let geojson_feature = to_geojson_feature(feature, &self.fields);
                serde_json::to_writer(&mut self.writer, &geojson_feature).map_err(|source| {
                    OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source: std::io::Error::other(source),
                    }
                })?;
                self.first_json_item = false;
            }
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        match self.format {
            OutputFormat::Json => {
                self.writer
                    .write_all(b"\n]\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source,
                    })?;
            }
            OutputFormat::Geojson => {
                self.writer
                    .write_all(b"\n]}\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source,
                    })?;
            }
            OutputFormat::Ndjson => {}
        }
        self.writer
            .flush()
            .map_err(|source| OsmshrinkError::WriteFile {
                path: self.path,
                source,
            })
    }
}

fn to_geojson_feature(feature: &Feature, fields: &[OutputField]) -> geojson::Feature {
    let mut properties = serde_json::Map::new();
    let mut id = None;

    if fields.contains(&OutputField::Id) {
        id = Some(geojson::feature::Id::String(format!(
            "{}/{}",
            feature.kind.as_str(),
            feature.id
        )));
        properties.insert("osm_id".to_owned(), serde_json::Value::from(feature.id));
    }
    if fields.contains(&OutputField::Type) {
        properties.insert(
            "osm_type".to_owned(),
            serde_json::Value::from(feature.kind.as_str()),
        );
    }
    if fields.contains(&OutputField::Tags) {
        for (key, value) in &feature.tags {
            properties.insert(key.clone(), serde_json::Value::from(value.clone()));
        }
    }

    geojson::Feature {
        bbox: None,
        geometry: Some(to_geojson_geometry(&feature.geometry)),
        id,
        properties: Some(properties),
        foreign_members: None,
    }
}

fn to_geojson_geometry(geometry: &Geometry) -> geojson::Geometry {
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
    use tempfile::NamedTempFile;

    use crate::geometry::Geometry;
    use crate::model::{ElementKind, Tags};

    use super::*;

    #[test]
    fn writes_geojson_feature_collection() {
        let file = NamedTempFile::with_suffix(".geojson").unwrap();
        let mut writer = OutputWriter::create(file.path(), OutputFormat::Geojson).unwrap();
        writer.set_fields(vec![
            OutputField::Id,
            OutputField::Type,
            OutputField::Tags,
            OutputField::Geometry,
        ]);
        writer
            .write_feature(&Feature {
                id: 123,
                kind: ElementKind::Node,
                tags: Tags::from([("name".to_owned(), "A".to_owned())]),
                geometry: Geometry::Point {
                    coordinates: [8.7, 48.9],
                },
            })
            .unwrap();
        writer.finish().unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(value["type"], "FeatureCollection");
        assert_eq!(value["features"][0]["id"], "node/123");
        assert_eq!(value["features"][0]["properties"]["osm_id"], 123);
        assert_eq!(value["features"][0]["properties"]["osm_type"], "node");
        assert_eq!(value["features"][0]["properties"]["name"], "A");
    }
}
