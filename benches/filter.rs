use std::error::Error;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use osmpbfreader::{fileformat, osmformat};
use osmshrink::filter::{FilterRunOptions, filter_pbf};
use osmshrink::index::IndexOptions;
use osmshrink::spec::{
    ElementType, FilterRules, FilterSpec, IncludeRules, IndexMode, ProcessingSpec, TagCondition,
};
use protobuf::Message;
use tempfile::tempdir;

const WAY_COUNT: usize = 5_000;
const NODES_PER_WAY: usize = 4;
const NODE_COUNT: usize = WAY_COUNT * NODES_PER_WAY;
const INPUT_ELEMENTS: u64 = (NODE_COUNT + WAY_COUNT) as u64;

fn bench_filter_pbf(c: &mut Criterion) {
    let fixture_dir = tempdir().expect("create benchmark fixture directory");
    let input = fixture_dir.path().join("synthetic.osm.pbf");
    write_synthetic_extract(&input).expect("write synthetic PBF fixture");

    let spec = way_filter_spec("residential");
    let rejected_spec = way_filter_spec("trunk");
    let index_options = IndexOptions {
        mode: IndexMode::Memory,
        memory_node_limit: NODE_COUNT + 1,
        disk_dir: None,
    };
    let output = fixture_dir.path().join("ways.ndjson");
    let rejected_output = fixture_dir.path().join("rejected.ndjson");

    let mut group = c.benchmark_group("filter_pbf");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(INPUT_ELEMENTS));

    group.bench_function("ways_memory_index", |b| {
        b.iter_batched(
            || FilterRunOptions {
                input: input.clone(),
                output: output.clone(),
                spec: spec.clone(),
                format_override: None,
                index_options: index_options.clone(),
            },
            |options| {
                let report = filter_pbf(options).expect("benchmark filter run succeeds");
                assert_eq!(report.objects_written, WAY_COUNT as u64);
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("ways_rejected_before_geometry", |b| {
        b.iter_batched(
            || FilterRunOptions {
                input: input.clone(),
                output: rejected_output.clone(),
                spec: rejected_spec.clone(),
                format_override: None,
                index_options: index_options.clone(),
            },
            |options| {
                let report = filter_pbf(options).expect("benchmark filter run succeeds");
                assert_eq!(report.objects_written, 0);
                black_box(report);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn way_filter_spec(highway: &str) -> FilterSpec {
    FilterSpec {
        source: None,
        filter: FilterRules {
            bbox: None,
            types: Some(vec![ElementType::Way]),
            include: Some(IncludeRules {
                any: vec![TagCondition {
                    key: "highway".to_owned(),
                    exists: None,
                    value: Some(highway.to_owned()),
                    values: None,
                    regex: None,
                    negate: false,
                }],
                all: Vec::new(),
            }),
            exclude: Vec::new(),
        },
        processing: ProcessingSpec::default(),
        output: Default::default(),
    }
}

fn write_synthetic_extract(path: &Path) -> Result<(), Box<dyn Error>> {
    let mut string_table = osmformat::StringTable::new();
    for value in ["", "highway", "residential", "name", "Synthetic Road"] {
        string_table.mut_s().push(value.as_bytes().to_vec());
    }

    let mut dense_nodes = osmformat::DenseNodes::new();
    let mut previous_id = 0_i64;
    let mut previous_lat = 0_i64;
    let mut previous_lon = 0_i64;

    for index in 0..NODE_COUNT {
        let id = index as i64 + 1;
        let lat = 480_000_000_i64 + (index / 1_000) as i64 * 10;
        let lon = 80_000_000_i64 + (index % 1_000) as i64 * 10;

        dense_nodes.id.push(id - previous_id);
        dense_nodes.lat.push(lat - previous_lat);
        dense_nodes.lon.push(lon - previous_lon);
        dense_nodes.keys_vals.push(0);

        previous_id = id;
        previous_lat = lat;
        previous_lon = lon;
    }

    let mut group = osmformat::PrimitiveGroup::new();
    group.set_dense(dense_nodes);
    for index in 0..WAY_COUNT {
        let first_node_id = (index * NODES_PER_WAY) as i64 + 1;
        let mut way = osmformat::Way::new();
        way.set_id(index as i64 + 1);
        way.keys = vec![1, 3];
        way.vals = vec![2, 4];
        way.refs = vec![first_node_id, 1, 1, 1];
        group.mut_ways().push(way);
    }

    let mut block = osmformat::PrimitiveBlock::new();
    block.set_stringtable(string_table);
    block.mut_primitivegroup().push(group);

    let mut file = File::create(path)?;
    write_raw_blob(&mut file, "OSMData", block.write_to_bytes()?)?;
    Ok(())
}

fn write_raw_blob(
    file: &mut File,
    field_type: &str,
    payload: Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    let mut blob = fileformat::Blob::new();
    blob.set_raw(payload);
    let blob_bytes = blob.write_to_bytes()?;

    let mut header = fileformat::BlobHeader::new();
    header.set_field_type(field_type.to_owned());
    header.set_datasize(blob_bytes.len().try_into()?);
    let header_bytes = header.write_to_bytes()?;

    let header_len: u32 = header_bytes.len().try_into()?;
    file.write_all(&header_len.to_be_bytes())?;
    file.write_all(&header_bytes)?;
    file.write_all(&blob_bytes)?;
    file.flush().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to flush synthetic PBF fixture: {error}"),
        )
    })?;
    Ok(())
}

criterion_group!(benches, bench_filter_pbf);
criterion_main!(benches);
