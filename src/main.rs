use clap::Parser;
use osmshrink::cli::{Cli, Commands};
use osmshrink::fetch::{FetchOptions, download_source};
use osmshrink::filter::{FilterRunOptions, filter_pbf};
use osmshrink::inspect::inspect_path;
use osmshrink::spec::FilterSpec;
use tempfile::tempdir;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet)?;

    match cli.command {
        Commands::Fetch { source, output } => {
            let options = FetchOptions {
                show_progress: !cli.quiet,
            };
            let report = download_source(&source, &output, &options).await?;
            if !cli.quiet {
                eprintln!(
                    "Downloaded {} bytes from {} to {}",
                    report.bytes_written,
                    report.url,
                    report.output.display()
                );
            }
        }
        Commands::Filter {
            input,
            spec,
            output,
            format,
        } => {
            let spec = FilterSpec::from_path(&spec)?;
            let report = filter_pbf(FilterRunOptions {
                input,
                output,
                spec,
                format_override: format,
            })?;
            if !cli.quiet {
                eprintln!(
                    "Wrote {} objects to {}",
                    report.objects_written,
                    report.output.display()
                );
            }
        }
        Commands::Run {
            source,
            spec,
            output,
            format,
        } => {
            let temp_dir = tempdir()?;
            let pbf_path = temp_dir.path().join("source.osm.pbf");
            let fetch_report = download_source(
                &source,
                &pbf_path,
                &FetchOptions {
                    show_progress: !cli.quiet,
                },
            )
            .await?;
            if !cli.quiet {
                eprintln!(
                    "Downloaded {} bytes from {}",
                    fetch_report.bytes_written, fetch_report.url
                );
            }

            let spec = FilterSpec::from_path(&spec)?;
            let report = filter_pbf(FilterRunOptions {
                input: pbf_path,
                output,
                spec,
                format_override: format,
            })?;
            if !cli.quiet {
                eprintln!(
                    "Wrote {} objects to {}",
                    report.objects_written,
                    report.output.display()
                );
            }
        }
        Commands::Inspect { input } => {
            let report = inspect_path(&input)?;
            println!("{report}");
        }
        Commands::ValidateSpec { spec } => {
            let spec = FilterSpec::from_path(&spec)?;
            spec.validate()?;
            if !cli.quiet {
                println!("Spec is valid.");
            }
        }
    }

    Ok(())
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
