use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::Serialize;
use serde_json::Value;

use crate::error::{OsmshrinkError, Result};
use crate::geo::{GeoDataset, GeoFeature, GeoFeatureId, GeoMetadata};
use crate::model::Feature;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum GeoFormat {
    Geojson,
    Json,
    Ndjson,
}

impl GeoFormat {
    pub fn from_path(path: &Path) -> Result<Self> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("geojson") => Ok(Self::Geojson),
            Some("json") => Ok(Self::Json),
            Some("ndjson") => Ok(Self::Ndjson),
            _ => Err(OsmshrinkError::UnsupportedGeoFile {
                path: path.to_path_buf(),
                expected: ".geojson, .json, or .ndjson",
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Geojson => "geojson",
            Self::Json => "json",
            Self::Ndjson => "ndjson",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Geojson => "GeoJSON",
            Self::Json => "JSON",
            Self::Ndjson => "NDJSON",
        }
    }
}

impl fmt::Display for GeoFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionLossKind {
    Metadata,
    Crs,
    Topology,
}

impl fmt::Display for ConversionLossKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Metadata => "metadata",
            Self::Crs => "crs",
            Self::Topology => "topology",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversionLoss {
    pub kind: ConversionLossKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversionReport {
    pub input: PathBuf,
    pub output: PathBuf,
    pub input_format: GeoFormat,
    pub output_format: GeoFormat,
    pub features_read: usize,
    pub features_written: usize,
    pub losses: Vec<ConversionLoss>,
}

impl ConversionReport {
    pub fn is_lossless(&self) -> bool {
        self.losses.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub input_format: Option<GeoFormat>,
    pub output_format: Option<GeoFormat>,
}

pub fn convert_path(options: ConvertOptions) -> Result<ConversionReport> {
    let input_format = options
        .input_format
        .unwrap_or(GeoFormat::from_path(&options.input)?);
    let output_format = options
        .output_format
        .unwrap_or(GeoFormat::from_path(&options.output)?);

    let dataset = read_dataset(&options.input, input_format)?;
    let features_read = dataset.features.len();
    let losses = conversion_losses(&dataset, output_format);
    write_dataset(&options.output, output_format, &dataset)?;

    Ok(ConversionReport {
        input: options.input,
        output: options.output,
        input_format,
        output_format,
        features_read,
        features_written: dataset.features.len(),
        losses,
    })
}

pub fn read_dataset(path: &Path, format: GeoFormat) -> Result<GeoDataset> {
    match format {
        GeoFormat::Geojson => {
            let contents = read_to_string(path)?;
            let document = contents
                .parse::<geojson::GeoJson>()
                .map_err(|source| parse_error(path, format, source.to_string()))?;
            Ok(dataset_from_geojson(document))
        }
        GeoFormat::Json => {
            let contents = read_to_string(path)?;
            parse_json_dataset(path, &contents)
        }
        GeoFormat::Ndjson => read_ndjson(path),
    }
}

pub fn write_dataset(path: &Path, format: GeoFormat, dataset: &GeoDataset) -> Result<()> {
    create_parent(path)?;
    let file = File::create(path).map_err(|source| OsmshrinkError::WriteFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut writer = BufWriter::new(file);

    match format {
        GeoFormat::Geojson => {
            let document = geojson::GeoJson::FeatureCollection(to_geojson_collection(dataset));
            serde_json::to_writer_pretty(&mut writer, &document).map_err(|source| {
                OsmshrinkError::WriteFile {
                    path: path.to_path_buf(),
                    source: std::io::Error::other(source),
                }
            })?;
            writer
                .write_all(b"\n")
                .map_err(|source| OsmshrinkError::WriteFile {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        GeoFormat::Json => {
            serde_json::to_writer_pretty(&mut writer, dataset).map_err(|source| {
                OsmshrinkError::WriteFile {
                    path: path.to_path_buf(),
                    source: std::io::Error::other(source),
                }
            })?;
            writer
                .write_all(b"\n")
                .map_err(|source| OsmshrinkError::WriteFile {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        GeoFormat::Ndjson => {
            for feature in &dataset.features {
                serde_json::to_writer(&mut writer, feature).map_err(|source| {
                    OsmshrinkError::WriteFile {
                        path: path.to_path_buf(),
                        source: std::io::Error::other(source),
                    }
                })?;
                writer
                    .write_all(b"\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: path.to_path_buf(),
                        source,
                    })?;
            }
        }
    }

    writer
        .flush()
        .map_err(|source| OsmshrinkError::WriteFile {
            path: path.to_path_buf(),
            source,
        })
}

fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| OsmshrinkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_json_dataset(path: &Path, contents: &str) -> Result<GeoDataset> {
    if let Ok(document) = contents.parse::<geojson::GeoJson>() {
        return Ok(dataset_from_geojson(document));
    }
    if let Ok(dataset) = serde_json::from_str::<GeoDataset>(contents) {
        return Ok(dataset);
    }
    if let Ok(features) = serde_json::from_str::<Vec<Feature>>(contents) {
        return Ok(GeoDataset {
            features: features.iter().map(GeoFeature::from_osm).collect(),
            ..GeoDataset::default()
        });
    }
    if let Ok(features) = serde_json::from_str::<Vec<GeoFeature>>(contents) {
        return Ok(GeoDataset {
            features,
            ..GeoDataset::default()
        });
    }
    if let Ok(feature) = serde_json::from_str::<Feature>(contents) {
        return Ok(GeoDataset {
            features: vec![GeoFeature::from_osm(&feature)],
            ..GeoDataset::default()
        });
    }
    if let Ok(feature) = serde_json::from_str::<GeoFeature>(contents) {
        return Ok(GeoDataset {
            features: vec![feature],
            ..GeoDataset::default()
        });
    }

    Err(parse_error(
        path,
        GeoFormat::Json,
        "expected GeoJSON, a GeoDataset object, generic feature JSON, or osmshrink normalized JSON",
    ))
}

fn read_ndjson(path: &Path) -> Result<GeoDataset> {
    let file = File::open(path).map_err(|source| OsmshrinkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = BufReader::new(file);
    let mut features = Vec::new();

    for (line_index, line) in reader.lines().enumerate() {
        let line = line.map_err(|source| OsmshrinkError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        features.push(parse_ndjson_feature(path, line_index + 1, line)?);
    }

    Ok(GeoDataset {
        features,
        ..GeoDataset::default()
    })
}

fn parse_ndjson_feature(path: &Path, line_number: usize, line: &str) -> Result<GeoFeature> {
    if let Ok(feature) = serde_json::from_str::<geojson::Feature>(line) {
        return Ok(feature_from_geojson(feature));
    }
    if let Ok(feature) = serde_json::from_str::<Feature>(line) {
        return Ok(GeoFeature::from_osm(&feature));
    }
    if let Ok(feature) = serde_json::from_str::<GeoFeature>(line) {
        return Ok(feature);
    }

    Err(parse_error(
        path,
        GeoFormat::Ndjson,
        format!("line {line_number} is not a supported feature record"),
    ))
}

fn dataset_from_geojson(document: geojson::GeoJson) -> GeoDataset {
    match document {
        geojson::GeoJson::FeatureCollection(collection) => {
            let mut metadata = collection.foreign_members.unwrap_or_default();
            let crs = take_legacy_crs(&mut metadata);
            GeoDataset {
                features: collection
                    .features
                    .into_iter()
                    .map(feature_from_geojson)
                    .collect(),
                bbox: collection.bbox,
                crs,
                metadata,
            }
        }
        geojson::GeoJson::Feature(feature) => GeoDataset {
            features: vec![feature_from_geojson(feature)],
            ..GeoDataset::default()
        },
        geojson::GeoJson::Geometry(geometry) => GeoDataset {
            features: vec![GeoFeature {
                id: None,
                properties: Default::default(),
                geometry: Some(geometry),
                bbox: None,
                metadata: Default::default(),
            }],
            ..GeoDataset::default()
        },
    }
}

fn feature_from_geojson(feature: geojson::Feature) -> GeoFeature {
    GeoFeature {
        id: feature.id.map(GeoFeatureId::from),
        properties: feature.properties.unwrap_or_default(),
        geometry: feature.geometry,
        bbox: feature.bbox,
        metadata: feature.foreign_members.unwrap_or_default(),
    }
}

fn to_geojson_collection(dataset: &GeoDataset) -> geojson::FeatureCollection {
    let metadata = sanitize_metadata(&dataset.metadata, &["type", "features", "bbox", "crs"]);
    geojson::FeatureCollection {
        bbox: dataset.bbox.clone(),
        features: dataset.features.iter().map(to_geojson_feature).collect(),
        foreign_members: (!metadata.is_empty()).then_some(metadata),
    }
}

fn to_geojson_feature(feature: &GeoFeature) -> geojson::Feature {
    let metadata = sanitize_metadata(
        &feature.metadata,
        &["type", "bbox", "geometry", "id", "properties"],
    );
    geojson::Feature {
        bbox: feature.bbox.clone(),
        geometry: feature.geometry.clone(),
        id: feature.id.clone().map(Into::into),
        properties: Some(feature.properties.clone()),
        foreign_members: (!metadata.is_empty()).then_some(metadata),
    }
}

fn take_legacy_crs(metadata: &mut GeoMetadata) -> Option<String> {
    let value = metadata.remove("crs")?;
    if let Some(crs) = crs_name(&value) {
        Some(crs)
    } else {
        metadata.insert("crs".to_owned(), value);
        None
    }
}

fn crs_name(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_owned());
    }
    value
        .get("properties")
        .and_then(|properties| properties.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn conversion_losses(dataset: &GeoDataset, output_format: GeoFormat) -> Vec<ConversionLoss> {
    let mut losses = Vec::new();

    match output_format {
        GeoFormat::Json => {}
        GeoFormat::Ndjson => {
            if dataset.bbox.is_some() || !dataset.metadata.is_empty() {
                losses.push(ConversionLoss {
                    kind: ConversionLossKind::Metadata,
                    detail: "NDJSON writes feature records only, so dataset-level bbox and metadata are omitted"
                        .to_owned(),
                });
            }
            if dataset.crs.is_some() {
                losses.push(ConversionLoss {
                    kind: ConversionLossKind::Crs,
                    detail: "NDJSON has no dataset envelope, so dataset CRS metadata is omitted"
                        .to_owned(),
                });
            }
        }
        GeoFormat::Geojson => {
            if dataset.crs.is_some() {
                losses.push(ConversionLoss {
                    kind: ConversionLossKind::Crs,
                    detail: "RFC 7946 GeoJSON uses WGS84/CRS84 and does not carry a custom CRS member"
                        .to_owned(),
                });
            }
            if has_reserved_metadata(dataset) {
                losses.push(ConversionLoss {
                    kind: ConversionLossKind::Metadata,
                    detail: "metadata keys that collide with reserved GeoJSON members are omitted"
                        .to_owned(),
                });
            }
        }
    }

    losses
}

fn has_reserved_metadata(dataset: &GeoDataset) -> bool {
    contains_any_key(&dataset.metadata, &["type", "features", "bbox", "crs"])
        || dataset.features.iter().any(|feature| {
            contains_any_key(
                &feature.metadata,
                &["type", "bbox", "geometry", "id", "properties"],
            )
        })
}

fn contains_any_key(metadata: &GeoMetadata, keys: &[&str]) -> bool {
    keys.iter().any(|key| metadata.contains_key(*key))
}

fn sanitize_metadata(metadata: &GeoMetadata, reserved: &[&str]) -> GeoMetadata {
    metadata
        .iter()
        .filter(|(key, _)| !reserved.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn create_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| OsmshrinkError::WriteFile {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn parse_error(path: &Path, format: GeoFormat, details: impl Into<String>) -> OsmshrinkError {
    OsmshrinkError::ParseGeoData {
        path: path.to_path_buf(),
        format: format.display_name(),
        details: details.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn converts_geojson_through_neutral_json_without_losing_feature_data() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input.geojson");
        let neutral = dir.path().join("neutral.json");
        let output = dir.path().join("output.geojson");
        fs::write(
            &input,
            r#"{
                "type":"FeatureCollection",
                "bbox":[8.0,48.0,9.0,49.0],
                "dataset_name":"example",
                "features":[{
                    "type":"Feature",
                    "id":"station-a",
                    "properties":{"name":"A","rank":3,"active":true},
                    "geometry":{"type":"MultiPoint","coordinates":[[8.7,48.9],[8.8,49.0]]},
                    "source":"fixture"
                }]
            }"#,
        )
        .unwrap();

        let first = convert_path(ConvertOptions {
            input: input.clone(),
            output: neutral.clone(),
            input_format: None,
            output_format: None,
        })
        .unwrap();
        assert!(first.is_lossless());

        let dataset: GeoDataset =
            serde_json::from_str(&fs::read_to_string(&neutral).unwrap()).unwrap();
        assert_eq!(dataset.bbox, Some(vec![8.0, 48.0, 9.0, 49.0]));
        assert_eq!(dataset.metadata["dataset_name"], "example");
        assert_eq!(
            dataset.features[0].id,
            Some(GeoFeatureId::String("station-a".to_owned()))
        );
        assert_eq!(dataset.features[0].properties["rank"], 3);
        assert_eq!(dataset.features[0].metadata["source"], "fixture");

        let second = convert_path(ConvertOptions {
            input: neutral,
            output: output.clone(),
            input_format: None,
            output_format: None,
        })
        .unwrap();
        assert!(second.is_lossless());

        let value: Value = serde_json::from_str(&fs::read_to_string(output).unwrap()).unwrap();
        assert_eq!(value["type"], "FeatureCollection");
        assert_eq!(value["dataset_name"], "example");
        assert_eq!(value["features"][0]["id"], "station-a");
        assert_eq!(value["features"][0]["properties"]["active"], true);
        assert_eq!(value["features"][0]["source"], "fixture");
        assert_eq!(value["features"][0]["geometry"]["type"], "MultiPoint");
    }

    #[test]
    fn ndjson_reports_dataset_envelope_loss() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input.json");
        let output = dir.path().join("output.ndjson");
        let dataset = GeoDataset {
            features: vec![GeoFeature {
                id: None,
                properties: Default::default(),
                geometry: Some(geojson::Geometry::new_point([8.7, 48.9])),
                bbox: None,
                metadata: Default::default(),
            }],
            bbox: Some(vec![8.0, 48.0, 9.0, 49.0]),
            crs: Some("EPSG:25832".to_owned()),
            metadata: GeoMetadata::from_iter([("source".to_owned(), json!("survey"))]),
        };
        fs::write(&input, serde_json::to_vec_pretty(&dataset).unwrap()).unwrap();

        let report = convert_path(ConvertOptions {
            input,
            output,
            input_format: None,
            output_format: None,
        })
        .unwrap();

        assert_eq!(report.features_written, 1);
        assert!(
            report
                .losses
                .iter()
                .any(|loss| loss.kind == ConversionLossKind::Metadata)
        );
        assert!(
            report
                .losses
                .iter()
                .any(|loss| loss.kind == ConversionLossKind::Crs)
        );
    }

    #[test]
    fn reads_legacy_osm_json_array() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("legacy.json");
        fs::write(
            &input,
            r#"[{
                "id":7,
                "type":"node",
                "tags":{"amenity":"bench"},
                "geometry":{"type":"Point","coordinates":[8.7,48.9]}
            }]"#,
        )
        .unwrap();

        let dataset = read_dataset(&input, GeoFormat::Json).unwrap();
        assert_eq!(dataset.features.len(), 1);
        assert_eq!(dataset.features[0].properties["amenity"], "bench");
        assert_eq!(
            dataset.features[0].id,
            Some(GeoFeatureId::String("node/7".to_owned()))
        );
    }
}
