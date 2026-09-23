//! Paired comparison: prior scalar lookup loop versus production batch resolution.
//! Index construction/insertion is outside timing; both sides allocate the same output.
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use osmpbfreader::NodeId;
use osmshrink::{
    geometry::Coordinate,
    index::{IndexOptions, MemoryNodeIndex, NodeIndex, RedbNodeIndex, StoredCoordinate},
    spec::IndexMode,
};
use std::time::Duration;

fn scalar(index: &dyn NodeIndex, ids: &[NodeId]) -> Option<Vec<Coordinate>> {
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        result.push(index.get(*id).unwrap()?.to_coordinate());
    }
    Some(result)
}
fn bench(c: &mut Criterion) {
    let entries: Vec<_> = (0..8192)
        .map(|id| (NodeId(id), StoredCoordinate::new(id as i32, -(id as i32))))
        .collect();
    let mut memory = MemoryNodeIndex::new();
    memory.insert_batch(&entries).unwrap();
    let mut disk = RedbNodeIndex::create(&IndexOptions {
        mode: IndexMode::Disk,
        memory_node_limit: 0,
        disk_dir: None,
    })
    .unwrap();
    disk.insert_batch(&entries).unwrap();
    let mut group = c.benchmark_group("coordinate_resolution");
    group
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_secs(1));
    for (backend, index) in [
        ("memory", &memory as &dyn NodeIndex),
        ("disk", &disk as &dyn NodeIndex),
    ] {
        for length in [4, 64, 1024] {
            let ids: Vec<_> = (0..length)
                .map(|i| NodeId((i * 127 % 8192) as i64))
                .collect();
            assert_eq!(
                scalar(index, &ids),
                index.resolve_coordinates(&ids).unwrap()
            );
            group.throughput(Throughput::Elements(length as u64));
            group.bench_with_input(
                BenchmarkId::new(format!("{backend}/scalar"), length),
                &ids,
                |b, ids| b.iter(|| black_box(scalar(black_box(index), black_box(ids)))),
            );
            group.bench_with_input(
                BenchmarkId::new(format!("{backend}/batch"), length),
                &ids,
                |b, ids| b.iter(|| black_box(index.resolve_coordinates(black_box(ids)).unwrap())),
            );
        }
    }
    group.finish();
}
criterion_group!(benches, bench);
criterion_main!(benches);
