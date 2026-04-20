use std::path::PathBuf;

use clap::Parser;
use osmshrink::cli::{Cli, Commands};
use osmshrink::fetch::{FetchOptions, download_source};
use osmshrink::filter::{FilterRunOptions, filter_pbf};
use osmshrink::index::IndexOptions;
use osmshrink::inspect::inspect_path;
use osmshrink::spec::FilterSpec;
use tempfile::tempdir;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet)?;

    match cli.command {
        Commands::Fetch {
            source,
            output,
            force,
        } => {
            let options = FetchOptions {
                show_progress: !cli.quiet,
                force,
            };
            let report = download_source(&source, &output, &options).await?;
            if !cli.quiet {
                if report.cached {
                    eprintln!(
                        "Using cached {} bytes from {} at {}",
                        report.bytes_written,
                        report.url,
                        report.output.display()
                    );
                } else {
                    eprintln!(
                        "Downloaded {} bytes from {} to {}",
                        report.bytes_written,
                        report.url,
                        report.output.display()
                    );
                }
            }
        }
        Commands::Filter {
            filter,
            input,
            spec,
            output,
            format,
            index,
            index_dir,
            memory_node_limit,
        } => {
            let spec = read_spec_input(spec, filter)?;
            let mut index_options = IndexOptions::from_spec(&spec.processing.index);
            index_options.apply_overrides(index, index_dir, memory_node_limit)?;
            let report = filter_pbf(FilterRunOptions {
                input,
                output,
                spec,
                format_override: format,
                index_options,
            })?;
            if !cli.quiet {
                print_filter_report(&report);
            }
        }
        Commands::Run {
            filter,
            source,
            spec,
            output,
            format,
            index,
            index_dir,
            memory_node_limit,
        } => {
            let temp_dir = tempdir()?;
            let pbf_path = temp_dir.path().join("source.osm.pbf");
            let fetch_report = download_source(
                &source,
                &pbf_path,
                &FetchOptions {
                    show_progress: !cli.quiet,
                    force: false,
                },
            )
            .await?;
            if !cli.quiet {
                eprintln!(
                    "Downloaded {} bytes from {}",
                    fetch_report.bytes_written, fetch_report.url
                );
            }

            let spec = read_spec_input(spec, filter)?;
            let mut index_options = IndexOptions::from_spec(&spec.processing.index);
            index_options.apply_overrides(index, index_dir, memory_node_limit)?;
            let report = filter_pbf(FilterRunOptions {
                input: pbf_path,
                output,
                spec,
                format_override: format,
                index_options,
            })?;
            if !cli.quiet {
                print_filter_report(&report);
            }
        }
        Commands::Inspect { input } => {
            let report = inspect_path(&input)?;
            println!("{report}");
        }
        Commands::ValidateSpec { filter, spec } => {
            let spec = read_spec_input(spec, filter)?;
            spec.validate()?;
            if !cli.quiet {
                println!("Spec is valid.");
            }
        }
    }

    Ok(())
}

fn read_spec_input(spec: Option<PathBuf>, filter: Option<String>) -> anyhow::Result<FilterSpec> {
    match (spec, filter) {
        (Some(path), None) => Ok(FilterSpec::from_path(&path)?),
        (None, Some(filter)) => Ok(FilterSpec::from_filter_arg(&filter)?),
        (None, None) => Err(anyhow::anyhow!(
            "missing filter input: pass an inline FILTER or use --spec <FILE>"
        )),
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "pass either an inline FILTER or --spec <FILE>, not both"
        )),
    }
}

fn print_filter_report(report: &osmshrink::filter::FilterReport) {
    eprintln!(
        "Wrote {} objects to {} using {} node index",
        report.objects_written,
        report.output.display(),
        report.index_backend.as_str()
    );
    if report.ways_skipped_missing_nodes > 0 {
        eprintln!(
            "Skipped {} ways with missing node coordinates",
            report.ways_skipped_missing_nodes
        );
    }
    if report.relations_skipped_non_area > 0 {
        eprintln!(
            "Skipped {} non-area relations",
            report.relations_skipped_non_area
        );
    }
    if report.relations_skipped_missing_members > 0 {
        eprintln!(
            "Skipped {} relations with missing members",
            report.relations_skipped_missing_members
        );
    }
    if report.relations_skipped_invalid_rings > 0 {
        eprintln!(
            "Skipped {} relations with invalid rings",
            report.relations_skipped_invalid_rings
        );
    }
    if report.relation_members_ignored_role > 0 {
        eprintln!(
            "Ignored {} relation members with unsupported roles or member types",
            report.relation_members_ignored_role
        );
    }
}

fn init_tracing(verbose: u8, quiet: bool) -> anyhow::Result<()> {
    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "warn",
            1 => "info",
            _ => "debug",
        }
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing: {error}"))?;
    Ok(())
}
