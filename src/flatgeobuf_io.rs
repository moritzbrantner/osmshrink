use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use flatgeobuf::{
    ColumnType, FallibleStreamingIterator, FgbCrs, FgbReader, FgbWriter, FgbWriterOptions,
    GeometryType,
};
use geozero::geojson::GeoJson;
use geozero::{ColumnValue, PropertyProcessor, ToJson};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::convert::parse_ndjson_feature;
use crate::error::{OsmshrinkError, Result};
use crate::geo::{GeoDataset, GeoFeature, GeoFeatureId, GeoMetadata};

const FGB_METADATA_VERSION: u8 = 1;
const FGB_METADATA_KIND: &str = "osmshrink.flatgeobuf";
const FEATURE_STATE_COLUMN: &str = "__osmshrink_feature";

#[derive(Debug, Clone)]
pub struct FlatGeobufStreamReport {
    pub features: usize,
    pub dataset: GeoDataset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyKind {
    Bool,
    Long,
    ULong,
    Double,
    String,
    Json,
}

impl PropertyKind {
    fn column_type(self) -> ColumnType {
        match self {
            Self::Bool => ColumnType::Bool,
            Self::Long => ColumnType::Long,
            Self::ULong => ColumnType::ULong,
            Self::Double => ColumnType::Double,
            Self::String => ColumnType::String,
            Self::Json => ColumnType::Json,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self == other { self } else { Self::Json }
    }
}

#[derive(Debug, Clone)]
struct PropertySchema {
    columns: Vec<(String, PropertyKind)>,
    feature_state_column: String,
}

impl PropertySchema {
    fn infer<'a>(features: impl IntoIterator<Item = &'a GeoFeature>) -> Self {
        let mut columns = BTreeMap::<String, Option<PropertyKind>>::new();
        for feature in features {
            observe_feature(&mut columns, feature);
        }
        Self::from_columns(columns)
    }

    fn from_columns(columns: BTreeMap<String, Option<PropertyKind>>) -> Self {
        let feature_state_column = unique_column_name(&columns, FEATURE_STATE_COLUMN);
        let columns = columns
            .into_iter()
            .map(|(name, kind)| (name, kind.unwrap_or(PropertyKind::Json)))
            .collect();
        Self {
            columns,
            feature_state_column,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FeatureState {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<GeoFeatureId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox: Option<geojson::Bbox>,
    #[serde(default, skip_serializing_if = "GeoMetadata::is_empty")]
    metadata: GeoMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    null_properties: Vec<String>,
}

impl FeatureState {
    fn from_feature(feature: &GeoFeature) -> Self {
        Self {
            id: feature.id.clone(),
            bbox: feature.bbox.clone(),
            metadata: feature.metadata.clone(),
            null_properties: feature
                .properties
                .iter()
                .filter_map(|(name, value)| value.is_null().then_some(name.clone()))
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.bbox.is_none()
            && self.metadata.is_empty()
            && self.null_properties.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HeaderState {
    kind: String,
    version: u8,
    feature_state_column: String,
    #[serde(default)]
    metadata: GeoMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox: Option<geojson::Bbox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crs: Option<String>,
}

impl HeaderState {
    fn new(dataset: &GeoDataset, feature_state_column: String) -> Self {
        Self {
            kind: FGB_METADATA_KIND.to_owned(),
            version: FGB_METADATA_VERSION,
            feature_state_column,
            metadata: dataset.metadata.clone(),
            bbox: dataset.bbox.clone(),
            crs: dataset.crs.clone(),
        }
    }

    fn is_supported(&self) -> bool {
        self.kind == FGB_METADATA_KIND && self.version == FGB_METADATA_VERSION
    }
}

#[derive(Debug, Clone)]
struct HeaderInfo {
    dataset: GeoDataset,
    feature_state_column: Option<String>,
}

#[derive(Debug, Clone)]
enum OwnedColumnValue {
    Bool(bool),
    Long(i64),
    ULong(u64),
    Double(f64),
    String(String),
    Json(String),
}

pub fn read_flatgeobuf_dataset(path: &Path) -> Result<GeoDataset> {
    let file = File::open(path).map_err(|source| OsmshrinkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = FgbReader::open(BufReader::new(file))
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let header = header_info(reader.header());
    let state_column = header.feature_state_column.clone();
    let mut dataset = header.dataset;
    let mut features = reader
        .select_all()
        .map_err(|source| fgb_error(path, source.to_string()))?;

    while let Some(feature) = features
        .next()
        .map_err(|source| fgb_error(path, source.to_string()))?
    {
        dataset.features.push(read_feature(path, feature, state_column.as_deref())?);
    }

    Ok(dataset)
}

pub fn write_flatgeobuf_dataset(
    path: &Path,
    dataset: &GeoDataset,
    assume_wgs84: bool,
) -> Result<usize> {
    let schema = PropertySchema::infer(&dataset.features);
    let metadata = serde_json::to_string(&HeaderState::new(
        dataset,
        schema.feature_state_column.clone(),
    ))
    .map_err(|source| fgb_error(path, source.to_string()))?;
    let effective_crs = dataset
        .crs
        .as_deref()
        .or(assume_wgs84.then_some("EPSG:4326"));
    let mut writer = create_writer(path, &schema, effective_crs, &metadata)?;

    for feature in &dataset.features {
        add_feature(path, &mut writer, &schema, feature)?;
    }
    finish_writer(path, writer)?;
    Ok(dataset.features.len())
}

pub fn stream_ndjson_to_flatgeobuf(path: &Path, output: &Path) -> Result<usize> {
    let (schema, feature_count) = infer_ndjson_schema(path)?;
    let dataset = GeoDataset::default();
    let metadata = serde_json::to_string(&HeaderState::new(
        &dataset,
        schema.feature_state_column.clone(),
    ))
    .map_err(|source| fgb_error(output, source.to_string()))?;
    let mut writer = create_writer(output, &schema, None, &metadata)?;

    let file = File::open(path).map_err(|source| OsmshrinkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|source| OsmshrinkError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let feature = parse_ndjson_feature(path, line_index + 1, line)?;
        add_feature(output, &mut writer, &schema, &feature)?;
    }

    finish_writer(output, writer)?;
    Ok(feature_count)
}

pub fn stream_flatgeobuf_to_ndjson(path: &Path, output: &Path) -> Result<FlatGeobufStreamReport> {
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| OsmshrinkError::WriteFile {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let file = File::open(path).map_err(|source| OsmshrinkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = FgbReader::open(BufReader::new(file))
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let header = header_info(reader.header());
    let state_column = header.feature_state_column.clone();
    let mut features = reader
        .select_all_seq()
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let output_file = File::create(output).map_err(|source| OsmshrinkError::WriteFile {
        path: output.to_path_buf(),
        source,
    })?;
    let mut output_writer = BufWriter::new(output_file);
    let mut count = 0;

    while let Some(feature) = features
        .next()
        .map_err(|source| fgb_error(path, source.to_string()))?
    {
        let feature = read_feature(path, feature, state_column.as_deref())?;
        serde_json::to_writer(&mut output_writer, &feature).map_err(|source| {
            OsmshrinkError::WriteFile {
                path: output.to_path_buf(),
                source: std::io::Error::other(source),
            }
        })?;
        output_writer
            .write_all(b"\n")
            .map_err(|source| OsmshrinkError::WriteFile {
                path: output.to_path_buf(),
                source,
            })?;
        count += 1;
    }
    output_writer
        .flush()
        .map_err(|source| OsmshrinkError::WriteFile {
            path: output.to_path_buf(),
            source,
        })?;

    Ok(FlatGeobufStreamReport {
        features: count,
        dataset: header.dataset,
    })
}

fn infer_ndjson_schema(path: &Path) -> Result<(PropertySchema, usize)> {
    let file = File::open(path).map_err(|source| OsmshrinkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut columns = BTreeMap::<String, Option<PropertyKind>>::new();
    let mut count = 0;

    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|source| OsmshrinkError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let feature = parse_ndjson_feature(path, line_index + 1, line)?;
        observe_feature(&mut columns, &feature);
        count += 1;
    }

    Ok((PropertySchema::from_columns(columns), count))
}

fn observe_feature(columns: &mut BTreeMap<String, Option<PropertyKind>>, feature: &GeoFeature) {
    for (name, value) in &feature.properties {
        let observed = property_kind(value);
        let entry = columns.entry(name.clone()).or_insert(None);
        if let Some(observed) = observed {
            *entry = Some(match *entry {
                Some(existing) => existing.merge(observed),
                None => observed,
            });
        }
    }
}

fn property_kind(value: &Value) -> Option<PropertyKind> {
    match value {
        Value::Null => None,
        Value::Bool(_) => Some(PropertyKind::Bool),
        Value::Number(number) if number.as_i64().is_some() => Some(PropertyKind::Long),
        Value::Number(number) if number.as_u64().is_some() => Some(PropertyKind::ULong),
        Value::Number(_) => Some(PropertyKind::Double),
        Value::String(_) => Some(PropertyKind::String),
        Value::Array(_) | Value::Object(_) => Some(PropertyKind::Json),
    }
}

fn unique_column_name(columns: &BTreeMap<String, Option<PropertyKind>>, base: &str) -> String {
    if !columns.contains_key(base) {
        return base.to_owned();
    }
    for suffix in 1_u32.. {
        let candidate = format!("{base}_{suffix}");
        if !columns.contains_key(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 column suffix space exhausted")
}

fn create_writer<'a>(
    path: &Path,
    schema: &PropertySchema,
    crs: Option<&'a str>,
    metadata: &'a str,
) -> Result<FgbWriter<'a>> {
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("dataset");
    let options = FgbWriterOptions {
        write_index: true,
        detect_type: false,
        promote_to_multi: false,
        crs: fgb_crs(crs),
        metadata: Some(metadata),
        ..Default::default()
    };
    let mut writer = FgbWriter::create_with_options(name, GeometryType::Unknown, options)
        .map_err(|source| fgb_error(path, source.to_string()))?;

    for (column, kind) in &schema.columns {
        writer.add_column(column, kind.column_type(), |_fbb, _column| {});
    }
    writer.add_column(
        &schema.feature_state_column,
        ColumnType::Json,
        |_fbb, _column| {},
    );
    Ok(writer)
}

fn add_feature(
    path: &Path,
    writer: &mut FgbWriter<'_>,
    schema: &PropertySchema,
    feature: &GeoFeature,
) -> Result<()> {
    let geometry = feature.geometry.as_ref().ok_or_else(|| {
        fgb_error(
            path,
            "FlatGeobuf output requires geometry for every feature; null geometry is not supported by this adapter",
        )
    })?;
    let geometry_json = serde_json::to_string(geometry)
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let values = encode_properties(path, schema, feature)?;
    let state = FeatureState::from_feature(feature);
    let state_json = (!state.is_empty())
        .then(|| serde_json::to_string(&state))
        .transpose()
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let mut property_error = None::<String>;

    writer
        .add_feature_geom(GeoJson(&geometry_json), |output| {
            for (index, (name, _kind)) in schema.columns.iter().enumerate() {
                let Some(value) = values[index].as_ref() else {
                    continue;
                };
                let result = match value {
                    OwnedColumnValue::Bool(value) => {
                        output.property(index, name, &ColumnValue::Bool(*value))
                    }
                    OwnedColumnValue::Long(value) => {
                        output.property(index, name, &ColumnValue::Long(*value))
                    }
                    OwnedColumnValue::ULong(value) => {
                        output.property(index, name, &ColumnValue::ULong(*value))
                    }
                    OwnedColumnValue::Double(value) => {
                        output.property(index, name, &ColumnValue::Double(*value))
                    }
                    OwnedColumnValue::String(value) => {
                        output.property(index, name, &ColumnValue::String(value))
                    }
                    OwnedColumnValue::Json(value) => {
                        output.property(index, name, &ColumnValue::Json(value))
                    }
                };
                if let Err(error) = result {
                    property_error = Some(error.to_string());
                    return;
                }
            }

            if let Some(state_json) = state_json.as_deref() {
                let index = schema.columns.len();
                if let Err(error) = output.property(
                    index,
                    &schema.feature_state_column,
                    &ColumnValue::Json(state_json),
                ) {
                    property_error = Some(error.to_string());
                }
            }
        })
        .map_err(|source| fgb_error(path, source.to_string()))?;

    if let Some(error) = property_error {
        return Err(fgb_error(path, error));
    }
    Ok(())
}

fn encode_properties(
    path: &Path,
    schema: &PropertySchema,
    feature: &GeoFeature,
) -> Result<Vec<Option<OwnedColumnValue>>> {
    schema
        .columns
        .iter()
        .map(|(name, kind)| {
            let Some(value) = feature.properties.get(name) else {
                return Ok(None);
            };
            if value.is_null() {
                return Ok(None);
            }
            encode_value(path, *kind, value).map(Some)
        })
        .collect()
}

fn encode_value(path: &Path, kind: PropertyKind, value: &Value) -> Result<OwnedColumnValue> {
    let encoded = match kind {
        PropertyKind::Bool => OwnedColumnValue::Bool(value.as_bool().ok_or_else(|| {
            fgb_error(path, "property schema expected a boolean value")
        })?),
        PropertyKind::Long => OwnedColumnValue::Long(value.as_i64().ok_or_else(|| {
            fgb_error(path, "property schema expected a signed integer value")
        })?),
        PropertyKind::ULong => OwnedColumnValue::ULong(value.as_u64().ok_or_else(|| {
            fgb_error(path, "property schema expected an unsigned integer value")
        })?),
        PropertyKind::Double => OwnedColumnValue::Double(value.as_f64().ok_or_else(|| {
            fgb_error(path, "property schema expected a numeric value")
        })?),
        PropertyKind::String => OwnedColumnValue::String(
            value
                .as_str()
                .ok_or_else(|| fgb_error(path, "property schema expected a string value"))?
                .to_owned(),
        ),
        PropertyKind::Json => OwnedColumnValue::Json(
            serde_json::to_string(value).map_err(|source| fgb_error(path, source.to_string()))?,
        ),
    };
    Ok(encoded)
}

fn finish_writer(path: &Path, writer: FgbWriter<'_>) -> Result<()> {
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
    writer
        .write(BufWriter::new(file))
        .map_err(|source| fgb_error(path, source.to_string()))
}

fn read_feature(
    path: &Path,
    feature: &flatgeobuf::FgbFeature,
    state_column: Option<&str>,
) -> Result<GeoFeature> {
    let json = feature
        .to_json()
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let parsed = serde_json::from_str::<geojson::Feature>(&json)
        .map_err(|source| fgb_error(path, source.to_string()))?;
    let mut feature = crate::convert::feature_from_geojson(parsed);

    if let Some(state_column) = state_column
        && let Some(value) = feature.properties.remove(state_column)
    {
        let state = parse_feature_state(path, value)?;
        feature.id = state.id;
        feature.bbox = state.bbox;
        feature.metadata = state.metadata;
        for name in state.null_properties {
            feature.properties.insert(name, Value::Null);
        }
    }

    Ok(feature)
}

fn parse_feature_state(path: &Path, value: Value) -> Result<FeatureState> {
    match value {
        Value::String(value) => serde_json::from_str(&value),
        value => serde_json::from_value(value),
    }
    .map_err(|source| fgb_error(path, source.to_string()))
}

fn header_info(header: flatgeobuf::Header<'_>) -> HeaderInfo {
    let raw_metadata = header.metadata().map(str::to_owned);
    let header_state = raw_metadata
        .as_deref()
        .and_then(|metadata| serde_json::from_str::<HeaderState>(metadata).ok())
        .filter(HeaderState::is_supported);

    if let Some(state) = header_state {
        return HeaderInfo {
            dataset: GeoDataset {
                features: Vec::new(),
                bbox: state.bbox.or_else(|| header_bbox(header)),
                crs: state.crs.or_else(|| header_crs(header)),
                metadata: state.metadata,
            },
            feature_state_column: Some(state.feature_state_column),
        };
    }

    let mut metadata = GeoMetadata::new();
    if let Some(name) = header.name() {
        metadata.insert("flatgeobuf_name".to_owned(), Value::from(name));
    }
    if let Some(title) = header.title() {
        metadata.insert("flatgeobuf_title".to_owned(), Value::from(title));
    }
    if let Some(description) = header.description() {
        metadata.insert("flatgeobuf_description".to_owned(), Value::from(description));
    }
    if let Some(raw_metadata) = raw_metadata {
        let value = serde_json::from_str::<Value>(&raw_metadata).unwrap_or(Value::String(raw_metadata));
        metadata.insert("flatgeobuf_metadata".to_owned(), value);
    }

    HeaderInfo {
        dataset: GeoDataset {
            features: Vec::new(),
            bbox: header_bbox(header),
            crs: header_crs(header),
            metadata,
        },
        feature_state_column: None,
    }
}

fn header_bbox(header: flatgeobuf::Header<'_>) -> Option<geojson::Bbox> {
    header
        .envelope()
        .map(|envelope| envelope.iter().collect::<Vec<_>>())
}

fn header_crs(header: flatgeobuf::Header<'_>) -> Option<String> {
    let crs = header.crs()?;
    let org = crs.org().filter(|org| !org.is_empty());
    if crs.code() != 0 {
        return Some(format!("{}:{}", org.unwrap_or("EPSG"), crs.code()));
    }
    if let Some(code) = crs.code_string().filter(|code| !code.is_empty()) {
        return Some(match org {
            Some(org) => format!("{org}:{code}"),
            None => code.to_owned(),
        });
    }
    crs.wkt()
        .filter(|wkt| !wkt.is_empty())
        .or_else(|| crs.name().filter(|name| !name.is_empty()))
        .map(str::to_owned)
}

fn fgb_crs(crs: Option<&str>) -> FgbCrs<'_> {
    let Some(crs) = crs else {
        return FgbCrs::default();
    };
    if let Some((org, code)) = crs.split_once(':') {
        if let Ok(code) = code.parse::<i32>() {
            return FgbCrs {
                org: Some(org),
                code,
                ..Default::default()
            };
        }
        return FgbCrs {
            org: Some(org),
            code_string: Some(code),
            ..Default::default()
        };
    }
    FgbCrs {
        code_string: Some(crs),
        ..Default::default()
    }
}

fn fgb_error(path: &Path, details: impl Into<String>) -> OsmshrinkError {
    OsmshrinkError::GeoData {
        path: PathBuf::from(path),
        format: "FlatGeobuf",
        details: details.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn fixture() -> GeoDataset {
        GeoDataset {
            features: vec![
                GeoFeature {
                    id: Some(GeoFeatureId::String("point-a".to_owned())),
                    properties: GeoMetadata::from_iter([
                        ("name".to_owned(), json!("A")),
                        ("rank".to_owned(), json!(3)),
                        ("active".to_owned(), json!(true)),
                        ("nullable".to_owned(), Value::Null),
                        ("nested".to_owned(), json!({"source":"fixture"})),
                    ]),
                    geometry: Some(geojson::Geometry::new_point([8.7, 48.9])),
                    bbox: Some(vec![8.7, 48.9, 8.7, 48.9]),
                    metadata: GeoMetadata::from_iter([("source".to_owned(), json!("test"))]),
                },
                GeoFeature {
                    id: Some(GeoFeatureId::Number(serde_json::Number::from(2))),
                    properties: GeoMetadata::from_iter([
                        ("name".to_owned(), json!("B")),
                        ("rank".to_owned(), json!(4)),
                        ("active".to_owned(), json!(false)),
                        ("nested".to_owned(), json!([1, 2, 3])),
                    ]),
                    geometry: Some(geojson::Geometry::new_line_string(vec![
                        vec![8.7, 48.9],
                        vec![8.8, 49.0],
                    ])),
                    bbox: None,
                    metadata: GeoMetadata::new(),
                },
            ],
            bbox: Some(vec![8.0, 48.0, 9.0, 49.5]),
            crs: Some("EPSG:4326".to_owned()),
            metadata: GeoMetadata::from_iter([("dataset".to_owned(), json!("roundtrip"))]),
        }
    }

    #[test]
    fn round_trips_mixed_geometry_properties_and_metadata() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mixed.fgb");
        let expected = fixture();

        write_flatgeobuf_dataset(&path, &expected, false).unwrap();
        let actual = read_flatgeobuf_dataset(&path).unwrap();

        assert_eq!(actual.bbox, expected.bbox);
        assert_eq!(actual.crs, expected.crs);
        assert_eq!(actual.metadata, expected.metadata);
        assert_eq!(actual.features.len(), 2);
        assert_eq!(actual.features[0].id, expected.features[0].id);
        assert_eq!(actual.features[0].bbox, expected.features[0].bbox);
        assert_eq!(actual.features[0].metadata, expected.features[0].metadata);
        assert_eq!(actual.features[0].properties, expected.features[0].properties);
        assert_eq!(actual.features[1].id, expected.features[1].id);
        assert_eq!(actual.features[1].properties, expected.features[1].properties);
        assert_eq!(
            actual.features[0].geometry.as_ref().unwrap().value,
            expected.features[0].geometry.as_ref().unwrap().value
        );
        assert_eq!(
            actual.features[1].geometry.as_ref().unwrap().value,
            expected.features[1].geometry.as_ref().unwrap().value
        );
    }

    #[test]
    fn streams_ndjson_to_fgb_and_back() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input.ndjson");
        let fgb = dir.path().join("output.fgb");
        let output = dir.path().join("output.ndjson");
        let feature = &fixture().features[0];
        fs::write(&input, format!("{}\n", serde_json::to_string(feature).unwrap())).unwrap();

        assert_eq!(stream_ndjson_to_flatgeobuf(&input, &fgb).unwrap(), 1);
        let report = stream_flatgeobuf_to_ndjson(&fgb, &output).unwrap();
        assert_eq!(report.features, 1);

        let line = fs::read_to_string(&output).unwrap();
        let roundtrip: GeoFeature = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(roundtrip.id, feature.id);
        assert_eq!(roundtrip.properties, feature.properties);
        assert_eq!(roundtrip.metadata, feature.metadata);
    }

    #[test]
    fn special_column_name_does_not_clobber_user_property() {
        let mut feature = fixture().features.remove(0);
        feature
            .properties
            .insert(FEATURE_STATE_COLUMN.to_owned(), json!("user value"));
        let schema = PropertySchema::infer([&feature]);
        assert_eq!(schema.feature_state_column, "__osmshrink_feature_1");
    }
}
