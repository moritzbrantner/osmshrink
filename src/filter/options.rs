use std::path::PathBuf;

use crate::index::{IndexBackend, IndexOptions};
use crate::model::Feature;
use crate::spec::FilterSpec;
#[cfg(feature = "cli")]
use crate::spec::OutputFormat;

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
    pub(super) fn new(output: PathBuf) -> Self {
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
