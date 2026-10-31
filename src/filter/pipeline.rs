use std::collections::{HashMap, HashSet};
#[cfg(feature = "cli")]
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use osmpbfreader::{NodeId, OsmObj, OsmPbfReader, Relation, Tags, Way, WayId};
use tracing::warn;

use crate::error::{OsmshrinkError, Result};
use crate::geometry::{Coordinate, point, way_geometry};
use crate::index::{AutoNodeIndex, IndexOptions, NodeIndex, StoredCoordinate};
use crate::model::{ElementKind, Feature, Tags as NormalizedTags};
#[cfg(feature = "cli")]
use crate::output::OutputWriter;
use crate::spec::ElementType;
#[cfg(feature = "cli")]
use crate::spec::FilterSpec;
#[cfg(feature = "cli")]
use crate::spec::OutputFormat;

use super::compiled::CompiledFilter;
use super::options::{CollectBytesOptions, CollectedFeatures, FilterReport};
#[cfg(feature = "cli")]
use super::options::{CollectRunOptions, FilterRunOptions};
use super::relation::{
    CandidateRelation, RelationAssemblyResult, RelationMemberRole, RelationWayMember,
    assemble_relation, is_area_relation_osm,
};
use super::sink::{FeatureSink, VecFeatureSink};

const NODE_INDEX_BATCH_SIZE: usize = 16_384;

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
pub(super) fn process_node_for_test(
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

    if context.compiled.matches_node_osm_tags(tags) {
        context.node_features.push(Feature {
            id: node_id.0,
            kind: ElementKind::Node,
            tags: normalize_tags(tags),
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
    // Only ways are needed here, and the first pass already decoded every other object.
    let mut reader = OsmPbfReader::new(reader);
    for way in reader.iter_ways() {
        let way = way.map_err(|source| OsmshrinkError::Pbf {
            path: input.to_path_buf(),
            source,
        })?;
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
    Ok(())
}

pub(super) fn collect_relation(
    relation: Relation,
    compiled: &CompiledFilter,
    candidates: &mut Vec<CandidateRelation>,
    required_way_ids: &mut HashSet<WayId>,
    report: &mut FilterReport,
) {
    if !compiled.matches_relation_osm_tags(&relation.tags) {
        return;
    }

    if !is_area_relation_osm(&relation.tags) {
        report.relations_skipped_non_area += 1;
        return;
    }

    let tags = normalize_tags(&relation.tags);
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

pub(super) fn process_way(
    way: &Way,
    compiled: &CompiledFilter,
    node_index: &dyn NodeIndex,
    required_way_ids: &HashSet<WayId>,
    relation_way_geometries: &mut HashMap<WayId, Vec<Coordinate>>,
    sink: &mut dyn FeatureSink,
    report: &mut FilterReport,
) -> Result<()> {
    let required_for_relation = required_way_ids.contains(&way.id);
    let matches_way_tags = compiled.matches_way_osm_tags(&way.tags);

    if !matches_way_tags && !required_for_relation {
        return Ok(());
    }

    let mut coordinates = match coordinates_for_way(&way.nodes, node_index)? {
        Some(coordinates) => coordinates,
        None => {
            if matches_way_tags {
                report.ways_skipped_missing_nodes += 1;
                warn!(
                    way_id = way.id.0,
                    "skipping way because one or more referenced nodes are missing"
                );
            }
            return Ok(());
        }
    };

    if matches_way_tags && compiled.matches_way_geometry(&coordinates) {
        sink.write_feature(Feature {
            id: way.id.0,
            kind: ElementKind::Way,
            tags: normalize_tags(&way.tags),
            geometry: way_geometry(&coordinates, way.is_closed(), compiled.geometry_mode),
        })?;
        report.objects_written += 1;
    }

    if required_for_relation {
        relation_way_geometries.insert(way.id, std::mem::take(&mut coordinates));
    }

    Ok(())
}

pub(super) fn emit_relations(
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
    Ok(node_index.get_batch(nodes)?.map(|coordinates| {
        coordinates
            .into_iter()
            .map(StoredCoordinate::to_coordinate)
            .collect()
    }))
}

pub(super) fn normalize_tags(tags: &Tags) -> NormalizedTags {
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
