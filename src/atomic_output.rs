//! Same-directory staging: never truncate the destination before successful completion.
use std::fs;
use std::io;
use std::path::Path;

use tempfile::NamedTempFile;

use crate::error::{OsmshrinkError, Result};

pub(crate) fn create_output(path: &Path) -> Result<NamedTempFile> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|source| write_error(path, source))?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(write_error(
                path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output must be a regular file, not a directory, symbolic link, or device",
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(write_error(path, error)),
    }
    tempfile::Builder::new()
        .prefix(".osmshrink-")
        .suffix(".part")
        .tempfile_in(parent)
        .map_err(|source| write_error(path, source))
}

pub(crate) fn publish_output(temp: NamedTempFile, path: &Path) -> Result<()> {
    temp.as_file()
        .sync_all()
        .map_err(|source| write_error(path, source))?;
    temp.persist(path)
        .map_err(|error| write_error(path, error.error))?;
    Ok(())
}

pub(crate) fn write_error(path: &Path, source: io::Error) -> OsmshrinkError {
    OsmshrinkError::WriteFile {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn staging_is_unique_and_abandoning_it_preserves_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("existing.ndjson");
        fs::write(&output, b"old complete output").unwrap();
        let mut a = create_output(&output).unwrap();
        let b = create_output(&output).unwrap();
        assert_ne!(a.path(), b.path());
        a.write_all(b"incomplete").unwrap();
        let paths = [a.path().to_path_buf(), b.path().to_path_buf()];
        drop((a, b));
        assert!(paths.iter().all(|p| !p.exists()));
        assert_eq!(fs::read(output).unwrap(), b"old complete output");
    }

    #[test]
    fn failed_publication_does_not_remove_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output");
        let temp = create_output(&output).unwrap();
        let temporary_path = temp.path().to_path_buf();
        fs::create_dir(&output).unwrap();
        fs::write(output.join("keep"), b"old").unwrap();
        assert!(publish_output(temp, &output).is_err());
        assert_eq!(fs::read(output.join("keep")).unwrap(), b"old");
        assert!(!temporary_path.exists());
    }
}
