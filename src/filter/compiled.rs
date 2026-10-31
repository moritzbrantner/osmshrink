use std::collections::HashSet;

use osmpbfreader::Tags;
use regex::Regex;

use crate::error::{OsmshrinkError, Result};
use crate::geometry::{BBox, Coordinate, Geometry};
use crate::model::Tags as NormalizedTags;
use crate::spec::{ElementType, FilterSpec, GeometryMode, IncludeRules, OutputField, TagCondition};

/// Checked for every element, so a plain lookup rather than a hashed set.
#[derive(Debug, Clone, Copy, Default)]
struct ElementTypeSet {
    node: bool,
    way: bool,
    relation: bool,
}

impl ElementTypeSet {
    fn from_types(types: &[ElementType]) -> Self {
        let mut set = Self::default();
        for element_type in types {
            match element_type {
                ElementType::Node => set.node = true,
                ElementType::Way => set.way = true,
                ElementType::Relation => set.relation = true,
            }
        }
        set
    }

    fn contains(self, element_type: ElementType) -> bool {
        match element_type {
            ElementType::Node => self.node,
            ElementType::Way => self.way,
            ElementType::Relation => self.relation,
        }
    }
}

#[derive(Debug)]
pub struct CompiledFilter {
    types: ElementTypeSet,
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
        let types = ElementTypeSet::from_types(
            spec.filter
                .types
                .as_deref()
                .unwrap_or(&[ElementType::Node, ElementType::Way]),
        );

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
        self.types.contains(element_type)
    }

    pub fn matches_node(&self, tags: &NormalizedTags, coordinate: Coordinate) -> bool {
        self.types.contains(ElementType::Node)
            && self.matches_node_tags(tags)
            && self.matches_node_bbox(coordinate)
    }

    fn matches_node_tags(&self, tags: &NormalizedTags) -> bool {
        self.types.contains(ElementType::Node) && self.matches_tags(tags)
    }

    pub(super) fn matches_node_osm_tags(&self, tags: &Tags) -> bool {
        self.types.contains(ElementType::Node) && self.matches_osm_tags(tags)
    }

    pub(super) fn matches_node_bbox(&self, coordinate: Coordinate) -> bool {
        self.bbox
            .map(|bbox| bbox.contains(coordinate))
            .unwrap_or(true)
    }

    pub fn matches_way(&self, tags: &NormalizedTags, coordinates: &[Coordinate]) -> bool {
        self.matches_way_tags(tags) && self.matches_way_geometry(coordinates)
    }

    fn matches_way_tags(&self, tags: &NormalizedTags) -> bool {
        self.types.contains(ElementType::Way) && self.matches_tags(tags)
    }

    pub(super) fn matches_way_osm_tags(&self, tags: &Tags) -> bool {
        self.types.contains(ElementType::Way) && self.matches_osm_tags(tags)
    }

    pub(super) fn matches_way_geometry(&self, coordinates: &[Coordinate]) -> bool {
        self.types.contains(ElementType::Way)
            && self
                .bbox
                .map(|bbox| bbox.intersects_any(coordinates))
                .unwrap_or(true)
    }

    pub(super) fn matches_relation_osm_tags(&self, tags: &Tags) -> bool {
        self.types.contains(ElementType::Relation) && self.matches_osm_tags(tags)
    }

    pub(super) fn matches_relation_geometry(&self, geometry: &Geometry) -> bool {
        self.types.contains(ElementType::Relation)
            && self
                .bbox
                .map(|bbox| geometry_intersects_bbox(geometry, bbox))
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

    fn matches_osm_tags(&self, tags: &Tags) -> bool {
        if self
            .exclude
            .iter()
            .any(|condition| condition.matches_osm(tags))
        {
            return false;
        }

        if !self.include_any.is_empty()
            && !self
                .include_any
                .iter()
                .any(|condition| condition.matches_osm(tags))
        {
            return false;
        }

        self.include_all
            .iter()
            .all(|condition| condition.matches_osm(tags))
    }
}

fn geometry_intersects_bbox(geometry: &Geometry, bbox: BBox) -> bool {
    match geometry {
        Geometry::Point { coordinates } => bbox.contains((*coordinates).into()),
        Geometry::LineString { coordinates } => coordinates
            .iter()
            .copied()
            .map(Coordinate::from)
            .any(|coordinate| bbox.contains(coordinate)),
        Geometry::Polygon { coordinates } => coordinates.iter().any(|ring| {
            ring.iter()
                .copied()
                .map(Coordinate::from)
                .any(|coordinate| bbox.contains(coordinate))
        }),
        Geometry::MultiPolygon { coordinates } => coordinates.iter().any(|polygon| {
            polygon.iter().any(|ring| {
                ring.iter()
                    .copied()
                    .map(Coordinate::from)
                    .any(|coordinate| bbox.contains(coordinate))
            })
        }),
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
        self.matches_value(tags.get(&self.key).map(String::as_str))
    }

    pub(super) fn matches_osm(&self, tags: &Tags) -> bool {
        let value = tags
            .iter()
            .find_map(|(key, value)| (key.as_str() == self.key.as_str()).then_some(value.as_str()));
        self.matches_value(value)
    }

    fn matches_value(&self, value: Option<&str>) -> bool {
        let matched = match &self.operator {
            ConditionOperator::Exists(expected) => value.is_some() == *expected,
            ConditionOperator::Value(expected) => value == Some(expected.as_str()),
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
