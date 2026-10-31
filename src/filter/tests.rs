use std::cell::Cell;
use std::collections::{HashMap, HashSet};
#[cfg(feature = "cli")]
use std::io::Write;
use std::path::PathBuf;

use osmpbfreader::{Node, NodeId, OsmId, OsmObj, Ref, Relation, RelationId, Tags, Way, WayId};
#[cfg(feature = "cli")]
use osmpbfreader::{fileformat, osmformat};
#[cfg(feature = "cli")]
use protobuf::Message;

use crate::error::{OsmshrinkError, Result};
use crate::geometry::{Coordinate, Geometry};
use crate::index::{IndexBackend, IndexOptions, MemoryNodeIndex, NodeIndex, StoredCoordinate};
use crate::model::{ElementKind, Feature, Tags as NormalizedTags};
use crate::spec::{
    ElementType, FilterRules, FilterSpec, IncludeRules, OutputSpec, ProcessingSpec, TagCondition,
};

use super::compiled::{CompiledCondition, CompiledFilter};
#[cfg(feature = "cli")]
use super::options::CollectRunOptions;
use super::options::{CollectBytesOptions, FilterReport};
#[cfg(feature = "cli")]
use super::pipeline::collect_pbf;
use super::pipeline::collect_pbf_bytes;
use super::pipeline::{
    collect_relation, emit_relations, normalize_tags, process_node_for_test, process_way,
};
use super::sink::VecFeatureSink;

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

struct CountingNodeIndex {
    inner: MemoryNodeIndex,
    single_gets: Cell<usize>,
    batch_gets: Cell<usize>,
}

impl CountingNodeIndex {
    fn with_nodes(entries: &[(NodeId, StoredCoordinate)]) -> Self {
        let mut inner = MemoryNodeIndex::new();
        inner.insert_batch(entries).unwrap();
        Self {
            inner,
            single_gets: Cell::new(0),
            batch_gets: Cell::new(0),
        }
    }
}

impl NodeIndex for CountingNodeIndex {
    fn insert(&mut self, node_id: NodeId, coordinate: StoredCoordinate) -> Result<()> {
        self.inner.insert(node_id, coordinate)
    }

    fn insert_batch(&mut self, entries: &[(NodeId, StoredCoordinate)]) -> Result<()> {
        self.inner.insert_batch(entries)
    }

    fn get(&self, node_id: NodeId) -> Result<Option<StoredCoordinate>> {
        self.single_gets.set(self.single_gets.get() + 1);
        self.inner.get(node_id)
    }

    fn get_batch(&self, node_ids: &[NodeId]) -> Result<Option<Vec<StoredCoordinate>>> {
        self.batch_gets.set(self.batch_gets.get() + 1);
        self.inner.get_batch(node_ids)
    }

    fn backend(&self) -> IndexBackend {
        self.inner.backend()
    }

    fn len(&self) -> usize {
        self.inner.len()
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
        string_table.s.push(value.as_bytes().to_vec());
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
    group.dense = protobuf::MessageField::some(dense_nodes);
    group.ways.push(way);

    let mut block = osmformat::PrimitiveBlock::new();
    block.stringtable = protobuf::MessageField::some(string_table);
    block.primitivegroup.push(group);

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
    header.set_type(field_type.to_owned());
    header.set_datasize(blob_bytes.len().try_into().unwrap());
    let header_bytes = header.write_to_bytes().unwrap();

    let header_len: u32 = header_bytes.len().try_into().unwrap();
    writer.write_all(&header_len.to_be_bytes()).unwrap();
    writer.write_all(&header_bytes).unwrap();
    writer.write_all(&blob_bytes).unwrap();
}

#[test]
fn rejected_way_skips_geometry_lookup_entirely() {
    let include = Some(IncludeRules {
        any: vec![TagCondition {
            key: "highway".to_owned(),
            exists: None,
            value: Some("residential".to_owned()),
            values: None,
            regex: None,
            negate: false,
        }],
        all: Vec::new(),
    });
    let compiled = CompiledFilter::compile(&spec(vec![ElementType::Way], include)).unwrap();
    let index = CountingNodeIndex::with_nodes(&[
        (NodeId(1), StoredCoordinate::from_degrees(8.7, 48.9)),
        (NodeId(2), StoredCoordinate::from_degrees(8.8, 49.0)),
    ]);
    let way = Way {
        id: WayId(7),
        tags: osm_tags(&[("highway", "service")]),
        nodes: vec![NodeId(1), NodeId(2)],
    };
    let mut sink = VecFeatureSink::new();
    let mut report = FilterReport::new(PathBuf::from("test.ndjson"));

    process_way(
        &way,
        &compiled,
        &index,
        &HashSet::new(),
        &mut HashMap::new(),
        &mut sink,
        &mut report,
    )
    .unwrap();

    assert_eq!(index.single_gets.get(), 0);
    assert_eq!(index.batch_gets.get(), 0);
    assert!(sink.features.is_empty());
}

#[test]
fn required_relation_way_uses_one_batch_lookup_even_when_direct_filter_rejects_it() {
    let include = Some(IncludeRules {
        any: vec![TagCondition {
            key: "highway".to_owned(),
            exists: None,
            value: Some("residential".to_owned()),
            values: None,
            regex: None,
            negate: false,
        }],
        all: Vec::new(),
    });
    let compiled = CompiledFilter::compile(&spec(
        vec![ElementType::Way, ElementType::Relation],
        include,
    ))
    .unwrap();
    let index = CountingNodeIndex::with_nodes(&[
        (NodeId(1), StoredCoordinate::from_degrees(8.7, 48.9)),
        (NodeId(2), StoredCoordinate::from_degrees(8.8, 49.0)),
    ]);
    let way = Way {
        id: WayId(7),
        tags: osm_tags(&[("highway", "service")]),
        nodes: vec![NodeId(1), NodeId(2)],
    };
    let required = HashSet::from([WayId(7)]);
    let mut geometries = HashMap::new();
    let mut sink = VecFeatureSink::new();
    let mut report = FilterReport::new(PathBuf::from("test.ndjson"));

    process_way(
        &way,
        &compiled,
        &index,
        &required,
        &mut geometries,
        &mut sink,
        &mut report,
    )
    .unwrap();

    assert_eq!(index.single_gets.get(), 0);
    assert_eq!(index.batch_gets.get(), 1);
    assert_eq!(geometries.get(&WayId(7)).map(Vec::len), Some(2));
    assert!(sink.features.is_empty());
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
fn osm_tag_matching_matches_normalized_tag_matching() {
    let conditions = [
        TagCondition {
            key: "amenity".to_owned(),
            exists: Some(true),
            value: None,
            values: None,
            regex: None,
            negate: false,
        },
        TagCondition {
            key: "amenity".to_owned(),
            exists: None,
            value: Some("school".to_owned()),
            values: None,
            regex: None,
            negate: false,
        },
        TagCondition {
            key: "amenity".to_owned(),
            exists: None,
            value: None,
            values: Some(vec!["school".to_owned(), "hospital".to_owned()]),
            regex: None,
            negate: false,
        },
        TagCondition {
            key: "name".to_owned(),
            exists: None,
            value: None,
            values: None,
            regex: Some("^Primary".to_owned()),
            negate: true,
        },
    ];
    let osm = osm_tags(&[("amenity", "school"), ("name", "Primary School")]);
    let normalized = normalize_tags(&osm);

    for condition in conditions {
        let compiled = CompiledCondition::compile(&condition).unwrap();
        assert_eq!(compiled.matches_osm(&osm), compiled.matches(&normalized));
    }
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
