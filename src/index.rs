use std::collections::HashMap;
#[cfg(feature = "disk-index")]
use std::path::Path;
use std::path::PathBuf;

use osmpbfreader::NodeId;
#[cfg(feature = "disk-index")]
use redb::{Database, ReadableDatabase, TableDefinition};
#[cfg(feature = "disk-index")]
use tempfile::{NamedTempFile, TempDir};

use crate::error::{OsmshrinkError, Result};
use crate::geometry::Coordinate;
use crate::spec::{IndexMode, IndexSpec};

#[cfg(feature = "disk-index")]
const NODE_TABLE: TableDefinition<i64, &[u8]> = TableDefinition::new("nodes");
#[cfg(feature = "disk-index")]
const STORED_COORDINATE_BYTES: usize = 8;

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub mode: IndexMode,
    pub memory_node_limit: usize,
    pub disk_dir: Option<PathBuf>,
}

impl IndexOptions {
    pub fn from_spec(spec: &IndexSpec) -> Self {
        Self {
            mode: spec.mode,
            memory_node_limit: spec.memory_node_limit,
            disk_dir: spec.disk_dir.clone(),
        }
    }

    pub fn apply_overrides(
        &mut self,
        mode: Option<IndexMode>,
        disk_dir: Option<PathBuf>,
        memory_node_limit: Option<usize>,
    ) -> Result<()> {
        if let Some(mode) = mode {
            self.mode = mode;
        }
        if let Some(disk_dir) = disk_dir {
            self.disk_dir = Some(disk_dir);
        }
        if let Some(memory_node_limit) = memory_node_limit {
            if memory_node_limit == 0 {
                return Err(OsmshrinkError::InvalidSpec(
                    "--memory-node-limit must be greater than zero".to_owned(),
                ));
            }
            self.memory_node_limit = memory_node_limit;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexBackend {
    Memory,
    Disk,
}

impl IndexBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Disk => "disk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredCoordinate {
    pub decimicro_lon: i32,
    pub decimicro_lat: i32,
}

impl StoredCoordinate {
    pub fn new(decimicro_lon: i32, decimicro_lat: i32) -> Self {
        Self {
            decimicro_lon,
            decimicro_lat,
        }
    }

    pub fn from_degrees(lon: f64, lat: f64) -> Self {
        Self {
            decimicro_lon: (lon / 1e-7).round() as i32,
            decimicro_lat: (lat / 1e-7).round() as i32,
        }
    }

    pub fn to_coordinate(self) -> Coordinate {
        Coordinate::new(
            self.decimicro_lon as f64 * 1e-7,
            self.decimicro_lat as f64 * 1e-7,
        )
    }

    #[cfg(feature = "disk-index")]
    fn to_bytes(self) -> [u8; STORED_COORDINATE_BYTES] {
        let mut bytes = [0_u8; STORED_COORDINATE_BYTES];
        bytes[..4].copy_from_slice(&self.decimicro_lon.to_le_bytes());
        bytes[4..].copy_from_slice(&self.decimicro_lat.to_le_bytes());
        bytes
    }

    #[cfg(feature = "disk-index")]
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != STORED_COORDINATE_BYTES {
            return None;
        }
        let lon = i32::from_le_bytes(bytes[..4].try_into().ok()?);
        let lat = i32::from_le_bytes(bytes[4..].try_into().ok()?);
        Some(Self::new(lon, lat))
    }
}

pub trait NodeIndex {
    fn insert(&mut self, node_id: NodeId, coordinate: StoredCoordinate) -> Result<()>;

    fn insert_batch(&mut self, entries: &[(NodeId, StoredCoordinate)]) -> Result<()> {
        for (node_id, coordinate) in entries {
            self.insert(*node_id, *coordinate)?;
        }
        Ok(())
    }

    fn get(&self, node_id: NodeId) -> Result<Option<StoredCoordinate>>;

    fn get_batch(&self, node_ids: &[NodeId]) -> Result<Option<Vec<StoredCoordinate>>> {
        let mut coordinates = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            let Some(coordinate) = self.get(*node_id)? else {
                return Ok(None);
            };
            coordinates.push(coordinate);
        }
        Ok(Some(coordinates))
    }

    fn backend(&self) -> IndexBackend;
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Default)]
pub struct MemoryNodeIndex {
    nodes: HashMap<NodeId, StoredCoordinate>,
}

impl MemoryNodeIndex {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "disk-index")]
    fn drain(self) -> HashMap<NodeId, StoredCoordinate> {
        self.nodes
    }
}

impl NodeIndex for MemoryNodeIndex {
    fn insert(&mut self, node_id: NodeId, coordinate: StoredCoordinate) -> Result<()> {
        self.nodes.insert(node_id, coordinate);
        Ok(())
    }

    fn insert_batch(&mut self, entries: &[(NodeId, StoredCoordinate)]) -> Result<()> {
        self.nodes.reserve(entries.len());
        for (node_id, coordinate) in entries {
            self.nodes.insert(*node_id, *coordinate);
        }
        Ok(())
    }

    fn get(&self, node_id: NodeId) -> Result<Option<StoredCoordinate>> {
        Ok(self.nodes.get(&node_id).copied())
    }

    fn get_batch(&self, node_ids: &[NodeId]) -> Result<Option<Vec<StoredCoordinate>>> {
        let mut coordinates = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            let Some(coordinate) = self.nodes.get(node_id).copied() else {
                return Ok(None);
            };
            coordinates.push(coordinate);
        }
        Ok(Some(coordinates))
    }

    fn backend(&self) -> IndexBackend {
        IndexBackend::Memory
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(feature = "disk-index")]
#[derive(Debug)]
pub struct RedbNodeIndex {
    database: Database,
    path: PathBuf,
    _temp_file: Option<NamedTempFile>,
    _temp_dir: Option<TempDir>,
    len: usize,
}

#[cfg(feature = "disk-index")]
impl RedbNodeIndex {
    pub fn create(options: &IndexOptions) -> Result<Self> {
        let (path, temp_file, temp_dir) = index_path(options)?;
        let database = Database::create(&path).map_err(|source| OsmshrinkError::NodeIndex {
            path: path.clone(),
            details: source.to_string(),
        })?;
        let index = Self {
            database,
            path,
            _temp_file: temp_file,
            _temp_dir: temp_dir,
            len: 0,
        };
        index.create_table()?;
        Ok(index)
    }

    fn create_table(&self) -> Result<()> {
        let write_txn =
            self.database
                .begin_write()
                .map_err(|source| OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: source.to_string(),
                })?;
        {
            write_txn
                .open_table(NODE_TABLE)
                .map_err(|source| OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: source.to_string(),
                })?;
        }
        write_txn
            .commit()
            .map_err(|source| OsmshrinkError::NodeIndex {
                path: self.path.clone(),
                details: source.to_string(),
            })
    }
}

#[cfg(feature = "disk-index")]
impl NodeIndex for RedbNodeIndex {
    fn insert(&mut self, node_id: NodeId, coordinate: StoredCoordinate) -> Result<()> {
        self.insert_batch(&[(node_id, coordinate)])
    }

    fn insert_batch(&mut self, entries: &[(NodeId, StoredCoordinate)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let write_txn =
            self.database
                .begin_write()
                .map_err(|source| OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: source.to_string(),
                })?;
        {
            let mut table =
                write_txn
                    .open_table(NODE_TABLE)
                    .map_err(|source| OsmshrinkError::NodeIndex {
                        path: self.path.clone(),
                        details: source.to_string(),
                    })?;
            for (node_id, coordinate) in entries {
                let bytes = coordinate.to_bytes();
                if table
                    .insert(node_id.0, bytes.as_slice())
                    .map_err(|source| OsmshrinkError::NodeIndex {
                        path: self.path.clone(),
                        details: source.to_string(),
                    })?
                    .is_none()
                {
                    self.len += 1;
                }
            }
        }
        write_txn
            .commit()
            .map_err(|source| OsmshrinkError::NodeIndex {
                path: self.path.clone(),
                details: source.to_string(),
            })
    }

    fn get(&self, node_id: NodeId) -> Result<Option<StoredCoordinate>> {
        let read_txn = self
            .database
            .begin_read()
            .map_err(|source| OsmshrinkError::NodeIndex {
                path: self.path.clone(),
                details: source.to_string(),
            })?;
        let table =
            read_txn
                .open_table(NODE_TABLE)
                .map_err(|source| OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: source.to_string(),
                })?;
        table
            .get(node_id.0)
            .map_err(|source| OsmshrinkError::NodeIndex {
                path: self.path.clone(),
                details: source.to_string(),
            })?
            .map(|value| {
                StoredCoordinate::from_bytes(value.value()).ok_or_else(|| {
                    OsmshrinkError::NodeIndex {
                        path: self.path.clone(),
                        details: "stored coordinate has invalid byte length".to_owned(),
                    }
                })
            })
            .transpose()
    }

    fn get_batch(&self, node_ids: &[NodeId]) -> Result<Option<Vec<StoredCoordinate>>> {
        let read_txn = self
            .database
            .begin_read()
            .map_err(|source| OsmshrinkError::NodeIndex {
                path: self.path.clone(),
                details: source.to_string(),
            })?;
        let table =
            read_txn
                .open_table(NODE_TABLE)
                .map_err(|source| OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: source.to_string(),
                })?;
        let mut coordinates = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            let Some(value) = table
                .get(node_id.0)
                .map_err(|source| OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: source.to_string(),
                })?
            else {
                return Ok(None);
            };
            let coordinate = StoredCoordinate::from_bytes(value.value()).ok_or_else(|| {
                OsmshrinkError::NodeIndex {
                    path: self.path.clone(),
                    details: "stored coordinate has invalid byte length".to_owned(),
                }
            })?;
            coordinates.push(coordinate);
        }
        Ok(Some(coordinates))
    }

    fn backend(&self) -> IndexBackend {
        IndexBackend::Disk
    }

    fn len(&self) -> usize {
        self.len
    }
}

#[derive(Debug)]
pub struct AutoNodeIndex {
    options: IndexOptions,
    inner: AutoNodeIndexInner,
}

#[derive(Debug)]
enum AutoNodeIndexInner {
    Memory(MemoryNodeIndex),
    #[cfg(feature = "disk-index")]
    Disk(RedbNodeIndex),
}

impl AutoNodeIndex {
    pub fn create(options: IndexOptions) -> Result<Self> {
        let inner = match options.mode {
            IndexMode::Memory | IndexMode::Auto => {
                AutoNodeIndexInner::Memory(MemoryNodeIndex::new())
            }
            #[cfg(feature = "disk-index")]
            IndexMode::Disk => AutoNodeIndexInner::Disk(RedbNodeIndex::create(&options)?),
            #[cfg(not(feature = "disk-index"))]
            IndexMode::Disk => {
                return Err(OsmshrinkError::UnsupportedRuntime(
                    "disk-backed node indexes are not supported in this build".to_owned(),
                ));
            }
        };
        Ok(Self { options, inner })
    }

    #[cfg(feature = "disk-index")]
    fn spill_to_disk(&mut self) -> Result<()> {
        let AutoNodeIndexInner::Memory(memory) = std::mem::replace(
            &mut self.inner,
            AutoNodeIndexInner::Memory(MemoryNodeIndex::new()),
        ) else {
            return Ok(());
        };
        let mut disk = RedbNodeIndex::create(&self.options)?;
        let entries: Vec<_> = memory.drain().into_iter().collect();
        disk.insert_batch(&entries)?;
        self.inner = AutoNodeIndexInner::Disk(disk);
        Ok(())
    }
}

impl NodeIndex for AutoNodeIndex {
    fn insert(&mut self, node_id: NodeId, coordinate: StoredCoordinate) -> Result<()> {
        match &mut self.inner {
            AutoNodeIndexInner::Memory(memory) => {
                memory.insert(node_id, coordinate)?;
                if self.options.mode == IndexMode::Auto
                    && memory.len() > self.options.memory_node_limit
                {
                    #[cfg(feature = "disk-index")]
                    self.spill_to_disk()?;
                }
                Ok(())
            }
            #[cfg(feature = "disk-index")]
            AutoNodeIndexInner::Disk(disk) => disk.insert(node_id, coordinate),
        }
    }

    fn insert_batch(&mut self, entries: &[(NodeId, StoredCoordinate)]) -> Result<()> {
        match &mut self.inner {
            AutoNodeIndexInner::Memory(memory) => {
                memory.insert_batch(entries)?;
                if self.options.mode == IndexMode::Auto
                    && memory.len() > self.options.memory_node_limit
                {
                    #[cfg(feature = "disk-index")]
                    self.spill_to_disk()?;
                }
                Ok(())
            }
            #[cfg(feature = "disk-index")]
            AutoNodeIndexInner::Disk(disk) => disk.insert_batch(entries),
        }
    }

    fn get(&self, node_id: NodeId) -> Result<Option<StoredCoordinate>> {
        match &self.inner {
            AutoNodeIndexInner::Memory(memory) => memory.get(node_id),
            #[cfg(feature = "disk-index")]
            AutoNodeIndexInner::Disk(disk) => disk.get(node_id),
        }
    }

    fn get_batch(&self, node_ids: &[NodeId]) -> Result<Option<Vec<StoredCoordinate>>> {
        match &self.inner {
            AutoNodeIndexInner::Memory(memory) => memory.get_batch(node_ids),
            #[cfg(feature = "disk-index")]
            AutoNodeIndexInner::Disk(disk) => disk.get_batch(node_ids),
        }
    }

    fn backend(&self) -> IndexBackend {
        match &self.inner {
            AutoNodeIndexInner::Memory(memory) => memory.backend(),
            #[cfg(feature = "disk-index")]
            AutoNodeIndexInner::Disk(disk) => disk.backend(),
        }
    }

    fn len(&self) -> usize {
        match &self.inner {
            AutoNodeIndexInner::Memory(memory) => memory.len(),
            #[cfg(feature = "disk-index")]
            AutoNodeIndexInner::Disk(disk) => disk.len(),
        }
    }
}

#[cfg(feature = "disk-index")]
fn index_path(options: &IndexOptions) -> Result<(PathBuf, Option<NamedTempFile>, Option<TempDir>)> {
    if let Some(dir) = &options.disk_dir {
        std::fs::create_dir_all(dir).map_err(|source| OsmshrinkError::NodeIndex {
            path: dir.clone(),
            details: source.to_string(),
        })?;
        let temp_file = NamedTempFile::new_in(dir).map_err(|source| OsmshrinkError::NodeIndex {
            path: dir.clone(),
            details: source.to_string(),
        })?;
        let path = temp_file.path().to_path_buf();
        Ok((path, Some(temp_file), None))
    } else {
        let temp_dir = tempfile::tempdir().map_err(|source| OsmshrinkError::NodeIndex {
            path: Path::new(".").to_path_buf(),
            details: source.to_string(),
        })?;
        let path = temp_dir.path().join("nodes.redb");
        Ok((path, None, Some(temp_dir)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_index_batch_lookup_preserves_order_and_missing_semantics() {
        let mut index = MemoryNodeIndex::new();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(10, 20)),
                (NodeId(2), StoredCoordinate::new(30, 40)),
            ])
            .unwrap();

        assert_eq!(
            index.get_batch(&[NodeId(2), NodeId(1)]).unwrap(),
            Some(vec![
                StoredCoordinate::new(30, 40),
                StoredCoordinate::new(10, 20),
            ])
        );
        assert_eq!(index.get_batch(&[NodeId(1), NodeId(3)]).unwrap(), None);
    }

    #[test]
    fn memory_index_round_trips_coordinates() {
        let mut index = MemoryNodeIndex::new();
        index
            .insert(NodeId(1), StoredCoordinate::new(87_000_000, 489_000_000))
            .unwrap();
        assert_eq!(
            index.get(NodeId(1)).unwrap(),
            Some(StoredCoordinate::new(87_000_000, 489_000_000))
        );
        assert_eq!(index.backend(), IndexBackend::Memory);
    }

    #[cfg(feature = "disk-index")]
    #[test]
    fn redb_index_batch_lookup_preserves_order_and_missing_semantics() {
        let options = IndexOptions {
            mode: IndexMode::Disk,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let mut index = RedbNodeIndex::create(&options).unwrap();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(10, 20)),
                (NodeId(2), StoredCoordinate::new(30, 40)),
            ])
            .unwrap();

        assert_eq!(
            index.get_batch(&[NodeId(2), NodeId(1)]).unwrap(),
            Some(vec![
                StoredCoordinate::new(30, 40),
                StoredCoordinate::new(10, 20),
            ])
        );
        assert_eq!(index.get_batch(&[NodeId(1), NodeId(3)]).unwrap(), None);
    }

    #[cfg(feature = "disk-index")]
    #[test]
    fn redb_index_round_trips_coordinates() {
        let options = IndexOptions {
            mode: IndexMode::Disk,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let mut index = RedbNodeIndex::create(&options).unwrap();
        index
            .insert(NodeId(2), StoredCoordinate::new(1_000_000, 2_000_000))
            .unwrap();
        assert_eq!(
            index.get(NodeId(2)).unwrap(),
            Some(StoredCoordinate::new(1_000_000, 2_000_000))
        );
        assert_eq!(index.backend(), IndexBackend::Disk);
    }

    #[cfg(feature = "disk-index")]
    #[test]
    fn redb_index_batch_round_trips_coordinates() {
        let options = IndexOptions {
            mode: IndexMode::Disk,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let mut index = RedbNodeIndex::create(&options).unwrap();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(10, 20)),
                (NodeId(2), StoredCoordinate::new(30, 40)),
                (NodeId(3), StoredCoordinate::new(50, 60)),
            ])
            .unwrap();

        assert_eq!(index.len(), 3);
        assert_eq!(
            index.get(NodeId(1)).unwrap(),
            Some(StoredCoordinate::new(10, 20))
        );
        assert_eq!(
            index.get(NodeId(3)).unwrap(),
            Some(StoredCoordinate::new(50, 60))
        );
    }

    #[cfg(feature = "disk-index")]
    #[test]
    fn batched_insert_replacement_preserves_len() {
        let options = IndexOptions {
            mode: IndexMode::Disk,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let mut index = RedbNodeIndex::create(&options).unwrap();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(10, 20)),
                (NodeId(2), StoredCoordinate::new(30, 40)),
            ])
            .unwrap();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(11, 22)),
                (NodeId(2), StoredCoordinate::new(33, 44)),
            ])
            .unwrap();

        assert_eq!(index.len(), 2);
        assert_eq!(
            index.get(NodeId(1)).unwrap(),
            Some(StoredCoordinate::new(11, 22))
        );
        assert_eq!(
            index.get(NodeId(2)).unwrap(),
            Some(StoredCoordinate::new(33, 44))
        );
    }

    #[cfg(feature = "disk-index")]
    #[test]
    fn auto_index_spills_after_threshold() {
        let options = IndexOptions {
            mode: IndexMode::Auto,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let mut index = AutoNodeIndex::create(options).unwrap();
        index
            .insert(NodeId(1), StoredCoordinate::new(1, 1))
            .unwrap();
        assert_eq!(index.backend(), IndexBackend::Memory);
        index
            .insert(NodeId(2), StoredCoordinate::new(2, 2))
            .unwrap();
        assert_eq!(index.backend(), IndexBackend::Disk);
        assert_eq!(
            index.get(NodeId(1)).unwrap(),
            Some(StoredCoordinate::new(1, 1))
        );
    }

    #[cfg(feature = "disk-index")]
    #[test]
    fn auto_index_spills_when_batch_crosses_threshold() {
        let options = IndexOptions {
            mode: IndexMode::Auto,
            memory_node_limit: 2,
            disk_dir: None,
        };
        let mut index = AutoNodeIndex::create(options).unwrap();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(1, 1)),
                (NodeId(2), StoredCoordinate::new(2, 2)),
                (NodeId(3), StoredCoordinate::new(3, 3)),
            ])
            .unwrap();

        assert_eq!(index.backend(), IndexBackend::Disk);
        assert_eq!(index.len(), 3);
        assert_eq!(
            index.get(NodeId(2)).unwrap(),
            Some(StoredCoordinate::new(2, 2))
        );
    }

    #[cfg(not(feature = "disk-index"))]
    #[test]
    fn disk_index_is_rejected_without_disk_feature() {
        let options = IndexOptions {
            mode: IndexMode::Disk,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let error = AutoNodeIndex::create(options).unwrap_err();
        assert!(matches!(error, OsmshrinkError::UnsupportedRuntime(_)));
    }

    #[cfg(not(feature = "disk-index"))]
    #[test]
    fn auto_index_stays_in_memory_without_disk_feature() {
        let options = IndexOptions {
            mode: IndexMode::Auto,
            memory_node_limit: 1,
            disk_dir: None,
        };
        let mut index = AutoNodeIndex::create(options).unwrap();
        index
            .insert_batch(&[
                (NodeId(1), StoredCoordinate::new(1, 1)),
                (NodeId(2), StoredCoordinate::new(2, 2)),
            ])
            .unwrap();

        assert_eq!(index.backend(), IndexBackend::Memory);
        assert_eq!(index.len(), 2);
    }
}
