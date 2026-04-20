use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{OsmshrinkError, Result};

#[derive(Debug, Clone)]
pub struct InspectReport {
    pub path: PathBuf,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub treated_as_pbf: bool,
}

impl fmt::Display for InspectReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "path: {}", self.path.display())?;
        writeln!(formatter, "exists: {}", self.exists)?;
        match self.size_bytes {
            Some(size) => writeln!(formatter, "size: {size} bytes")?,
            None => writeln!(formatter, "size: unavailable")?,
        }
        writeln!(
            formatter,
            "input: {}",
            if self.treated_as_pbf {
                "treated as OSM PBF input"
            } else {
                "not recognized as .osm.pbf"
            }
        )?;
        writeln!(
            formatter,
            "detectable: nodes, ways, and area relations can be filtered"
        )
    }
}

pub fn inspect_path(path: &Path) -> Result<InspectReport> {
    let exists = path.exists();
    let size_bytes = if exists {
        Some(
            fs::metadata(path)
                .map_err(|source| OsmshrinkError::ReadFile {
                    path: path.to_path_buf(),
                    source,
                })?
                .len(),
        )
    } else {
        None
    };
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    Ok(InspectReport {
        path: path.to_path_buf(),
        exists,
        size_bytes,
        treated_as_pbf: filename.ends_with(".osm.pbf") || filename.ends_with(".pbf"),
    })
}
