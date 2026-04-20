use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{OsmshrinkError, Result};
use crate::geofabrik::geofabrik_url_for_region;
use crate::geometry::BBox;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilterSpec {
    #[serde(default)]
    pub source: Option<SourceSpec>,
    #[serde(default)]
    pub filter: FilterRules,
    #[serde(default)]
    pub processing: ProcessingSpec,
    #[serde(default)]
    pub output: OutputSpec,
}

impl Default for FilterSpec {
    fn default() -> Self {
        Self {
            source: None,
            filter: FilterRules::default(),
            processing: ProcessingSpec::default(),
            output: OutputSpec::default(),
        }
    }
}

impl FilterSpec {
    pub fn from_path(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).map_err(|source| OsmshrinkError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;

        let spec = match path.extension().and_then(|extension| extension.to_str()) {
            Some("json") => {
                serde_json::from_str(&contents).map_err(|source| OsmshrinkError::ParseSpec {
                    path: path.to_path_buf(),
                    format: "JSON",
                    source: Box::new(source),
                })?
            }
            Some("yaml") | Some("yml") => {
                serde_yaml::from_str(&contents).map_err(|source| OsmshrinkError::ParseSpec {
                    path: path.to_path_buf(),
                    format: "YAML",
                    source: Box::new(source),
                })?
            }
            _ => parse_spec_unknown_extension(path, &contents)?,
        };

        Ok(spec)
    }

    pub fn from_filter_arg(input: &str) -> Result<Self> {
        let path = Path::new(input);
        if path.exists() || looks_like_spec_path(path) {
            Self::from_path(path)
        } else {
            Self::from_inline(input)
        }
    }

    pub fn from_inline(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(OsmshrinkError::InvalidSpec(
                "inline filter must not be empty".to_owned(),
            ));
        }

        if let Ok(spec) = parse_inline::<FilterSpec>(input) {
            return Ok(spec);
        }

        if let Ok(filter) = parse_inline::<FilterRules>(input) {
            return Ok(Self {
                filter,
                ..Self::default()
            });
        }

        if let Ok(condition) = parse_inline::<TagCondition>(input) {
            return Ok(Self::from_include_all(vec![condition]));
        }

        if let Ok(conditions) = parse_inline::<Vec<TagCondition>>(input) {
            return Ok(Self::from_include_all(conditions));
        }

        if let Some(condition) = parse_tag_condition_expression(input) {
            return Ok(Self::from_include_all(vec![condition]));
        }

        Err(OsmshrinkError::InvalidSpec(
            "inline filter must be a JSON/YAML filter spec, filter rules object, tag condition, list of tag conditions, or key=value condition".to_owned(),
        ))
    }

    fn from_include_all(conditions: Vec<TagCondition>) -> Self {
        Self {
            filter: FilterRules {
                include: Some(IncludeRules {
                    all: conditions,
                    any: Vec::new(),
                }),
                ..FilterRules::default()
            },
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(source) = &self.source {
            source.validate()?;
        }
        self.filter.validate()?;
        self.processing.validate()?;
        self.output.validate()?;
        Ok(())
    }
}

fn looks_like_spec_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "yaml" | "yml")
    )
}

fn parse_inline<T>(input: &str) -> std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(input)
        .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
        .or_else(|_| {
            serde_yaml::from_str(input)
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
        })
}

fn parse_tag_condition_expression(input: &str) -> Option<TagCondition> {
    let input = input.trim();
    if input.starts_with('{') || input.starts_with('[') || input.is_empty() {
        return None;
    }

    for (operator, negate) in [("!=", true), ("=", false)] {
        if let Some((key, value)) = input.split_once(operator) {
            let key = parse_expression_key(key)?;
            return Some(TagCondition {
                key,
                exists: None,
                value: Some(value.trim().to_owned()),
                values: None,
                regex: None,
                negate,
            });
        }
    }

    if let Some((key, pattern)) = input.split_once('~') {
        let key = parse_expression_key(key)?;
        return Some(TagCondition {
            key,
            exists: None,
            value: None,
            values: None,
            regex: Some(pattern.trim().to_owned()),
            negate: false,
        });
    }

    let key = parse_expression_key(input)?;
    Some(TagCondition {
        key,
        exists: Some(true),
        value: None,
        values: None,
        regex: None,
        negate: false,
    })
}

fn parse_expression_key(key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        None
    } else {
        Some(key.to_owned())
    }
}

fn parse_spec_unknown_extension(path: &Path, contents: &str) -> Result<FilterSpec> {
    serde_json::from_str(contents)
        .or_else(|_| serde_yaml::from_str(contents))
        .map_err(|source| OsmshrinkError::ParseSpec {
            path: path.to_path_buf(),
            format: "JSON or YAML",
            source: Box::new(source),
        })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpec {
    pub provider: String,
    pub region: Option<String>,
    pub url: Option<String>,
}

impl SourceSpec {
    pub fn validate(&self) -> Result<()> {
        match self.provider.as_str() {
            "geofabrik" => {
                if self.url.is_none() && self.region.is_none() {
                    return Err(OsmshrinkError::InvalidSpec(
                        "source.provider geofabrik requires either region or url".to_owned(),
                    ));
                }
                if let Some(region) = &self.region {
                    geofabrik_url_for_region(region)?;
                }
                Ok(())
            }
            provider => Err(OsmshrinkError::InvalidSpec(format!(
                "unsupported source provider `{provider}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilterRules {
    pub bbox: Option<[f64; 4]>,
    pub types: Option<Vec<ElementType>>,
    pub include: Option<IncludeRules>,
    pub exclude: Vec<TagCondition>,
}

impl FilterRules {
    pub fn validate(&self) -> Result<()> {
        if let Some(bbox) = self.bbox {
            BBox::new(bbox)
                .validate()
                .map_err(OsmshrinkError::InvalidSpec)?;
        }

        let types = self
            .types
            .clone()
            .unwrap_or_else(|| vec![ElementType::Node, ElementType::Way]);
        if types.is_empty() {
            return Err(OsmshrinkError::InvalidSpec(
                "filter.types must not be empty".to_owned(),
            ));
        }
        if let Some(include) = &self.include {
            include.validate()?;
        }
        for condition in &self.exclude {
            condition.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct IncludeRules {
    pub any: Vec<TagCondition>,
    pub all: Vec<TagCondition>,
}

impl IncludeRules {
    fn validate(&self) -> Result<()> {
        for condition in self.any.iter().chain(self.all.iter()) {
            condition.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TagCondition {
    pub key: String,
    pub exists: Option<bool>,
    pub value: Option<String>,
    pub values: Option<Vec<String>>,
    pub regex: Option<String>,
    #[serde(default, rename = "not")]
    pub negate: bool,
}

impl TagCondition {
    pub fn validate(&self) -> Result<()> {
        if self.key.trim().is_empty() {
            return Err(OsmshrinkError::InvalidSpec(
                "condition key must not be empty".to_owned(),
            ));
        }

        let operator_count = usize::from(self.exists.is_some())
            + usize::from(self.value.is_some())
            + usize::from(self.values.is_some())
            + usize::from(self.regex.is_some());
        if operator_count == 0 {
            return Err(OsmshrinkError::InvalidSpec(format!(
                "condition for key `{}` must include exists, value, values, or regex",
                self.key
            )));
        }
        if operator_count > 1 {
            return Err(OsmshrinkError::InvalidSpec(format!(
                "condition for key `{}` must use only one of exists, value, values, or regex",
                self.key
            )));
        }

        if let Some(values) = &self.values
            && values.is_empty()
        {
            return Err(OsmshrinkError::InvalidSpec(format!(
                "condition values for key `{}` must not be empty",
                self.key
            )));
        }

        if let Some(pattern) = &self.regex {
            Regex::new(pattern).map_err(|source| OsmshrinkError::Regex {
                pattern: pattern.clone(),
                source,
            })?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum ElementType {
    Node,
    Way,
    Relation,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessingSpec {
    pub index: IndexSpec,
}

impl ProcessingSpec {
    pub fn validate(&self) -> Result<()> {
        self.index.validate()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct IndexSpec {
    pub mode: IndexMode,
    pub memory_node_limit: usize,
    pub disk_dir: Option<PathBuf>,
}

impl Default for IndexSpec {
    fn default() -> Self {
        Self {
            mode: IndexMode::Auto,
            memory_node_limit: 5_000_000,
            disk_dir: None,
        }
    }
}

impl IndexSpec {
    fn validate(&self) -> Result<()> {
        if self.memory_node_limit == 0 {
            return Err(OsmshrinkError::InvalidSpec(
                "processing.index.memory_node_limit must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum IndexMode {
    #[default]
    Auto,
    Memory,
    Disk,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputSpec {
    pub format: OutputFormat,
    pub geometry: GeometryMode,
    pub fields: Vec<OutputField>,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Ndjson,
            geometry: GeometryMode::Full,
            fields: OutputField::defaults(),
        }
    }
}

impl OutputSpec {
    pub fn validate(&self) -> Result<()> {
        if self.fields.is_empty() {
            return Err(OsmshrinkError::InvalidSpec(
                "output.fields must not be empty".to_owned(),
            ));
        }
        if self.format == OutputFormat::Geojson && !self.fields.contains(&OutputField::Geometry) {
            return Err(OsmshrinkError::InvalidSpec(
                "output.fields must include geometry when output.format is geojson".to_owned(),
            ));
        }

        let mut seen = HashSet::new();
        for field in &self.fields {
            if !seen.insert(*field) {
                return Err(OsmshrinkError::InvalidSpec(format!(
                    "output.fields contains duplicate `{}`",
                    field.as_str()
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum OutputFormat {
    #[default]
    Ndjson,
    Json,
    Geojson,
}

impl OutputFormat {
    pub fn from_output_path(path: &Path) -> Result<Self> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("ndjson") => Ok(Self::Ndjson),
            Some("json") => Ok(Self::Json),
            Some("geojson") => Ok(Self::Geojson),
            _ => Err(OsmshrinkError::UnsupportedOutputFile {
                path: PathBuf::from(path),
                expected: ".ndjson, .json, or .geojson",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryMode {
    #[default]
    Full,
    Polygon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputField {
    Id,
    Type,
    Tags,
    Geometry,
}

impl OutputField {
    pub fn defaults() -> Vec<Self> {
        vec![Self::Id, Self::Type, Self::Tags, Self::Geometry]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Type => "type",
            Self::Tags => "tags",
            Self::Geometry => "geometry",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_SPEC: &str = r#"
    {
      "source": {
        "provider": "geofabrik",
        "region": "europe/germany/baden-wuerttemberg"
      },
      "filter": {
        "bbox": [8.5, 48.8, 9.3, 49.2],
        "types": ["node", "way"],
        "include": {
          "any": [
            { "key": "amenity", "values": ["school", "hospital"] }
          ],
          "all": [
            { "key": "name", "exists": true }
          ]
        },
        "exclude": [
          { "key": "access", "values": ["private"] }
        ]
      },
      "output": {
        "format": "ndjson",
        "geometry": "full",
        "fields": ["id", "type", "tags", "geometry"]
      }
    }
    "#;

    #[test]
    fn parses_spec_from_json() {
        let spec: FilterSpec = serde_json::from_str(JSON_SPEC).unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.output.format, OutputFormat::Ndjson);
        assert_eq!(spec.processing.index.mode, IndexMode::Auto);
    }

    #[test]
    fn parses_spec_from_yaml() {
        let yaml = r#"
source:
  provider: geofabrik
  region: europe/germany/baden-wuerttemberg
filter:
  types: [node, way]
  include:
    any:
      - key: highway
        values: [primary, secondary]
output:
  format: json
  geometry: full
  fields: [id, geometry]
"#;
        let spec: FilterSpec = serde_yaml::from_str(yaml).unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.output.format, OutputFormat::Json);
        assert_eq!(
            spec.output.fields,
            vec![OutputField::Id, OutputField::Geometry]
        );
    }

    #[test]
    fn parses_processing_index_options() {
        let yaml = r#"
processing:
  index:
    mode: disk
    memory_node_limit: 12
    disk_dir: /tmp/osmshrink-index
output:
  format: ndjson
"#;
        let spec: FilterSpec = serde_yaml::from_str(yaml).unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.processing.index.mode, IndexMode::Disk);
        assert_eq!(spec.processing.index.memory_node_limit, 12);
        assert_eq!(
            spec.processing.index.disk_dir,
            Some(PathBuf::from("/tmp/osmshrink-index"))
        );
    }

    #[test]
    fn parses_inline_full_filter_spec() {
        let spec =
            FilterSpec::from_inline(r#"{"filter":{"types":["node"]},"output":{"format":"json"}}"#)
                .unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.filter.types, Some(vec![ElementType::Node]));
        assert_eq!(spec.output.format, OutputFormat::Json);
    }

    #[test]
    fn parses_inline_filter_rules() {
        let spec = FilterSpec::from_inline(
            r#"{types: [way], include: {any: [{key: highway, value: primary}]}}"#,
        )
        .unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.filter.types, Some(vec![ElementType::Way]));
        assert_eq!(spec.filter.include.unwrap().any[0].key, "highway");
    }

    #[test]
    fn parses_inline_tag_condition_as_include_all() {
        let spec = FilterSpec::from_inline(r#"{key: amenity, value: school}"#).unwrap();
        spec.validate().unwrap();
        let include = spec.filter.include.unwrap();
        assert_eq!(include.all[0].key, "amenity");
        assert_eq!(include.all[0].value.as_deref(), Some("school"));
    }

    #[test]
    fn parses_inline_key_value_expression_as_tag_condition() {
        let spec = FilterSpec::from_inline("amenity=school").unwrap();
        spec.validate().unwrap();
        let include = spec.filter.include.unwrap();
        assert_eq!(include.all[0].key, "amenity");
        assert_eq!(include.all[0].value.as_deref(), Some("school"));
    }

    #[test]
    fn geojson_requires_geometry_field() {
        let yaml = r#"
output:
  format: geojson
  fields: [id, tags]
"#;
        let spec: FilterSpec = serde_yaml::from_str(yaml).unwrap();
        let error = spec.validate().unwrap_err().to_string();
        assert!(error.contains("must include geometry"));
    }

    #[test]
    fn detects_geojson_output_extension() {
        assert_eq!(
            OutputFormat::from_output_path(Path::new("areas.geojson")).unwrap(),
            OutputFormat::Geojson
        );
    }
}
