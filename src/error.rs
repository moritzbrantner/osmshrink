use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, OsmshrinkError>;

#[derive(Debug, Error)]
pub enum OsmshrinkError {
    #[error("invalid Geofabrik source `{0}`: expected geofabrik:<region-path>")]
    InvalidGeofabrikSource(String),

    #[error(
        "invalid Geofabrik region `{0}`: regions may only contain lowercase letters, numbers, dashes, underscores, and slashes"
    )]
    InvalidGeofabrikRegion(String),

    #[error("unsupported source `{0}`: use http(s) URLs or geofabrik:<region-path>")]
    UnsupportedSource(String),

    #[error("unsupported input file `{path}`: expected an .osm.pbf file")]
    UnsupportedInputFile { path: PathBuf },

    #[error("unsupported output file `{path}`: expected extension {expected}")]
    UnsupportedOutputFile {
        path: PathBuf,
        expected: &'static str,
    },

    #[error("unsupported geospatial file `{path}`: expected extension {expected}")]
    UnsupportedGeoFile {
        path: PathBuf,
        expected: &'static str,
    },

    #[error("unable to read `{path}`: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unable to write `{path}`: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unable to parse `{path}` as {format}: {source}")]
    ParseSpec {
        path: PathBuf,
        format: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("unable to parse geospatial data `{path}` as {format}: {details}")]
    ParseGeoData {
        path: PathBuf,
        format: &'static str,
        details: String,
    },

    #[error("invalid filter spec: {0}")]
    InvalidSpec(String),

    #[cfg(feature = "cli")]
    #[error("download failed for `{url}`: {source}")]
    Download {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[cfg(feature = "cli")]
    #[error("HTTP request for `{url}` failed with status {status}")]
    HttpStatus {
        url: String,
        status: reqwest::StatusCode,
    },

    #[error("OSM PBF parsing failed for `{path}`: {source}")]
    Pbf {
        path: PathBuf,
        #[source]
        source: osmpbfreader::Error,
    },

    #[error("regex `{pattern}` is invalid: {source}")]
    Regex {
        pattern: String,
        #[source]
        source: regex::Error,
    },

    #[error("node index failed for `{path}`: {details}")]
    NodeIndex { path: PathBuf, details: String },

    #[error("unsupported runtime option: {0}")]
    UnsupportedRuntime(String),
}
