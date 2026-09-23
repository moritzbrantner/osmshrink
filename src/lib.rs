#[cfg(feature = "flatgeobuf")]
mod atomic_output;
#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "cli")]
pub mod convert;
pub mod error;
#[cfg(feature = "flatgeobuf")]
mod feature_io;
#[cfg(feature = "cli")]
pub mod fetch;
pub mod filter;
#[cfg(feature = "flatgeobuf")]
pub mod flatgeobuf_io;
pub mod geo;
pub mod geofabrik;
pub mod geometry;
pub mod index;
#[cfg(feature = "cli")]
pub mod inspect;
pub mod model;
#[cfg(feature = "cli")]
pub mod output;
pub mod spec;

#[cfg(feature = "cli")]
pub use convert::{
    ConversionLoss, ConversionLossKind, ConversionReport, ConvertOptions, GeoFormat, convert_path,
    read_dataset, write_dataset,
};
pub use error::{OsmshrinkError, Result};
pub use filter::{
    CollectBytesOptions, CollectReport, CollectedFeatures, FilterReport, collect_pbf_bytes,
};
#[cfg(feature = "cli")]
pub use filter::{CollectRunOptions, FilterRunOptions, collect_pbf, filter_pbf};
#[cfg(feature = "flatgeobuf")]
pub use flatgeobuf_io::{
    FlatGeobufStreamReport, read_flatgeobuf_dataset, stream_flatgeobuf_to_ndjson,
    stream_ndjson_to_flatgeobuf, write_flatgeobuf_dataset,
};
pub use geo::{
    GeoDataset, GeoFeature, GeoFeatureId, GeoFeatureReader, GeoFeatureWriter, GeoMetadata,
    GeoProperties, pipe_features,
};
pub use geometry::Geometry;
pub use model::{ElementKind, Feature, Tags};
pub use spec::{FilterSpec, OutputFormat};
