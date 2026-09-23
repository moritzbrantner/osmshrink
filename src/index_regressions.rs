use super::*;

fn assert_coordinate_contract(index: &mut dyn NodeIndex) {
    let a = StoredCoordinate::new(80_000_000, 480_000_000);
    let b = StoredCoordinate::new(-80_000_000, -480_000_000);
    index
        .insert_batch(&[(NodeId(1), a), (NodeId(2), b)])
        .unwrap();
    assert_eq!(
        index
            .resolve_coordinates(&[NodeId(2), NodeId(1), NodeId(2)])
            .unwrap(),
        Some(vec![
            b.to_coordinate(),
            a.to_coordinate(),
            b.to_coordinate()
        ])
    );
    assert_eq!(
        index
            .resolve_coordinates(&[NodeId(1), NodeId(99), NodeId(2)])
            .unwrap(),
        None
    );
    assert_eq!(index.resolve_coordinates(&[]).unwrap(), Some(Vec::new()));
}
#[test]
fn memory_batch_resolution_preserves_order_duplicates_and_missing_semantics() {
    assert_coordinate_contract(&mut MemoryNodeIndex::new());
}
#[cfg(feature = "disk-index")]
fn options(mode: IndexMode) -> IndexOptions {
    IndexOptions {
        mode,
        memory_node_limit: 2,
        disk_dir: None,
    }
}
#[test]
#[cfg(feature = "disk-index")]
fn disk_batch_resolution_uses_one_transaction_independent_of_way_length() {
    let mut index = RedbNodeIndex::create(&options(IndexMode::Disk)).unwrap();
    assert_coordinate_contract(&mut index);
    for size in [1, 4, 64, 4096] {
        index.read_transactions.set(0);
        let coordinates = index
            .resolve_coordinates(&vec![NodeId(1); size])
            .unwrap()
            .unwrap();
        assert_eq!(coordinates.len(), size);
        assert_eq!(index.read_transactions.get(), 1);
    }
    index.read_transactions.set(0);
    assert_eq!(index.resolve_coordinates(&[]).unwrap(), Some(Vec::new()));
    assert_eq!(index.read_transactions.get(), 0);
}
#[test]
#[cfg(feature = "disk-index")]
fn automatic_index_forwards_batch_reads_to_disk() {
    let mut index = AutoNodeIndex::create(options(IndexMode::Auto)).unwrap();
    index
        .insert_batch(
            &(1..=4)
                .map(|id| (NodeId(id), StoredCoordinate::new(id as i32, 0)))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(index.backend(), IndexBackend::Disk);
    assert_eq!(
        index
            .resolve_coordinates(&[NodeId(1); 64])
            .unwrap()
            .unwrap()
            .len(),
        64
    );
    let AutoNodeIndexInner::Disk(disk) = &index.inner else {
        panic!("expected disk");
    };
    assert_eq!(
        disk.read_transactions.get(),
        1,
        "Auto must not fall back to scalar get"
    );
}
#[test]
#[cfg(feature = "disk-index")]
fn failed_spill_retains_every_node_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let invalid_dir = dir.path().join("not-a-directory");
    std::fs::write(&invalid_dir, b"sentinel").unwrap();
    let mut settings = options(IndexMode::Auto);
    settings.disk_dir = Some(invalid_dir.clone());
    let mut index = AutoNodeIndex::create(settings).unwrap();
    index
        .insert(NodeId(1), StoredCoordinate::new(1, 0))
        .unwrap();
    index
        .insert(NodeId(2), StoredCoordinate::new(2, 0))
        .unwrap();
    assert!(
        index
            .insert(NodeId(3), StoredCoordinate::new(3, 0))
            .is_err()
    );
    assert_eq!(index.backend(), IndexBackend::Memory);
    assert_eq!(index.len(), 3);
    for id in 1..=3 {
        assert_eq!(
            index.get(NodeId(id)).unwrap(),
            Some(StoredCoordinate::new(id as i32, 0))
        );
    }
    assert_eq!(std::fs::read(&invalid_dir).unwrap(), b"sentinel");
    std::fs::remove_file(&invalid_dir).unwrap();
    std::fs::create_dir(&invalid_dir).unwrap();
    index.spill_to_disk().unwrap();
    assert_eq!(index.backend(), IndexBackend::Disk);
    assert_eq!(index.len(), 3);
    for id in 1..=3 {
        assert_eq!(
            index.get(NodeId(id)).unwrap(),
            Some(StoredCoordinate::new(id as i32, 0))
        );
    }
}
#[test]
#[cfg(feature = "disk-index")]
fn spilling_across_transfer_batch_boundary_preserves_all_entries() {
    let mut settings = options(IndexMode::Auto);
    settings.memory_node_limit = 16_384;
    let mut index = AutoNodeIndex::create(settings).unwrap();
    let entries: Vec<_> = (0..16_389)
        .map(|id| (NodeId(id), StoredCoordinate::new(id as i32, -(id as i32))))
        .collect();
    index.insert_batch(&entries).unwrap();
    assert_eq!(index.len(), entries.len());
    assert_eq!(index.backend(), IndexBackend::Disk);
    let actual = index
        .resolve_coordinates(&entries.iter().map(|(id, _)| *id).collect::<Vec<_>>())
        .unwrap()
        .unwrap();
    assert_eq!(
        actual,
        entries
            .iter()
            .map(|(_, c)| c.to_coordinate())
            .collect::<Vec<_>>()
    );
}
