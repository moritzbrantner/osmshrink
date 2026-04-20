use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::PathBuf;

use osmpbfreader::{NodeId, OsmObj, OsmPbfReader, Tags};
use regex::Regex;
use tracing::warn;

use crate::error::{OsmshrinkError, Result};
use crate::geometry::{BBox, Coordinate, point, way_geometry};
use crate::model::{ElementKind, Feature, Tags as NormalizedTags};
use crate::output::OutputWriter;
use crate::spec::{
    ElementType, FilterSpec, GeometryMode, IncludeRules, OutputField, OutputFormat, TagCondition,
};

#[derive(Debug, Clone)]
pub struct FilterRunOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub spec: FilterSpec,
    pub format_override: Option<OutputFormat>,
}

#[derive(Debug, Clone)]
pub struct FilterReport {
    pub output: PathBuf,
    pub objects_written: u64,
    pub ways_skipped_missing_nodes: u64,
}

pub fn filter_pbf(options: FilterRunOptions) -> Result<FilterReport> {
    validate_input_path(&options.input)?;
    options.spec.validate()?;

    let format = options
        .format_override
        .unwrap_or_else(|| options.spec.output.format);
    let path_format = OutputFormat::from_output_path(&options.output)?;
    if path_format != format {
        return Err(OsmshrinkError::InvalidSpec(format!(
            "output extension does not match spec format `{format:?}`"
        )));
    }

    let compiled = CompiledFilter::compile(&options.spec)?;
    let mut output = OutputWriter::create(&options.output, format)?;
    output.set_fields(compiled.fields.clone());
    let mut node_index: HashMap<NodeId, Coordinate> = HashMap::new();
    let mut objects_written = 0_u64;

    let file = File::open(&options.input).map_err(|source| OsmshrinkError::ReadFile {
        path: options.input.clone(),
        source,
    })?;
    let mut reader = OsmPbfReader::new(file);
    for object in reader.iter() {
        let object = object.map_err(|source| OsmshrinkError::Pbf {
            path: options.input.clone(),
            source,
        })?;

        if let OsmObj::Node(node) = object {
            let coordinate = Coordinate::new(node.lon(), node.lat());
            node_index.insert(node.id, coordinate);

            let tags = normalize_tags(&node.tags);
            if compiled.matches_node(&tags, coordinate) {
                output.write_feature(&Feature {
                    id: node.id.0,
                    kind: ElementKind::Node,
                    tags,
                    geometry: point(coordinate),
                })?;
                objects_written += 1;
            }
        }
    }

    let mut ways_skipped_missing_nodes = 0_u64;
    if compiled.includes_type(ElementType::Way) {
        let file = File::open(&options.input).map_err(|source| OsmshrinkError::ReadFile {
            path: options.input.clone(),
            source,
        })?;
        let mut reader = OsmPbfReader::new(file);

        for object in reader.iter() {
            let object = object.map_err(|source| OsmshrinkError::Pbf {
                path: options.input.clone(),
                source,
            })?;

            if let OsmObj::Way(way) = object {
                let Some(coordinates) = coordinates_for_way(&way.nodes, &node_index) else {
                    ways_skipped_missing_nodes += 1;
                    warn!(
                        way_id = way.id.0,
                        "skipping way because one or more referenced nodes are missing"
                    );
                    continue;
                };

                let tags = normalize_tags(&way.tags);
                if compiled.matches_way(&tags, &coordinates) {
                    let is_closed = way.nodes.first() == way.nodes.last();
                    output.write_feature(&Feature {
                        id: way.id.0,
                        kind: ElementKind::Way,
                        tags,
                        geometry: way_geometry(&coordinates, is_closed, compiled.geometry_mode),
                    })?;
                    objects_written += 1;
                }
            }
        }
    }

    output.finish()?;

    Ok(FilterReport {
        output: options.output,
        objects_written,
        ways_skipped_missing_nodes,
    })
}

fn validate_input_path(path: &std::path::Path) -> Result<()> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if filename.ends_with(".osm.pbf") || filename.ends_with(".pbf") {
        Ok(())
    } else {
        Err(OsmshrinkError::UnsupportedInputFile {
            path: path.to_path_buf(),
        })
    }
}

fn coordinates_for_way(
    nodes: &[NodeId],
    node_index: &HashMap<NodeId, Coordinate>,
) -> Option<Vec<Coordinate>> {
    nodes
        .iter()
        .map(|node_id| node_index.get(node_id).copied())
        .collect()
}

fn normalize_tags(tags: &Tags) -> NormalizedTags {
    tags.iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[derive(Debug)]
pub struct CompiledFilter {
    types: HashSet<ElementType>,
    bbox: Option<BBox>,
    include_any: Vec<CompiledCondition>,
    include_all: Vec<CompiledCondition>,
    exclude: Vec<CompiledCondition>,
    pub geometry_mode: GeometryMode,
    pub fields: Vec<OutputField>,
}

impl CompiledFilter {
    pub fn compile(spec: &FilterSpec) -> Result<Self> {
        spec.validate()?;
        let types: HashSet<ElementType> = spec
            .filter
            .types
            .clone()
            .unwrap_or_else(|| vec![ElementType::Node, ElementType::Way])
            .into_iter()
            .collect();

        if types.contains(&ElementType::Relation) {
            return Err(OsmshrinkError::UnsupportedRelations);
        }

        let IncludeRules { any, all } = spec.filter.include.clone().unwrap_or_default();

        Ok(Self {
            types,
            bbox: spec.filter.bbox.map(BBox::new),
            include_any: compile_conditions(&any)?,
            include_all: compile_conditions(&all)?,
            exclude: compile_conditions(&spec.filter.exclude)?,
            geometry_mode: spec.output.geometry,
            fields: spec.output.fields.clone(),
        })
    }

    pub fn includes_type(&self, element_type: ElementType) -> bool {
        self.types.contains(&element_type)
    }

    pub fn matches_node(&self, tags: &NormalizedTags, coordinate: Coordinate) -> bool {
        self.types.contains(&ElementType::Node)
            && self.matches_tags(tags)
            && self
                .bbox
                .map(|bbox| bbox.contains(coordinate))
                .unwrap_or(true)
    }

    pub fn matches_way(&self, tags: &NormalizedTags, coordinates: &[Coordinate]) -> bool {
        self.types.contains(&ElementType::Way)
            && self.matches_tags(tags)
            && self
                .bbox
                .map(|bbox| bbox.intersects_any(coordinates))
                .unwrap_or(true)
    }

    fn matches_tags(&self, tags: &NormalizedTags) -> bool {
        if self.exclude.iter().any(|condition| condition.matches(tags)) {
            return false;
        }

        if !self.include_any.is_empty()
            && !self
                .include_any
                .iter()
                .any(|condition| condition.matches(tags))
        {
            return false;
        }

        self.include_all
            .iter()
            .all(|condition| condition.matches(tags))
    }
}

fn compile_conditions(conditions: &[TagCondition]) -> Result<Vec<CompiledCondition>> {
    conditions.iter().map(CompiledCondition::compile).collect()
}

#[derive(Debug)]
pub struct CompiledCondition {
    key: String,
    operator: ConditionOperator,
    negate: bool,
}

impl CompiledCondition {
    pub fn compile(condition: &TagCondition) -> Result<Self> {
        condition.validate()?;
        let operator = if let Some(exists) = condition.exists {
            ConditionOperator::Exists(exists)
        } else if let Some(value) = &condition.value {
            ConditionOperator::Value(value.clone())
        } else if let Some(values) = &condition.values {
            ConditionOperator::Values(values.iter().cloned().collect())
        } else if let Some(pattern) = &condition.regex {
            ConditionOperator::Regex(Regex::new(pattern).map_err(|source| {
                OsmshrinkError::Regex {
                    pattern: pattern.clone(),
                    source,
                }
            })?)
        } else {
            return Err(OsmshrinkError::InvalidSpec(format!(
                "condition for key `{}` is missing an operator",
                condition.key
            )));
        };

        Ok(Self {
            key: condition.key.clone(),
            operator,
            negate: condition.negate,
        })
    }

    pub fn matches(&self, tags: &NormalizedTags) -> bool {
        let value = tags.get(&self.key);
        let matched = match &self.operator {
            ConditionOperator::Exists(expected) => value.is_some() == *expected,
            ConditionOperator::Value(expected) => value == Some(expected),
            ConditionOperator::Values(expected) => {
                value.map(|value| expected.contains(value)).unwrap_or(false)
            }
            ConditionOperator::Regex(regex) => {
                value.map(|value| regex.is_match(value)).unwrap_or(false)
            }
        };

        if self.negate { !matched } else { matched }
    }
}

#[derive(Debug)]
enum ConditionOperator {
    Exists(bool),
    Value(String),
    Values(HashSet<String>),
    Regex(Regex),
}

#[cfg(test)]
mod tests {
    use crate::spec::{FilterRules, OutputSpec};

    use super::*;

    fn tags(values: &[(&str, &str)]) -> NormalizedTags {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn condition_matches_values() {
        let condition = CompiledCondition::compile(&TagCondition {
            key: "amenity".to_owned(),
            exists: None,
            value: None,
            values: Some(vec!["school".to_owned(), "hospital".to_owned()]),
            regex: None,
            negate: false,
        })
        .unwrap();

        assert!(condition.matches(&tags(&[("amenity", "school")])));
        assert!(!condition.matches(&tags(&[("amenity", "cafe")])));
    }

    #[test]
    fn condition_supports_negation() {
        let condition = CompiledCondition::compile(&TagCondition {
            key: "access".to_owned(),
            exists: None,
            value: Some("private".to_owned()),
            values: None,
            regex: None,
            negate: true,
        })
        .unwrap();

        assert!(!condition.matches(&tags(&[("access", "private")])));
        assert!(condition.matches(&tags(&[("access", "yes")])));
    }

    #[test]
    fn compiled_filter_applies_include_and_exclude() {
        let spec = FilterSpec {
            source: None,
            filter: FilterRules {
                bbox: None,
                types: Some(vec![ElementType::Node]),
                include: Some(IncludeRules {
                    any: vec![TagCondition {
                        key: "amenity".to_owned(),
                        exists: None,
                        value: Some("school".to_owned()),
                        values: None,
                        regex: None,
                        negate: false,
                    }],
                    all: vec![TagCondition {
                        key: "name".to_owned(),
                        exists: Some(true),
                        value: None,
                        values: None,
                        regex: None,
                        negate: false,
                    }],
                }),
                exclude: vec![TagCondition {
                    key: "access".to_owned(),
                    exists: None,
                    value: Some("private".to_owned()),
                    values: None,
                    regex: None,
                    negate: false,
                }],
            },
            output: OutputSpec::default(),
        };
        let filter = CompiledFilter::compile(&spec).unwrap();

        assert!(filter.matches_node(
            &tags(&[("amenity", "school"), ("name", "Primary")]),
            Coordinate::new(8.7, 48.9)
        ));
        assert!(!filter.matches_node(
            &tags(&[
                ("amenity", "school"),
                ("name", "Primary"),
                ("access", "private")
            ]),
            Coordinate::new(8.7, 48.9)
        ));
        assert!(!filter.matches_node(&tags(&[("amenity", "school")]), Coordinate::new(8.7, 48.9)));
    }

    #[test]
    fn compiled_filter_applies_bbox_to_ways() {
        let spec = FilterSpec {
            source: None,
            filter: FilterRules {
                bbox: Some([8.5, 48.8, 9.3, 49.2]),
                types: Some(vec![ElementType::Way]),
                include: None,
                exclude: Vec::new(),
            },
            output: OutputSpec::default(),
        };
        let filter = CompiledFilter::compile(&spec).unwrap();

        assert!(filter.matches_way(
            &tags(&[]),
            &[Coordinate::new(8.7, 48.9), Coordinate::new(10.0, 49.0)]
        ));
        assert!(!filter.matches_way(&tags(&[]), &[Coordinate::new(10.0, 49.0)]));
    }
}
