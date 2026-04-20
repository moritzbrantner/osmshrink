use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::spec::{IndexMode, OutputFormat};

#[derive(Debug, Parser)]
#[command(name = "osmshrink")]
#[command(about = "Download and filter OpenStreetMap PBF extracts", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase logging verbosity. Use twice for debug logs.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress non-error status output.
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Download an .osm.pbf extract.
    Fetch {
        /// Direct URL or shorthand such as geofabrik:europe/germany/baden-wuerttemberg.
        source: String,

        /// Output .osm.pbf path.
        #[arg(short, long)]
        output: PathBuf,

        /// Redownload even when the output file already exists.
        #[arg(long)]
        force: bool,
    },

    /// Filter a local .osm.pbf file into JSON, NDJSON, or GeoJSON.
    Filter {
        /// Inline JSON/YAML filter, filter rules, tag condition, or spec path.
        #[arg(value_name = "FILTER", conflicts_with = "spec")]
        filter: Option<String>,

        /// Input .osm.pbf path.
        #[arg(short, long)]
        input: PathBuf,

        /// JSON or YAML filter spec file.
        #[arg(short, long, visible_alias = "filter-file", visible_short_alias = 'f')]
        spec: Option<PathBuf>,

        /// Output .json, .ndjson, or .geojson path.
        #[arg(short, long)]
        output: PathBuf,

        /// Override output format from the spec or output extension.
        #[arg(long)]
        format: Option<OutputFormat>,

        /// Override node index mode from the spec.
        #[arg(long, value_enum)]
        index: Option<IndexMode>,

        /// Directory for disk-backed node indexes.
        #[arg(long)]
        index_dir: Option<PathBuf>,

        /// Node count threshold before auto indexing spills to disk.
        #[arg(long)]
        memory_node_limit: Option<usize>,
    },

    /// Fetch and filter in one command.
    Run {
        /// Inline JSON/YAML filter, filter rules, tag condition, or spec path.
        #[arg(value_name = "FILTER", conflicts_with = "spec")]
        filter: Option<String>,

        /// Direct URL or shorthand source.
        #[arg(short, long)]
        source: String,

        /// JSON or YAML filter spec file.
        #[arg(short, long, visible_alias = "filter-file", visible_short_alias = 'f')]
        spec: Option<PathBuf>,

        /// Output .json, .ndjson, or .geojson path.
        #[arg(short, long)]
        output: PathBuf,

        /// Override output format from the spec or output extension.
        #[arg(long)]
        format: Option<OutputFormat>,

        /// Override node index mode from the spec.
        #[arg(long, value_enum)]
        index: Option<IndexMode>,

        /// Directory for disk-backed node indexes.
        #[arg(long)]
        index_dir: Option<PathBuf>,

        /// Node count threshold before auto indexing spills to disk.
        #[arg(long)]
        memory_node_limit: Option<usize>,
    },

    /// Print basic information about an input file.
    Inspect {
        /// Input .osm.pbf path.
        #[arg(short, long)]
        input: PathBuf,
    },

    /// Validate a JSON or YAML filter spec.
    ValidateSpec {
        /// Inline JSON/YAML filter, filter rules, tag condition, or spec path.
        #[arg(value_name = "FILTER", conflicts_with = "spec")]
        filter: Option<String>,

        /// JSON or YAML filter spec file.
        #[arg(short, long, visible_alias = "filter-file", visible_short_alias = 'f')]
        spec: Option<PathBuf>,
    },
}
