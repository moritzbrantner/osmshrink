pub mod cli;
pub mod error;
pub mod fetch;
pub mod filter;
pub mod geofabrik;
pub mod geometry;
pub mod index;
pub mod inspect;
pub mod model;
pub mod output;
pub mod spec;

pub use error::{OsmshrinkError, Result};
pub use filter::{
    CollectReport, CollectRunOptions, CollectedFeatures, FilterReport, FilterRunOptions,
    collect_pbf, filter_pbf,
};
pub use geometry::Geometry;
pub use model::{ElementKind, Feature, Tags};
pub use spec::{FilterSpec, OutputFormat};
