use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

use crate::convert::GeoFormat;
use crate::spec::{IndexMode, OutputFormat};

const CLI_LONG_ABOUT: &str = "\
Download OpenStreetMap extracts, filter nodes, ways, and area relations from \
.osm.pbf files, and convert common geospatial JSON formats through a reusable \
neutral feature model.";

const CLI_AFTER_HELP: &str = "\
Examples:
  osmshrink fetch geofabrik:europe/germany/saarland --output data/saarland.osm.pbf
  osmshrink filter --input data/saarland.osm.pbf --spec examples/schools.json --output out/schools.ndjson
  osmshrink convert data/features.geojson --output out/features.json
  osmshrink run --source geofabrik:europe/germany/saarland --filter-file examples/schools.json --output out/schools.geojson

Use `osmshrink <COMMAND> --help` for command-specific options.";

const FETCH_AFTER_HELP: &str = "\
Source values can be direct .osm.pbf URLs or Geofabrik shorthands such as:
  geofabrik:europe/germany/saarland
  https://download.geofabrik.de/europe/germany/saarland-latest.osm.pbf";

const FILTER_AFTER_HELP: &str = "\
Filter input can be a JSON/YAML spec file or an inline filter expression.

Examples:
  osmshrink filter --input data/saarland.osm.pbf --spec examples/schools.json --output out/schools.ndjson
  osmshrink filter --input data/saarland.osm.pbf --output out/schools.ndjson amenity=school
  osmshrink filter --input data/saarland.osm.pbf --output out/roads.geojson '{types: [way], include: {any: [{key: highway}]}}'";

const CONVERT_AFTER_HELP: &str = "\
Input and output formats are normally inferred from .geojson, .json, and .ndjson \
extensions. Use --from or --to when the extension is ambiguous.

The JSON representation is the neutral GeoDataset envelope and preserves dataset \
metadata and CRS information. NDJSON contains feature records only, so the \
conversion report explicitly calls out dataset-level metadata loss.

Examples:
  osmshrink convert data/features.geojson --output out/features.json
  osmshrink convert out/features.json --output out/features.ndjson
  osmshrink convert data/export.json --from geojson --to ndjson --output out/features.ndjson";

const RUN_AFTER_HELP: &str = "\
Fetches the source extract into a temporary file, filters it, and writes the \
requested output.

Examples:
  osmshrink run --source geofabrik:europe/germany/saarland --spec examples/schools.json --output out/schools.ndjson
  osmshrink run -s geofabrik:europe/germany/saarland -f examples/roads.yaml --format geojson --output out/roads.geojson";

const VALIDATE_AFTER_HELP: &str = "\
Examples:
  osmshrink validate-spec --spec examples/schools.json
  osmshrink validate-spec amenity=school";

#[derive(Debug, Parser)]
#[command(name = "osmshrink")]
#[command(about = "Download, filter, and convert geospatial data")]
#[command(long_about = CLI_LONG_ABOUT)]
#[command(after_help = CLI_AFTER_HELP)]
#[command(arg_required_else_help = true)]
#[command(propagate_version = true)]
#[command(next_line_help = true)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase logging verbosity. Use twice for debug logs.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// Suppress non-error status output.
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Download an .osm.pbf extract.
    #[command(after_help = FETCH_AFTER_HELP)]
    Fetch {
        /// Direct URL or shorthand such as geofabrik:europe/germany/baden-wuerttemberg.
        #[arg(value_name = "SOURCE")]
        source: String,

        /// Output .osm.pbf path.
        #[arg(short, long, value_name = "PBF")]
        output: PathBuf,

        /// Redownload even when the output file already exists.
        #[arg(long)]
        force: bool,
    },

    /// Filter a local .osm.pbf file into JSON, NDJSON, or GeoJSON.
    #[command(after_help = FILTER_AFTER_HELP)]
    Filter {
        /// Inline JSON/YAML filter, filter rules, tag condition, or spec path.
        #[arg(value_name = "FILTER", conflicts_with = "spec")]
        filter: Option<String>,

        /// Input .osm.pbf path.
        #[arg(short, long, value_name = "PBF")]
        input: PathBuf,

        /// JSON or YAML filter spec file.
        #[arg(
            short,
            long,
            value_name = "FILE",
            visible_alias = "filter-file",
            visible_short_alias = 'f'
        )]
        spec: Option<PathBuf>,

        /// Output .json, .ndjson, or .geojson path.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,

        /// Override output format from the spec or output extension.
        #[arg(long, value_name = "FORMAT")]
        format: Option<OutputFormat>,

        /// Override node index mode from the spec.
        #[arg(long, value_enum, value_name = "MODE")]
        index: Option<IndexMode>,

        /// Directory for disk-backed node indexes.
        #[arg(long, value_name = "DIR")]
        index_dir: Option<PathBuf>,

        /// Node count threshold before auto indexing spills to disk.
        #[arg(long, value_name = "N")]
        memory_node_limit: Option<usize>,
    },

    /// Convert a geospatial dataset between supported interchange formats.
    #[command(after_help = CONVERT_AFTER_HELP)]
    Convert {
        /// Input geospatial file.
        #[arg(value_name = "INPUT")]
        input: PathBuf,

        /// Output geospatial file.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,

        /// Override the input format instead of inferring it from the extension.
        #[arg(long, value_enum, value_name = "FORMAT")]
        from: Option<GeoFormat>,

        /// Override the output format instead of inferring it from the extension.
        #[arg(long, value_enum, value_name = "FORMAT")]
        to: Option<GeoFormat>,
    },

    /// Fetch and filter in one command.
    #[command(after_help = RUN_AFTER_HELP)]
    Run {
        /// Inline JSON/YAML filter, filter rules, tag condition, or spec path.
        #[arg(value_name = "FILTER", conflicts_with = "spec")]
        filter: Option<String>,

        /// Direct URL or shorthand source.
        #[arg(short, long, value_name = "SOURCE")]
        source: String,

        /// JSON or YAML filter spec file.
        #[arg(short = 'f', long, value_name = "FILE", visible_alias = "filter-file")]
        spec: Option<PathBuf>,

        /// Output .json, .ndjson, or .geojson path.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,

        /// Override output format from the spec or output extension.
        #[arg(long, value_name = "FORMAT")]
        format: Option<OutputFormat>,

        /// Override node index mode from the spec.
        #[arg(long, value_enum, value_name = "MODE")]
        index: Option<IndexMode>,

        /// Directory for disk-backed node indexes.
        #[arg(long, value_name = "DIR")]
        index_dir: Option<PathBuf>,

        /// Node count threshold before auto indexing spills to disk.
        #[arg(long, value_name = "N")]
        memory_node_limit: Option<usize>,
    },

    /// Print basic information about an input file.
    Inspect {
        /// Input .osm.pbf path.
        #[arg(short, long, value_name = "PBF")]
        input: PathBuf,
    },

    /// Validate a JSON or YAML filter spec.
    #[command(after_help = VALIDATE_AFTER_HELP)]
    ValidateSpec {
        /// Inline JSON/YAML filter, filter rules, tag condition, or spec path.
        #[arg(value_name = "FILTER", conflicts_with = "spec")]
        filter: Option<String>,

        /// JSON or YAML filter spec file.
        #[arg(
            short,
            long,
            value_name = "FILE",
            visible_alias = "filter-file",
            visible_short_alias = 'f'
        )]
        spec: Option<PathBuf>,
    },
}
