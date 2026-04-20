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
    pub output: OutputSpec,
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

    pub fn validate(&self) -> Result<()> {
        if let Some(source) = &self.source {
            source.validate()?;
        }
        self.filter.validate()?;
        self.output.validate()?;
        Ok(())
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
        if types.contains(&ElementType::Relation) {
            return Err(OsmshrinkError::UnsupportedRelations);
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct IncludeRules {
    pub any: Vec<TagCondition>,
    pub all: Vec<TagCondition>,
}

impl Default for IncludeRules {
    fn default() -> Self {
        Self {
            any: Vec::new(),
            all: Vec::new(),
        }
    }
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

        if let Some(values) = &self.values {
            if values.is_empty() {
                return Err(OsmshrinkError::InvalidSpec(format!(
                    "condition values for key `{}` must not be empty",
                    self.key
                )));
            }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum OutputFormat {
    Ndjson,
    Json,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Ndjson
    }
}

impl OutputFormat {
    pub fn from_output_path(path: &Path) -> Result<Self> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("ndjson") => Ok(Self::Ndjson),
            Some("json") => Ok(Self::Json),
            _ => Err(OsmshrinkError::UnsupportedOutputFile {
                path: PathBuf::from(path),
                expected: ".ndjson or .json",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryMode {
    Full,
    Polygon,
}

impl Default for GeometryMode {
    fn default() -> Self {
        Self::Full
    }
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
}
