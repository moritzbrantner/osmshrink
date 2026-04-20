use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::{OsmshrinkError, Result};
use crate::model::Feature;
use crate::spec::OutputFormat;

pub struct OutputWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    format: OutputFormat,
    first_json_item: bool,
    fields: Vec<crate::spec::OutputField>,
}

impl OutputWriter {
    pub fn create(path: &Path, format: OutputFormat) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| OsmshrinkError::WriteFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let file = File::create(path).map_err(|source| OsmshrinkError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;
        let mut writer = BufWriter::new(file);
        if format == OutputFormat::Json {
            writer
                .write_all(b"[\n")
                .map_err(|source| OsmshrinkError::WriteFile {
                    path: path.to_path_buf(),
                    source,
                })?;
        }

        Ok(Self {
            path: path.to_path_buf(),
            writer,
            format,
            first_json_item: true,
            fields: crate::spec::OutputField::defaults(),
        })
    }

    pub fn set_fields(&mut self, fields: Vec<crate::spec::OutputField>) {
        self.fields = fields;
    }

    pub fn write_feature(&mut self, feature: &Feature) -> Result<()> {
        let value = feature.to_value_with_fields(&self.fields);

        match self.format {
            OutputFormat::Ndjson => {
                serde_json::to_writer(&mut self.writer, &value).map_err(|source| {
                    OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source: std::io::Error::new(std::io::ErrorKind::Other, source),
                    }
                })?;
                self.writer
                    .write_all(b"\n")
                    .map_err(|source| OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source,
                    })?;
            }
            OutputFormat::Json => {
                if !self.first_json_item {
                    self.writer
                        .write_all(b",\n")
                        .map_err(|source| OsmshrinkError::WriteFile {
                            path: self.path.clone(),
                            source,
                        })?;
                }
                serde_json::to_writer_pretty(&mut self.writer, &value).map_err(|source| {
                    OsmshrinkError::WriteFile {
                        path: self.path.clone(),
                        source: std::io::Error::new(std::io::ErrorKind::Other, source),
                    }
                })?;
                self.first_json_item = false;
            }
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        if self.format == OutputFormat::Json {
            self.writer
                .write_all(b"\n]\n")
                .map_err(|source| OsmshrinkError::WriteFile {
                    path: self.path.clone(),
                    source,
                })?;
        }
        self.writer
            .flush()
            .map_err(|source| OsmshrinkError::WriteFile {
                path: self.path,
                source,
            })
    }
}
