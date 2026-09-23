use std::collections::{HashMap, HashSet};
#[cfg(feature = "cli")]
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use osmpbfreader::{NodeId, OsmObj, OsmPbfReader, Relation, RelationId, Tags, Way, WayId};
use regex::Regex;
use tracing::warn;

use crate::error::{OsmshrinkError, Result};
use crate::geometry::{
    BBox, Coordinate, Geometry, normalize_ring_orientation, point, point_in_ring,
    polygon_or_multipolygon, ring_area, way_geometry,
};
use crate::index::{AutoNodeIndex, IndexBackend, IndexOptions, NodeIndex, StoredCoordinate};
use crate::model::{ElementKind, Feature, Tags as NormalizedTags};
#[cfg(feature = "cli")]
use crate::output::OutputWriter;
#[cfg(feature = "cli")]
use crate::spec::OutputFormat;
use crate::spec::{ElementType, FilterSpec, GeometryMode, IncludeRules, OutputField, TagCondition};

const NODE_INDEX_BATCH_SIZE: usize = 16_384;

#[derive(Debug, Clone)]
#[cfg(feature = "cli")]
pub struct FilterRunOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub spec: FilterSpec,
    pub format_override: Option<OutputFormat>,
    pub index_options: IndexOptions,
}

#[derive(Debug, Clone)]
#[cfg(feature = "cli")]
pub struct CollectRunOptions {
    pub input: PathBuf,
    pub spec: FilterSpec,
    pub index_options: IndexOptions,
}

#[derive(Debug, Clone)]
pub struct CollectBytesOptions<'a> {
    pub input: &'a [u8],
    pub spec: FilterSpec,
    pub index_options: IndexOptions,
}

#[derive(Debug, Clone)]
pub struct CollectedFeatures {
    pub features: Vec<Feature>,
    pub report: CollectReport,
}

#[derive(Debug, Clone)]
pub struct CollectReport {
    pub objects_collected: u64,
    pub ways_skipped_missing_nodes: u64,
    pub relations_skipped_non_area: u64,
    pub relations_skipped_missing_members: u64,
    pub relations_skipped_invalid_rings: u64,
    pub relation_members_ignored_role: u64,
    pub index_backend: IndexBackend,
}

impl From<FilterReport> for CollectReport {
    fn from(report: FilterReport) -> Self {
        Self {
            objects_collected: report.objects_written,
            ways_skipped_missing_nodes: report.ways_skipped_missing_nodes,
            relations_skipped_non_area: report.relations_skipped_non_area,
            relations_skipped_missing_members: report.relations_skipped_missing_members,
            relations_skipped_invalid_rings: report.relations_skipped_invalid_rings,
            relation_members_ignored_role: report.relation_members_ignored_role,
            index_backend: report.index_backend,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterReport {
    pub output: PathBuf,
    pub objects_written: u64,
    pub ways_skipped_missing_nodes: u64,
    pub relations_skipped_non_area: u64,
    pub relations_skipped_missing_members: u64,
    pub relations_skipped_invalid_rings: u64,
    pub relation_members_ignored_role: u64,
    pub index_backend: IndexBackend,
}

impl FilterReport {
    fn new(output: PathBuf) -> Self {
        Self {
            output,
            objects_written: 0,
            ways_skipped_missing_nodes: 0,
            relations_skipped_non_area: 0,
            relations_skipped_missing_members: 0,
            relations_skipped_invalid_rings: 0,
            relation_members_ignored_role: 0,
            index_backend: IndexBackend::Memory,
        }
    }
}

#[cfg(feature = "cli")]
pub fn filter_pbf(options: FilterRunOptions) -> Result<FilterReport> {
    validate_input_path(&options.input)?;
    let format = options
        .format_override
        .unwrap_or(options.spec.output.format);
    let mut effective_output = options.spec.output.clone();
    effective_output.format = format;
    effective_output.validate()?;

    let path_format = OutputFormat::from_output_path(&options.output)?;
    if path_format != format {
        return Err(OsmshrinkError::InvalidSpec(format!(
            "output extension does not match spec format `{format:?}`"
        )));
    }

    let effective_spec = FilterSpec {
        output: effective_output.clone(),
        ..options.spec.clone()
    };
    let compiled = CompiledFilter::compile(&effective_spec)?;
    let mut output = OutputWriter::create(&options.output, format)?;
    output.set_fields(compiled.fields.clone());
    let mut report = FilterReport::new(options.output.clone());

    run_filter_pipeline_from_path(
        options.input.clone(),
        &compiled,
        options.index_options,
        &mut output,
        &mut report,
    )?;
    output.finish()?;
    Ok(report)
}

#[cfg(feature = "cli")]
pub fn collect_pbf(options: CollectRunOptions) -> Result<CollectedFeatures> {
    validate_input_path(&options.input)?;
    let compiled = CompiledFilter::compile(&options.spec)?;
    let mut sink = VecFeatureSink::new();
    let mut report = FilterReport::new(PathBuf::from("<memory>"));

    run_filter_pipeline_from_path(
        options.input.clone(),
        &compiled,
        options.index_options,
        &mut sink,
        &mut report,
    )?;

    Ok(CollectedFeatures {
        features: sink.features,
        report: report.into(),
    })
}

pub fn collect_pbf_bytes(options: CollectBytesOptions<'_>) -> Result<CollectedFeatures> {
    let compiled = CompiledFilter::compile(&options.spec)?;
    let mut sink = VecFeatureSink::new();
    let mut report = FilterReport::new(PathBuf::from("<memory>"));

    run_filter_pipeline(
        PathBuf::from("<memory>"),
        || Ok(Cursor::new(options.input)),
        &compiled,
        options.index_options,
        &mut sink,
        &mut report,
    )?;

    Ok(CollectedFeatures {
        features: sink.features,
        report: report.into(),
    })
}

#[cfg(feature = "cli")]
fn run_filter_pipeline_from_path(
    input: PathBuf,
    compiled: &CompiledFilter,
    index_options: IndexOptions,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()> {
    run_filter_pipeline(
        input.clone(),
        || {
            File::open(&input).map_err(|source| OsmshrinkError::ReadFile {
                path: input.clone(),
                source,
            })
        },
        compiled,
        index_options,
        sink,
        report,
    )
}

fn run_filter_pipeline<R, OpenReader>(
    input: PathBuf,
    mut open_reader: OpenReader,
    compiled: &CompiledFilter,
    index_options: IndexOptions,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()>
where
    R: Read,
    OpenReader: FnMut() -> Result<R>,
{
    let includes_way = compiled.includes_type(ElementType::Way);
    let includes_relation = compiled.includes_type(ElementType::Relation);
    let needs_node_index = includes_way || includes_relation;
    let mut node_index = if needs_node_index {
        Some(AutoNodeIndex::create(index_options)?)
    } else {
        None
    };
    let mut candidates = Vec::new();
    let mut required_way_ids = HashSet::new();

    process_first_pass_from_reader(
        &input,
        open_reader()?,
        compiled,
        node_index.as_mut().map(|index| index as &mut dyn NodeIndex),
        sink,
        &mut candidates,
        &mut required_way_ids,
        report,
    )?;

    let mut relation_way_geometries = HashMap::new();
    if needs_node_index && (includes_way || !required_way_ids.is_empty()) {
        process_ways_from_reader(
            &input,
            open_reader()?,
            compiled,
            node_index
                .as_ref()
                .expect("node index exists when ways are processed"),
            &required_way_ids,
            &mut relation_way_geometries,
            sink,
            report,
        )?;
    }

    if includes_relation {
        emit_relations(
            compiled,
            &candidates,
            &relation_way_geometries,
            sink,
            report,
        )?;
    }

    if let Some(node_index) = &node_index {
        report.index_backend = node_index.backend();
    }
    Ok(())
}

fn process_first_pass_from_reader<R: Read>(
    input: &std::path::Path,
    reader: R,
    compiled: &CompiledFilter,
    mut node_index: Option<&mut dyn NodeIndex>,
    sink: &mut dyn FeatureSink,
    candidates: &mut Vec<CandidateRelation>,
    required_way_ids: &mut HashSet<WayId>,
    report: &mut FilterReport,
) -> Result<()> {
    let mut reader = OsmPbfReader::new(reader);
    let mut node_batch = Vec::with_capacity(NODE_INDEX_BATCH_SIZE);
    let mut node_features = Vec::new();
    let should_index_nodes = node_index.is_some();

    for object in reader.iter() {
        let object = object.map_err(|source| OsmshrinkError::Pbf {
            path: input.to_path_buf(),
            source,
        })?;
        match object {
            OsmObj::Node(node) => {
                process_node(
                    node.id,
                    StoredCoordinate::new(node.decimicro_lon, node.decimicro_lat),
                    &node.tags,
                    NodeProcessingContext {
                        compiled,
                        should_index_node: should_index_nodes,
                        node_batch: &mut node_batch,
                        node_features: &mut node_features,
                        report,
                    },
                );
                if node_batch.len() >= NODE_INDEX_BATCH_SIZE {
                    if let Some(index) = node_index.as_mut() {
                        flush_indexed_node_batch(
                            &mut **index,
                            &mut node_batch,
                            &mut node_features,
                            sink,
                        )?;
                    }
                } else if !should_index_nodes {
                    flush_node_features(&mut node_features, sink)?;
                }
            }
            OsmObj::Relation(relation) if compiled.includes_type(ElementType::Relation) => {
                collect_relation(relation, compiled, candidates, required_way_ids, report);
            }
            _ => {}
        }
    }

    if let Some(index) = node_index.as_mut() {
        flush_indexed_node_batch(&mut **index, &mut node_batch, &mut node_features, sink)?;
    }
    flush_node_features(&mut node_features, sink)?;
    Ok(())
}

fn flush_indexed_node_batch(
    node_index: &mut dyn NodeIndex,
    node_batch: &mut Vec<(NodeId, StoredCoordinate)>,
    node_features: &mut Vec<Feature>,
    sink: &mut dyn FeatureSink,
) -> Result<()> {
    if !node_batch.is_empty() {
        node_index.insert_batch(node_batch)?;
        node_batch.clear();
    }
    flush_node_features(node_features, sink)
}

fn flush_node_features(node_features: &mut Vec<Feature>, sink: &mut dyn FeatureSink) -> Result<()> {
    for feature in node_features.drain(..) {
        sink.write_feature(feature)?;
    }
    Ok(())
}

#[cfg(test)]
fn process_node_for_test(
    node_id: NodeId,
    coordinate: StoredCoordinate,
    tags: &Tags,
    compiled: &CompiledFilter,
    node_index: &mut dyn NodeIndex,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()> {
    let mut node_batch = Vec::with_capacity(1);
    let mut node_features = Vec::new();
    process_node(
        node_id,
        coordinate,
        tags,
        NodeProcessingContext {
            compiled,
            should_index_node: true,
            node_batch: &mut node_batch,
            node_features: &mut node_features,
            report,
        },
    );
    node_index.insert_batch(&node_batch)?;
    flush_node_features(&mut node_features, sink)
}

struct NodeProcessingContext<'a> {
    compiled: &'a CompiledFilter,
    should_index_node: bool,
    node_batch: &'a mut Vec<(NodeId, StoredCoordinate)>,
    node_features: &'a mut Vec<Feature>,
    report: &'a mut FilterReport,
}

fn process_node(
    node_id: NodeId,
    coordinate: StoredCoordinate,
    tags: &Tags,
    context: NodeProcessingContext<'_>,
) {
    if context.should_index_node {
        context.node_batch.push((node_id, coordinate));
    }

    if !context.compiled.includes_type(ElementType::Node) {
        return;
    }

    let coordinate = coordinate.to_coordinate();
    if !context.compiled.matches_node_bbox(coordinate) {
        return;
    }

    let tags = normalize_tags(tags);
    if context.compiled.matches_node_tags(&tags) {
        context.node_features.push(Feature {
            id: node_id.0,
            kind: ElementKind::Node,
            tags,
            geometry: point(coordinate),
        });
        context.report.objects_written += 1;
    }
}

fn process_ways_from_reader<R: Read>(
    input: &std::path::Path,
    reader: R,
    compiled: &CompiledFilter,
    node_index: &dyn NodeIndex,
    required_way_ids: &HashSet<WayId>,
    relation_way_geometries: &mut HashMap<WayId, Vec<Coordinate>>,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()> {
    let mut reader = OsmPbfReader::new(reader);
    for object in reader.iter() {
        let object = object.map_err(|source| OsmshrinkError::Pbf {
            path: input.to_path_buf(),
            source,
        })?;
        if let OsmObj::Way(way) = object {
            process_way(
                &way,
                compiled,
                node_index,
                required_way_ids,
                relation_way_geometries,
                sink,
                report,
            )?;
        }
    }
    Ok(())
}

fn collect_relation(
    relation: Relation,
    compiled: &CompiledFilter,
    candidates: &mut Vec<CandidateRelation>,
    required_way_ids: &mut HashSet<WayId>,
    report: &mut FilterReport,
) {
    let tags = normalize_tags(&relation.tags);
    if !compiled.matches_relation_tags(&tags) {
        return;
    }

    if !is_area_relation(&tags) {
        report.relations_skipped_non_area += 1;
        return;
    }

    let mut members = Vec::new();
    for member in relation.refs {
        let role = member.role.as_str();
        let Some(way_id) = member.member.way() else {
            report.relation_members_ignored_role += 1;
            continue;
        };
        let member_role = match role {
            "" | "outer" => RelationMemberRole::Outer,
            "inner" => RelationMemberRole::Inner,
            _ => {
                report.relation_members_ignored_role += 1;
                continue;
            }
        };
        required_way_ids.insert(way_id);
        members.push(RelationWayMember {
            way_id,
            role: member_role,
        });
    }

    candidates.push(CandidateRelation {
        id: relation.id,
        tags,
        members,
    });
}

fn process_way(
    way: &Way,
    compiled: &CompiledFilter,
    node_index: &dyn NodeIndex,
    required_way_ids: &HashSet<WayId>,
    relation_way_geometries: &mut HashMap<WayId, Vec<Coordinate>>,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()> {
    let required_by_relation = required_way_ids.contains(&way.id);
    let tags = compiled
        .includes_type(ElementType::Way)
        .then(|| normalize_tags(&way.tags));
    let eligible = tags
        .as_ref()
        .is_some_and(|tags| compiled.matches_way_tags(tags));
    if !eligible && !required_by_relation {
        return Ok(());
    }
    let coordinates = match coordinates_for_way(&way.nodes, node_index)? {
        Some(coordinates) => coordinates,
        None => {
            // Count only ways eligible for direct output, not already-rejected objects.
            if eligible {
                report.ways_skipped_missing_nodes += 1;
                warn!(
                    way_id = way.id.0,
                    "skipping way because one or more referenced nodes are missing"
                );
            }
            return Ok(());
        }
    };
    if eligible && compiled.matches_way_bbox(&coordinates) {
        sink.write_feature(Feature {
            id: way.id.0,
            kind: ElementKind::Way,
            tags: tags.expect("eligible ways have normalized tags"),
            geometry: way_geometry(&coordinates, way.is_closed(), compiled.geometry_mode),
        })?;
        report.objects_written += 1;
    }
    if required_by_relation {
        relation_way_geometries.insert(way.id, coordinates);
    }
    Ok(())
}

fn emit_relations(
    compiled: &CompiledFilter,
    candidates: &[CandidateRelation],
    relation_way_geometries: &HashMap<WayId, Vec<Coordinate>>,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()> {
    for candidate in candidates {
        let geometry = match assemble_relation(candidate, relation_way_geometries) {
            RelationAssemblyResult::Geometry(geometry) => geometry,
            RelationAssemblyResult::MissingMembers => {
                report.relations_skipped_missing_members += 1;
                continue;
            }
            RelationAssemblyResult::InvalidRings => {
                report.relations_skipped_invalid_rings += 1;
                continue;
            }
        };

        if !compiled.matches_relation_geometry(&geometry) {
            continue;
        }

        sink.write_feature(Feature {
            id: candidate.id.0,
            kind: ElementKind::Relation,
            tags: candidate.tags.clone(),
            geometry,
        })?;
        report.objects_written += 1;
    }
    Ok(())
}

fn coordinates_for_way(
    nodes: &[NodeId],
    node_index: &dyn NodeIndex,
) -> Result<Option<Vec<Coordinate>>> {
    node_index.resolve_coordinates(nodes)
}

fn normalize_tags(tags: &Tags) -> NormalizedTags {
    tags.iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[cfg(feature = "cli")]
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

trait FeatureSink {
    fn write_feature(&mut self, feature: Feature) -> Result<()>;
}

#[cfg(feature = "cli")]
impl FeatureSink for OutputWriter {
    fn write_feature(&mut self, feature: Feature) -> Result<()> {
        OutputWriter::write_feature(self, &feature)
    }
}

#[derive(Debug, Clone)]
struct VecFeatureSink {
    features: Vec<Feature>,
}

impl VecFeatureSink {
    fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }
}

impl FeatureSink for VecFeatureSink {
    fn write_feature(&mut self, feature: Feature) -> Result<()> {
        self.features.push(feature);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CandidateRelation {
    id: RelationId,
    tags: NormalizedTags,
    members: Vec<RelationWayMember>,
}

#[derive(Debug, Clone, Copy)]
struct RelationWayMember {
    way_id: WayId,
    role: RelationMemberRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationMemberRole {
    Outer,
    Inner,
}

#[derive(Debug)]
enum RelationAssemblyResult {
    Geometry(Geometry),
    MissingMembers,
    InvalidRings,
}

fn is_area_relation(tags: &NormalizedTags) -> bool {
    matches!(
        tags.get("type").map(String::as_str),
        Some("multipolygon" | "boundary")
    )
}

fn assemble_relation(
    candidate: &CandidateRelation,
    relation_way_geometries: &HashMap<WayId, Vec<Coordinate>>,
) -> RelationAssemblyResult {
    let mut outer_segments = Vec::new();
    let mut inner_segments = Vec::new();
    for member in &candidate.members {
        let Some(coordinates) = relation_way_geometries.get(&member.way_id) else {
            return RelationAssemblyResult::MissingMembers;
        };
        match member.role {
            RelationMemberRole::Outer => outer_segments.push(coordinates.clone()),
            RelationMemberRole::Inner => inner_segments.push(coordinates.clone()),
        }
    }

    let Some(mut outer_rings) = stitch_rings(outer_segments) else {
        return RelationAssemblyResult::InvalidRings;
    };
    let Some(mut inner_rings) = stitch_rings(inner_segments) else {
        return RelationAssemblyResult::InvalidRings;
    };

    if outer_rings.is_empty() {
        return RelationAssemblyResult::InvalidRings;
    }

    for ring in &mut outer_rings {
        normalize_ring_orientation(ring, true);
    }
    for ring in &mut inner_rings {
        normalize_ring_orientation(ring, false);
    }

    let mut polygons: Vec<Vec<Vec<Coordinate>>> =
        outer_rings.into_iter().map(|outer| vec![outer]).collect();

    for inner in inner_rings {
        let Some(point) = inner.first().copied() else {
            return RelationAssemblyResult::InvalidRings;
        };
        let Some((target_index, _)) = polygons
            .iter()
            .enumerate()
            .filter_map(|(index, polygon)| {
                let outer = &polygon[0];
                if point_in_ring(point, outer) {
                    Some((index, ring_area(outer).abs()))
                } else {
                    None
                }
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
        else {
            return RelationAssemblyResult::InvalidRings;
        };
        polygons[target_index].push(inner);
    }

    match polygon_or_multipolygon(polygons) {
        Some(geometry) => RelationAssemblyResult::Geometry(geometry),
        None => RelationAssemblyResult::InvalidRings,
    }
}

fn stitch_rings(mut segments: Vec<Vec<Coordinate>>) -> Option<Vec<Vec<Coordinate>>> {
    let mut rings = Vec::new();

    while !segments.is_empty() {
        let mut ring = segments.remove(0);
        if ring.len() < 2 {
            return None;
        }

        loop {
            if is_valid_closed_ring(&ring) {
                rings.push(ring);
                break;
            }

            let (index, action) = find_connecting_segment(&ring, &segments)?;
            let segment = segments.remove(index);
            apply_segment(&mut ring, segment, action);
        }
    }

    Some(rings)
}

fn is_valid_closed_ring(ring: &[Coordinate]) -> bool {
    ring.len() >= 4 && ring.first() == ring.last()
}

#[derive(Debug, Clone, Copy)]
enum StitchAction {
    AppendForward,
    AppendReverse,
    PrependForward,
    PrependReverse,
}

fn find_connecting_segment(
    ring: &[Coordinate],
    segments: &[Vec<Coordinate>],
) -> Option<(usize, StitchAction)> {
    let first = *ring.first()?;
    let last = *ring.last()?;
    segments.iter().enumerate().find_map(|(index, segment)| {
        let segment_first = *segment.first()?;
        let segment_last = *segment.last()?;
        if last == segment_first {
            Some((index, StitchAction::AppendForward))
        } else if last == segment_last {
            Some((index, StitchAction::AppendReverse))
        } else if first == segment_last {
            Some((index, StitchAction::PrependForward))
        } else if first == segment_first {
            Some((index, StitchAction::PrependReverse))
        } else {
            None
        }
    })
}

fn apply_segment(ring: &mut Vec<Coordinate>, mut segment: Vec<Coordinate>, action: StitchAction) {
    match action {
        StitchAction::AppendForward => ring.extend(segment.into_iter().skip(1)),
        StitchAction::AppendReverse => {
            segment.reverse();
            ring.extend(segment.into_iter().skip(1));
        }
        StitchAction::PrependForward => {
            segment.pop();
            segment.extend(ring.iter().copied());
            *ring = segment;
        }
        StitchAction::PrependReverse => {
            segment.reverse();
            segment.pop();
            segment.extend(ring.iter().copied());
            *ring = segment;
        }
    }
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
            && self.matches_node_tags(tags)
            && self.matches_node_bbox(coordinate)
    }

    fn matches_node_tags(&self, tags: &NormalizedTags) -> bool {
        self.types.contains(&ElementType::Node) && self.matches_tags(tags)
    }

    fn matches_node_bbox(&self, coordinate: Coordinate) -> bool {
        self.bbox
            .map(|bbox| bbox.contains(coordinate))
            .unwrap_or(true)
    }

    pub fn matches_way(&self, tags: &NormalizedTags, coordinates: &[Coordinate]) -> bool {
        self.matches_way_tags(tags) && self.matches_way_bbox(coordinates)
    }

    fn matches_way_tags(&self, tags: &NormalizedTags) -> bool {
        self.types.contains(&ElementType::Way) && self.matches_tags(tags)
    }

    fn matches_way_bbox(&self, coordinates: &[Coordinate]) -> bool {
        self.bbox
            .map(|bbox| bbox.intersects_any(coordinates))
            .unwrap_or(true)
    }

    fn matches_relation_tags(&self, tags: &NormalizedTags) -> bool {
        self.types.contains(&ElementType::Relation) && self.matches_tags(tags)
    }

    fn matches_relation_geometry(&self, geometry: &Geometry) -> bool {
        self.types.contains(&ElementType::Relation)
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
    #[cfg(feature = "cli")]
    use std::io::Write;

    use osmpbfreader::{Node, OsmId, Ref, RelationId};
    #[cfg(feature = "cli")]
    use osmpbfreader::{fileformat, osmformat};
    #[cfg(feature = "cli")]
    use protobuf::Message;

    use crate::index::{IndexOptions, MemoryNodeIndex};
    use crate::spec::{FilterRules, OutputSpec, ProcessingSpec};

    use super::*;

    fn tags(values: &[(&str, &str)]) -> NormalizedTags {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn osm_tags(values: &[(&str, &str)]) -> Tags {
        values
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    fn node(id: i64, lon: f64, lat: f64, tags: Tags) -> OsmObj {
        let stored = StoredCoordinate::from_degrees(lon, lat);
        OsmObj::Node(Node {
            id: NodeId(id),
            tags,
            decimicro_lat: stored.decimicro_lat,
            decimicro_lon: stored.decimicro_lon,
        })
    }

    fn way(id: i64, nodes: &[i64], tags: Tags) -> OsmObj {
        OsmObj::Way(Way {
            id: WayId(id),
            tags,
            nodes: nodes.iter().copied().map(NodeId).collect(),
        })
    }

    fn relation(id: i64, refs: Vec<(OsmId, &str)>, tags: Tags) -> OsmObj {
        OsmObj::Relation(Relation {
            id: RelationId(id),
            tags,
            refs: refs
                .into_iter()
                .map(|(member, role)| Ref {
                    member,
                    role: role.into(),
                })
                .collect(),
        })
    }

    fn spec(types: Vec<ElementType>, include: Option<IncludeRules>) -> FilterSpec {
        FilterSpec {
            source: None,
            filter: FilterRules {
                bbox: None,
                types: Some(types),
                include,
                exclude: Vec::new(),
            },
            processing: ProcessingSpec::default(),
            output: OutputSpec::default(),
        }
    }

    fn memory_index_options() -> IndexOptions {
        IndexOptions {
            mode: crate::spec::IndexMode::Memory,
            memory_node_limit: 10,
            disk_dir: None,
        }
    }

    fn filter_constructed_objects(
        objects: &[OsmObj],
        spec: FilterSpec,
    ) -> Result<(Vec<Feature>, FilterReport)> {
        let compiled = CompiledFilter::compile(&spec)?;
        let mut node_index = MemoryNodeIndex::new();
        let mut sink = VecFeatureSink::new();
        let mut report = FilterReport::new(PathBuf::from("test.ndjson"));

        for object in objects {
            if let OsmObj::Node(node) = object {
                process_node_for_test(
                    node.id,
                    StoredCoordinate::new(node.decimicro_lon, node.decimicro_lat),
                    &node.tags,
                    &compiled,
                    &mut node_index,
                    &mut sink,
                    &mut report,
                )?;
            }
        }

        let mut candidates = Vec::new();
        let mut required_way_ids = HashSet::new();
        if compiled.includes_type(ElementType::Relation) {
            for object in objects {
                if let OsmObj::Relation(relation) = object {
                    collect_relation(
                        relation.clone(),
                        &compiled,
                        &mut candidates,
                        &mut required_way_ids,
                        &mut report,
                    );
                }
            }
        }

        let mut relation_way_geometries = HashMap::new();
        if compiled.includes_type(ElementType::Way) || !required_way_ids.is_empty() {
            for object in objects {
                if let OsmObj::Way(way) = object {
                    process_way(
                        way,
                        &compiled,
                        &node_index,
                        &required_way_ids,
                        &mut relation_way_geometries,
                        &mut sink,
                        &mut report,
                    )?;
                }
            }
        }

        emit_relations(
            &compiled,
            &candidates,
            &relation_way_geometries,
            &mut sink,
            &mut report,
        )?;

        Ok((sink.features, report))
    }

    #[cfg(feature = "cli")]
    fn synthetic_pbf_bytes() -> Vec<u8> {
        let mut string_table = osmformat::StringTable::new();
        for value in [
            "",
            "amenity",
            "school",
            "highway",
            "residential",
            "name",
            "Synthetic Road",
        ] {
            string_table.mut_s().push(value.as_bytes().to_vec());
        }

        let mut dense_nodes = osmformat::DenseNodes::new();
        let mut previous_id = 0_i64;
        let mut previous_lat = 0_i64;
        let mut previous_lon = 0_i64;
        for (id, lat, lon) in [
            (1_i64, 480_000_000_i64, 80_000_000_i64),
            (2, 480_001_000, 80_001_000),
            (3, 480_002_000, 80_002_000),
        ] {
            dense_nodes.id.push(id - previous_id);
            dense_nodes.lat.push(lat - previous_lat);
            dense_nodes.lon.push(lon - previous_lon);
            previous_id = id;
            previous_lat = lat;
            previous_lon = lon;
        }
        dense_nodes.keys_vals = vec![1, 2, 0, 0, 0];

        let mut way = osmformat::Way::new();
        way.set_id(10);
        way.keys = vec![3, 5];
        way.vals = vec![4, 6];
        way.refs = vec![1, 1, 1];

        let mut group = osmformat::PrimitiveGroup::new();
        group.set_dense(dense_nodes);
        group.mut_ways().push(way);

        let mut block = osmformat::PrimitiveBlock::new();
        block.set_stringtable(string_table);
        block.mut_primitivegroup().push(group);

        let mut bytes = Vec::new();
        write_raw_blob(&mut bytes, "OSMData", block.write_to_bytes().unwrap());
        bytes
    }

    #[cfg(feature = "cli")]
    fn write_raw_blob(writer: &mut Vec<u8>, field_type: &str, payload: Vec<u8>) {
        let mut blob = fileformat::Blob::new();
        blob.set_raw(payload);
        let blob_bytes = blob.write_to_bytes().unwrap();

        let mut header = fileformat::BlobHeader::new();
        header.set_field_type(field_type.to_owned());
        header.set_datasize(blob_bytes.len().try_into().unwrap());
        let header_bytes = header.write_to_bytes().unwrap();

        let header_len: u32 = header_bytes.len().try_into().unwrap();
        writer.write_all(&header_len.to_be_bytes()).unwrap();
        writer.write_all(&header_bytes).unwrap();
        writer.write_all(&blob_bytes).unwrap();
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
    fn collect_pbf_bytes_reports_corrupt_input_as_pbf_error() {
        let error = collect_pbf_bytes(CollectBytesOptions {
            input: b"not an osm pbf file",
            spec: spec(vec![ElementType::Node], None),
            index_options: memory_index_options(),
        })
        .unwrap_err();

        assert!(matches!(error, OsmshrinkError::Pbf { .. }));
        assert!(error.to_string().contains("<memory>"));
    }

    #[cfg(feature = "cli")]
    #[test]
    fn collect_pbf_bytes_matches_path_collection() {
        let bytes = synthetic_pbf_bytes();
        let file = tempfile::NamedTempFile::with_suffix(".osm.pbf").unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let spec = spec(vec![ElementType::Node, ElementType::Way], None);
        let index_options = memory_index_options();

        let from_path = collect_pbf(CollectRunOptions {
            input: file.path().to_path_buf(),
            spec: spec.clone(),
            index_options: index_options.clone(),
        })
        .unwrap();
        let from_bytes = collect_pbf_bytes(CollectBytesOptions {
            input: &bytes,
            spec,
            index_options,
        })
        .unwrap();

        let path_keys: Vec<_> = from_path
            .features
            .iter()
            .map(|feature| (feature.kind, feature.id))
            .collect();
        let byte_keys: Vec<_> = from_bytes
            .features
            .iter()
            .map(|feature| (feature.kind, feature.id))
            .collect();
        assert_eq!(byte_keys, path_keys);
        assert_eq!(
            from_bytes.report.objects_collected,
            from_path.report.objects_collected
        );
        assert_eq!(from_bytes.report.index_backend, IndexBackend::Memory);
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
            processing: ProcessingSpec::default(),
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
            processing: ProcessingSpec::default(),
            output: OutputSpec::default(),
        };
        let filter = CompiledFilter::compile(&spec).unwrap();

        assert!(filter.matches_way(
            &tags(&[]),
            &[Coordinate::new(8.7, 48.9), Coordinate::new(10.0, 49.0)]
        ));
        assert!(!filter.matches_way(&tags(&[]), &[Coordinate::new(10.0, 49.0)]));
    }

    #[test]
    fn relation_multipolygon_is_emitted_from_constructed_objects() {
        let objects = vec![
            node(1, 0.0, 0.0, osm_tags(&[])),
            node(2, 1.0, 0.0, osm_tags(&[])),
            node(3, 1.0, 1.0, osm_tags(&[])),
            node(4, 0.0, 1.0, osm_tags(&[])),
            way(10, &[1, 2, 3], osm_tags(&[])),
            way(11, &[3, 4, 1], osm_tags(&[])),
            relation(
                20,
                vec![
                    (OsmId::Way(WayId(10)), "outer"),
                    (OsmId::Way(WayId(11)), ""),
                ],
                osm_tags(&[("type", "multipolygon"), ("name", "Area")]),
            ),
        ];

        let (features, report) =
            filter_constructed_objects(&objects, spec(vec![ElementType::Relation], None)).unwrap();

        assert_eq!(report.objects_written, 1);
        assert_eq!(features[0].kind, ElementKind::Relation);
        assert!(matches!(features[0].geometry, Geometry::Polygon { .. }));
    }

    #[test]
    fn relation_stitches_reversed_way_fragments() {
        let objects = vec![
            node(1, 0.0, 0.0, osm_tags(&[])),
            node(2, 1.0, 0.0, osm_tags(&[])),
            node(3, 1.0, 1.0, osm_tags(&[])),
            node(4, 0.0, 1.0, osm_tags(&[])),
            way(10, &[1, 2, 3], osm_tags(&[])),
            way(11, &[1, 4, 3], osm_tags(&[])),
            relation(
                20,
                vec![
                    (OsmId::Way(WayId(10)), "outer"),
                    (OsmId::Way(WayId(11)), "outer"),
                ],
                osm_tags(&[("type", "multipolygon")]),
            ),
        ];

        let (features, report) =
            filter_constructed_objects(&objects, spec(vec![ElementType::Relation], None)).unwrap();

        assert_eq!(report.objects_written, 1);
        assert!(matches!(features[0].geometry, Geometry::Polygon { .. }));
    }

    #[test]
    fn relation_with_multiple_outers_emits_multipolygon() {
        let objects = vec![
            node(1, 0.0, 0.0, osm_tags(&[])),
            node(2, 1.0, 0.0, osm_tags(&[])),
            node(3, 1.0, 1.0, osm_tags(&[])),
            node(4, 0.0, 1.0, osm_tags(&[])),
            node(5, 3.0, 3.0, osm_tags(&[])),
            node(6, 4.0, 3.0, osm_tags(&[])),
            node(7, 4.0, 4.0, osm_tags(&[])),
            node(8, 3.0, 4.0, osm_tags(&[])),
            way(10, &[1, 2, 3, 4, 1], osm_tags(&[])),
            way(11, &[5, 6, 7, 8, 5], osm_tags(&[])),
            relation(
                20,
                vec![
                    (OsmId::Way(WayId(10)), "outer"),
                    (OsmId::Way(WayId(11)), "outer"),
                ],
                osm_tags(&[("type", "multipolygon")]),
            ),
        ];

        let (features, _) =
            filter_constructed_objects(&objects, spec(vec![ElementType::Relation], None)).unwrap();

        assert!(matches!(
            features[0].geometry,
            Geometry::MultiPolygon { .. }
        ));
    }

    #[test]
    fn relation_with_open_ring_is_skipped() {
        let objects = vec![
            node(1, 0.0, 0.0, osm_tags(&[])),
            node(2, 1.0, 0.0, osm_tags(&[])),
            node(3, 1.0, 1.0, osm_tags(&[])),
            way(10, &[1, 2, 3], osm_tags(&[])),
            relation(
                20,
                vec![(OsmId::Way(WayId(10)), "outer")],
                osm_tags(&[("type", "multipolygon")]),
            ),
        ];

        let (features, report) =
            filter_constructed_objects(&objects, spec(vec![ElementType::Relation], None)).unwrap();

        assert!(features.is_empty());
        assert_eq!(report.relations_skipped_invalid_rings, 1);
    }

    #[test]
    fn relation_with_hole_assigns_inner_ring() {
        let objects = vec![
            node(1, 0.0, 0.0, osm_tags(&[])),
            node(2, 4.0, 0.0, osm_tags(&[])),
            node(3, 4.0, 4.0, osm_tags(&[])),
            node(4, 0.0, 4.0, osm_tags(&[])),
            node(5, 1.0, 1.0, osm_tags(&[])),
            node(6, 2.0, 1.0, osm_tags(&[])),
            node(7, 2.0, 2.0, osm_tags(&[])),
            node(8, 1.0, 2.0, osm_tags(&[])),
            way(10, &[1, 2, 3, 4, 1], osm_tags(&[])),
            way(11, &[5, 6, 7, 8, 5], osm_tags(&[])),
            relation(
                20,
                vec![
                    (OsmId::Way(WayId(10)), "outer"),
                    (OsmId::Way(WayId(11)), "inner"),
                ],
                osm_tags(&[("type", "multipolygon")]),
            ),
        ];

        let (features, _) =
            filter_constructed_objects(&objects, spec(vec![ElementType::Relation], None)).unwrap();

        let Geometry::Polygon { coordinates } = &features[0].geometry else {
            panic!("expected polygon");
        };
        assert_eq!(coordinates.len(), 2);
    }

    #[test]
    fn non_area_relation_is_counted_and_skipped() {
        let objects = vec![relation(
            20,
            vec![],
            osm_tags(&[("type", "route"), ("route", "bus")]),
        )];

        let (features, report) =
            filter_constructed_objects(&objects, spec(vec![ElementType::Relation], None)).unwrap();

        assert!(features.is_empty());
        assert_eq!(report.relations_skipped_non_area, 1);
    }

    #[test]
    fn node_way_regression_still_ignores_relations_when_not_requested() {
        let objects = vec![
            node(1, 0.0, 0.0, osm_tags(&[("amenity", "school")])),
            node(2, 1.0, 0.0, osm_tags(&[])),
            way(10, &[1, 2], osm_tags(&[("highway", "primary")])),
            relation(20, vec![], osm_tags(&[("type", "multipolygon")])),
        ];

        let (features, report) = filter_constructed_objects(
            &objects,
            spec(vec![ElementType::Node, ElementType::Way], None),
        )
        .unwrap();

        assert_eq!(report.objects_written, 3);
        assert!(
            features
                .iter()
                .all(|feature| feature.kind != ElementKind::Relation)
        );
    }
}

#[cfg(test)]
#[path = "filter_regressions.rs"]
mod audit_regressions;
