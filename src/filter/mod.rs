//! Filters OSM PBF extracts into features.
//!
//! - `pipeline`: the read passes over the input and per-element processing
//! - `compiled`: compiled filter rules and tag matching
//! - `relation`: area-relation assembly from member ways
//! - `sink`: where matched features are written
//! - `options`: run options and reports

mod compiled;
mod options;
mod pipeline;
mod relation;
mod sink;
#[cfg(test)]
mod tests;

pub use compiled::{CompiledCondition, CompiledFilter};
pub use options::{CollectBytesOptions, CollectReport, CollectedFeatures, FilterReport};
#[cfg(feature = "cli")]
pub use options::{CollectRunOptions, FilterRunOptions};
pub use pipeline::collect_pbf_bytes;
#[cfg(feature = "cli")]
pub use pipeline::{collect_pbf, filter_pbf};
