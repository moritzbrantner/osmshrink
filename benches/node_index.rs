use std::time::Duration;

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use osmpbfreader::NodeId;
use osmshrink::index::{IndexOptions, NodeIndex, RedbNodeIndex, StoredCoordinate};
use osmshrink::spec::IndexMode;
use tempfile::tempdir;

const NODE_COUNT: usize = 20_000;
const LOOKUP_WIDTH: usize = 128;

fn bench_disk_node_reads(c: &mut Criterion) {
    let dir = tempdir().expect("create benchmark directory");
    let options = IndexOptions {
        mode: IndexMode::Disk,
        memory_node_limit: 1,
        disk_dir: Some(dir.path().to_path_buf()),
    };
    let mut index = RedbNodeIndex::create(&options).expect("create redb node index");
    let entries: Vec<_> = (0..NODE_COUNT)
        .map(|index| {
            (
                NodeId(index as i64 + 1),
                StoredCoordinate::new(index as i32, index as i32 + 1),
            )
        })
        .collect();
    index
        .insert_batch(&entries)
        .expect("populate benchmark node index");

    let ids: Vec<_> = (1..=LOOKUP_WIDTH).map(|id| NodeId(id as i64)).collect();

    let mut group = c.benchmark_group("disk_node_index_reads");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(LOOKUP_WIDTH as u64));

    group.bench_function("repeated_get_legacy_shape", |b| {
        b.iter(|| {
            let values: Vec<_> = ids
                .iter()
                .map(|id| {
                    index
                        .get(*id)
                        .expect("lookup succeeds")
                        .expect("node exists")
                })
                .collect();
            black_box(values);
        });
    });

    group.bench_function("get_batch", |b| {
        b.iter(|| {
            let values = index
                .get_batch(black_box(&ids))
                .expect("batch lookup succeeds")
                .expect("all nodes exist");
            black_box(values);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_disk_node_reads);
criterion_main!(benches);
