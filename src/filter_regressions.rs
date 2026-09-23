//! Operation-count ratchets exercise the production way-processing path, not a copy.
use super::*;
use serde_json::json;
use std::cell::Cell;

#[derive(Default)]
struct CountingIndex {
    gets: Cell<usize>,
    missing: Option<NodeId>,
    fail: bool,
}
impl NodeIndex for CountingIndex {
    fn insert(&mut self, _: NodeId, _: StoredCoordinate) -> Result<()> {
        unreachable!()
    }
    fn get(&self, id: NodeId) -> Result<Option<StoredCoordinate>> {
        self.gets.set(self.gets.get() + 1);
        if self.fail {
            return Err(OsmshrinkError::UnsupportedRuntime(
                "injected index failure".into(),
            ));
        }
        Ok((self.missing != Some(id)).then(|| StoredCoordinate::new(id.0 as i32, 0)))
    }
    fn backend(&self) -> IndexBackend {
        IndexBackend::Memory
    }
    fn len(&self) -> usize {
        100
    }
}
fn compiled(types: &[&str], bbox: Option<[f64; 4]>) -> CompiledFilter {
    let spec = serde_json::from_value(json!({"filter": {
        "types": types, "bbox": bbox,
        "include": {"any": [{"key": "highway", "value": "primary"}]}
    }}))
    .unwrap();
    CompiledFilter::compile(&spec).unwrap()
}
fn way(value: &str) -> Way {
    Way {
        id: WayId(7),
        tags: [("highway".into(), value.into())].into_iter().collect(),
        nodes: vec![NodeId(1), NodeId(2), NodeId(3), NodeId(1)],
    }
}
fn run(
    way: &Way,
    compiled: &CompiledFilter,
    index: &CountingIndex,
    required: bool,
) -> Result<(
    VecFeatureSink,
    FilterReport,
    HashMap<WayId, Vec<Coordinate>>,
)> {
    let mut sink = VecFeatureSink::new();
    let mut report = FilterReport::new(PathBuf::from("ratchet.ndjson"));
    let required = if required {
        HashSet::from([way.id])
    } else {
        HashSet::new()
    };
    let mut geometries = HashMap::new();
    process_way(
        way,
        compiled,
        index,
        &required,
        &mut geometries,
        &mut sink,
        &mut report,
    )?;
    Ok((sink, report, geometries))
}
#[test]
fn rejected_nonmembers_do_zero_node_lookups_at_every_way_size() {
    for size in [0, 4, 64, 4096] {
        let mut way = way("residential");
        way.nodes = (0..size).map(NodeId).collect();
        let index = CountingIndex {
            fail: true,
            ..Default::default()
        };
        let (sink, report, geometries) =
            run(&way, &compiled(&["way"], None), &index, false).unwrap();
        assert_eq!(index.gets.get(), 0, "rejected way size {size}");
        assert!(sink.features.is_empty() && geometries.is_empty());
        assert_eq!(
            report.ways_skipped_missing_nodes, 0,
            "rejected ways are not missing-node candidates"
        );
    }
}
#[test]
fn relation_only_nonmembers_do_zero_node_lookups() {
    let index = CountingIndex {
        fail: true,
        ..Default::default()
    };
    run(
        &way("primary"),
        &compiled(&["relation"], None),
        &index,
        false,
    )
    .unwrap();
    assert_eq!(index.gets.get(), 0);
}
#[test]
fn required_members_survive_tag_rejection_without_becoming_direct_output() {
    for types in [vec!["way", "relation"], vec!["relation"]] {
        let index = CountingIndex::default();
        let (sink, _, geometries) =
            run(&way("residential"), &compiled(&types, None), &index, true).unwrap();
        assert_eq!(index.gets.get(), 4);
        assert!(sink.features.is_empty());
        let points = &geometries[&WayId(7)];
        assert_eq!(points.len(), 4);
        assert_eq!(
            points[0], points[3],
            "ring closure and duplicate references preserved"
        );
    }
}
#[test]
fn required_geometry_survives_direct_bbox_rejection() {
    let index = CountingIndex::default();
    let (sink, _, geometries) = run(
        &way("primary"),
        &compiled(&["way", "relation"], Some([10., 10., 20., 20.])),
        &index,
        true,
    )
    .unwrap();
    assert!(sink.features.is_empty());
    assert_eq!(geometries[&WayId(7)].len(), 4);
    assert_eq!(
        index.gets.get(),
        4,
        "one resolution, not separate member/output lookups"
    );
}
#[test]
fn matching_output_and_member_share_one_resolution() {
    let index = CountingIndex::default();
    let (sink, report, geometries) = run(
        &way("primary"),
        &compiled(&["way", "relation"], None),
        &index,
        true,
    )
    .unwrap();
    assert_eq!(sink.features.len(), 1);
    assert_eq!(report.objects_written, 1);
    assert_eq!(geometries.len(), 1);
    assert_eq!(index.gets.get(), 4);
}
#[test]
fn missing_nodes_abort_without_partial_geometry_and_errors_propagate() {
    let index = CountingIndex {
        missing: Some(NodeId(2)),
        ..Default::default()
    };
    let (sink, report, geometries) =
        run(&way("primary"), &compiled(&["way"], None), &index, true).unwrap();
    assert!(sink.features.is_empty() && geometries.is_empty());
    assert_eq!(report.ways_skipped_missing_nodes, 1);
    assert_eq!(index.gets.get(), 2, "stop at first missing reference");
    let index = CountingIndex {
        fail: true,
        ..Default::default()
    };
    assert!(run(&way("primary"), &compiled(&["way"], None), &index, false).is_err());
}
