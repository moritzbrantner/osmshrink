//! End-to-end PBF parsing, indexing, selective way/member resolution, and output.
//! Deterministic fixture and output counts; no wall-clock pass/fail threshold.
//! This harness also compiles at audited revision 81674b14 for paired evidence.
use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use osmpbfreader::{fileformat, osmformat};
use osmshrink::{
    FilterSpec,
    filter::{FilterRunOptions, filter_pbf},
    index::IndexOptions,
    spec::IndexMode,
};
use protobuf::Message;
use serde_json::json;
use std::{fs::File, io::Write, path::Path, time::Duration};
const WAYS: usize = 2000;
const NODES_PER_WAY: usize = 16;

fn fixture(path: &Path) {
    let mut strings = osmformat::StringTable::new();
    for s in [
        "",
        "highway",
        "primary",
        "residential",
        "type",
        "multipolygon",
        "landuse",
        "forest",
        "outer",
    ] {
        strings.mut_s().push(s.as_bytes().to_vec());
    }
    let mut group = osmformat::PrimitiveGroup::new();
    for w in 0..WAYS {
        // Clockwise perimeter of a small rectangle, with intermediate distinct points.
        for j in 0..NODES_PER_WAY {
            let mut node = osmformat::Node::new();
            node.set_id((w * NODES_PER_WAY + j + 1) as i64);
            let (x, y) = match j / 4 {
                0 => (j % 4, 0),
                1 => (4, j % 4),
                2 => (4 - j % 4, 4),
                _ => (0, 4 - j % 4),
            };
            node.set_lon(80_000_000 + (w as i64 * 100) + x as i64 * 10);
            node.set_lat(480_000_000 + y as i64 * 10);
            group.mut_nodes().push(node);
        }
        let mut way = osmformat::Way::new();
        way.set_id(w as i64 + 1);
        way.keys = vec![1];
        way.vals = vec![if w % 100 == 0 { 2 } else { 3 }];
        way.refs = vec![(w * NODES_PER_WAY + 1) as i64];
        way.refs.extend(std::iter::repeat_n(1, NODES_PER_WAY - 1));
        way.refs.push(-((NODES_PER_WAY - 1) as i64));
        group.mut_ways().push(way);
        if w % 100 == 0 {
            let mut relation = osmformat::Relation::new();
            relation.set_id(w as i64 + 1);
            relation.keys = vec![4, 6];
            relation.vals = vec![5, 7];
            relation.roles_sid = vec![8];
            relation.memids = vec![w as i64 + 1];
            relation.types = vec![osmformat::Relation_MemberType::WAY];
            group.mut_relations().push(relation);
        }
    }
    let mut block = osmformat::PrimitiveBlock::new();
    block.set_stringtable(strings);
    block.mut_primitivegroup().push(group);
    let mut blob = fileformat::Blob::new();
    blob.set_raw(block.write_to_bytes().unwrap());
    let body = blob.write_to_bytes().unwrap();
    let mut header = fileformat::BlobHeader::new();
    header.set_field_type("OSMData".into());
    header.set_datasize(body.len() as i32);
    let header = header.write_to_bytes().unwrap();
    let mut f = File::create(path).unwrap();
    f.write_all(&(header.len() as u32).to_be_bytes()).unwrap();
    f.write_all(&header).unwrap();
    f.write_all(&body).unwrap();
}
fn bench(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("selective.pbf");
    fixture(&input);
    let mut group = c.benchmark_group("selective_filter");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_secs(1));
    group.throughput(Throughput::Elements(
        (WAYS * (NODES_PER_WAY + 1) + WAYS / 100) as u64,
    ));
    for (name, rules, expected) in [
        ("all", json!({"types":["way"]}), WAYS),
        (
            "one_percent",
            json!({"types":["way"],"include":{"any":[{"key":"highway","value":"primary"}]}}),
            WAYS / 100,
        ),
        (
            "rejected",
            json!({"types":["way"],"include":{"any":[{"key":"highway","value":"never"}]}}),
            0,
        ),
        (
            "relation_only",
            json!({"types":["relation"],"include":{"any":[{"key":"landuse","value":"forest"}]}}),
            WAYS / 100,
        ),
    ] {
        let spec: FilterSpec = serde_json::from_value(json!({"filter":rules})).unwrap();
        for (backend, mode) in [("memory", IndexMode::Memory), ("disk", IndexMode::Disk)] {
            let options = FilterRunOptions {
                input: input.clone(),
                output: dir.path().join("result.ndjson"),
                spec: spec.clone(),
                format_override: None,
                index_options: IndexOptions {
                    mode,
                    memory_node_limit: usize::MAX,
                    disk_dir: None,
                },
            };
            let report = filter_pbf(options.clone()).unwrap();
            assert_eq!(report.objects_written, expected as u64);
            group.bench_function(format!("{name}/{backend}"), |b| {
                b.iter(|| {
                    let report = filter_pbf(black_box(options.clone())).unwrap();
                    assert_eq!(report.objects_written, expected as u64);
                    black_box(report);
                })
            });
        }
    }
    group.finish();
}
criterion_group!(benches, bench);
criterion_main!(benches);
