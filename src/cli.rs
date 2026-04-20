use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::spec::OutputFormat;

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
    },

    /// Filter a local .osm.pbf file into JSON or NDJSON.
    Filter {
        /// Input .osm.pbf path.
        #[arg(short, long)]
        input: PathBuf,

        /// JSON or YAML filter spec.
        #[arg(short, long)]
        spec: PathBuf,

        /// Output .json or .ndjson path.
        #[arg(short, long)]
        output: PathBuf,

        /// Override output format from the spec or output extension.
        #[arg(long)]
        format: Option<OutputFormat>,
    },

    /// Fetch and filter in one command.
    Run {
        /// Direct URL or shorthand source.
        #[arg(short, long)]
        source: String,

        /// JSON or YAML filter spec.
        #[arg(short, long)]
        spec: PathBuf,

        /// Output .json or .ndjson path.
        #[arg(short, long)]
        output: PathBuf,

        /// Override output format from the spec or output extension.
        #[arg(long)]
        format: Option<OutputFormat>,
    },

    /// Print basic information about an input file.
    Inspect {
        /// Input .osm.pbf path.
        #[arg(short, long)]
        input: PathBuf,
    },

    /// Validate a JSON or YAML filter spec.
    ValidateSpec {
        /// JSON or YAML filter spec.
        #[arg(short, long)]
        spec: PathBuf,
    },
}
