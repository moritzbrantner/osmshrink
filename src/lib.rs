#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
#[cfg(feature = "cli")]
pub mod fetch;
pub mod filter;
pub mod geofabrik;
pub mod geometry;
pub mod index;
#[cfg(feature = "cli")]
pub mod inspect;
pub mod model;
#[cfg(feature = "cli")]
pub mod output;
pub mod spec;

pub use error::{OsmshrinkError, Result};
pub use filter::{
    CollectBytesOptions, CollectReport, CollectedFeatures, FilterReport, collect_pbf_bytes,
};
#[cfg(feature = "cli")]
pub use filter::{CollectRunOptions, FilterRunOptions, collect_pbf, filter_pbf};
pub use geometry::Geometry;
pub use model::{ElementKind, Feature, Tags};
pub use spec::{FilterSpec, OutputFormat};
