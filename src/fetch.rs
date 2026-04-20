use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::AsyncWriteExt;

use crate::error::{OsmshrinkError, Result};
use crate::geofabrik::resolve_source;

#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub show_progress: bool,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct FetchReport {
    pub url: String,
    pub output: PathBuf,
    pub bytes_written: u64,
    pub cached: bool,
}

pub async fn download_source(
    source: &str,
    output: &Path,
    options: &FetchOptions,
) -> Result<FetchReport> {
    validate_pbf_output_path(output)?;
    let url = resolve_source(source)?;
    download_url(&url, output, options).await
}

pub async fn download_url(url: &str, output: &Path, options: &FetchOptions) -> Result<FetchReport> {
    validate_pbf_output_path(output)?;
    if !options.force {
        if let Some(report) = cached_report(url, output).await? {
            return Ok(report);
        }
    }

    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| OsmshrinkError::WriteFile {
                path: parent.to_path_buf(),
                source,
            })?;
    }

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|source| OsmshrinkError::Download {
            url: url.to_owned(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(OsmshrinkError::HttpStatus {
            url: url.to_owned(),
            status,
        });
    }

    let tmp_path = partial_path(output);
    let mut tmp_file =
        tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|source| OsmshrinkError::WriteFile {
                path: tmp_path.clone(),
                source,
            })?;

    let total = response.content_length();
    let progress = progress_bar(total, options.show_progress);
    let mut bytes_written = 0_u64;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(source) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(OsmshrinkError::Download {
                    url: url.to_owned(),
                    source,
                });
            }
        };
        tmp_file
            .write_all(&chunk)
            .await
            .map_err(|source| OsmshrinkError::WriteFile {
                path: tmp_path.clone(),
                source,
            })?;
        bytes_written += chunk.len() as u64;
        if let Some(progress) = &progress {
            progress.inc(chunk.len() as u64);
        }
    }

    tmp_file
        .flush()
        .await
        .map_err(|source| OsmshrinkError::WriteFile {
            path: tmp_path.clone(),
            source,
        })?;
    drop(tmp_file);

    if output.exists() {
        tokio::fs::remove_file(output)
            .await
            .map_err(|source| OsmshrinkError::WriteFile {
                path: output.to_path_buf(),
                source,
            })?;
    }
    tokio::fs::rename(&tmp_path, output)
        .await
        .map_err(|source| OsmshrinkError::WriteFile {
            path: output.to_path_buf(),
            source,
        })?;

    if let Some(progress) = progress {
        progress.finish_and_clear();
    }

    Ok(FetchReport {
        url: url.to_owned(),
        output: output.to_path_buf(),
        bytes_written,
        cached: false,
    })
}

async fn cached_report(url: &str, output: &Path) -> Result<Option<FetchReport>> {
    let metadata = match tokio::fs::metadata(output).await {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(OsmshrinkError::ReadFile {
                path: output.to_path_buf(),
                source,
            });
        }
    };

    if !metadata.is_file() {
        return Ok(None);
    }

    Ok(Some(FetchReport {
        url: url.to_owned(),
        output: output.to_path_buf(),
        bytes_written: metadata.len(),
        cached: true,
    }))
}

fn progress_bar(total: Option<u64>, show: bool) -> Option<ProgressBar> {
    if !show {
        return None;
    }

    let progress = match total {
        Some(total) => ProgressBar::new(total),
        None => ProgressBar::new_spinner(),
    };
    let style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar());
    progress.set_style(style);
    Some(progress)
}

fn partial_path(output: &Path) -> PathBuf {
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.osm.pbf");
    output.with_file_name(format!("{file_name}.part"))
}

fn validate_pbf_output_path(path: &Path) -> Result<()> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if filename.ends_with(".osm.pbf") || filename.ends_with(".pbf") {
        Ok(())
    } else {
        Err(OsmshrinkError::UnsupportedOutputFile {
            path: path.to_path_buf(),
            expected: ".osm.pbf or .pbf",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_pbf_output_extensions() {
        assert!(validate_pbf_output_path(Path::new("extract.osm.pbf")).is_ok());
        assert!(validate_pbf_output_path(Path::new("extract.pbf")).is_ok());
        assert!(validate_pbf_output_path(Path::new("extract.osm")).is_err());
    }

    #[test]
    fn derives_partial_path_next_to_output() {
        assert_eq!(
            partial_path(Path::new("data/extract.osm.pbf")),
            PathBuf::from("data/extract.osm.pbf.part")
        );
    }

    #[tokio::test]
    async fn returns_cached_report_for_existing_output() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output = temp_dir.path().join("cached.osm.pbf");
        tokio::fs::write(&output, b"already here").await.unwrap();

        let report = download_url(
            "http://127.0.0.1:9/never-requested.osm.pbf",
            &output,
            &FetchOptions {
                show_progress: false,
                force: false,
            },
        )
        .await
        .unwrap();

        assert!(report.cached);
        assert_eq!(report.bytes_written, 12);
        assert_eq!(tokio::fs::read(&output).await.unwrap(), b"already here");
    }
}
