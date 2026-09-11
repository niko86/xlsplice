//! The package layer: a zip container of parts, opened for reading.
//!
//! Nothing here knows what a part means. Its job is to say which parts a
//! package holds, in container order, and to hand one over whole. Container
//! order is kept because the write path must preserve it (ADR-0001), and a
//! part is handed over exactly as stored, byte-order mark and all, because the
//! splice layer will take byte ranges from the same text roxmltree parsed.
//!
//! Every failure here is [`unreadable`](crate::error::ErrorCode::Unreadable)
//! or, for a part the caller named and the package does not have,
//! [`not_found`](crate::error::ErrorCode::NotFound).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::error::{Error, Result};

/// An open package: the zip container, and the paths of its parts in the
/// order the container lists them.
pub struct Package {
    path: PathBuf,
    archive: ZipArchive<BufReader<File>>,
    part_paths: Vec<String>,
}

impl Package {
    /// Open the package at `path`.
    ///
    /// A path that cannot be read, or that is not a zip container, is
    /// `unreadable`: a text file, an empty file and a directory all land here.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|err| {
            Error::unreadable(format!(
                "cannot read {}: {err}. Give the path of an .xlsx or .xlsm package.",
                path.display()
            ))
        })?;
        let archive = ZipArchive::new(BufReader::new(file)).map_err(|err| {
            Error::unreadable(format!(
                "{} is not a package: {err}. An .xlsx or .xlsm package is a zip container.",
                path.display()
            ))
        })?;
        let part_paths = archive.file_names().map(str::to_owned).collect();
        Ok(Package {
            path: path.to_owned(),
            archive,
            part_paths,
        })
    }

    /// The path the package was opened from, for messages.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every part's path, in container order.
    pub fn part_paths(&self) -> &[String] {
        &self.part_paths
    }

    /// Whether the package holds a part at `part`.
    pub fn has_part(&self, part: &str) -> bool {
        self.archive.index_for_name(part).is_some()
    }

    /// The bytes of one part, exactly as stored.
    pub fn read_part(&mut self, part: &str) -> Result<Vec<u8>> {
        let mut entry = self.archive.by_name(part).map_err(|_| {
            Error::not_found(format!(
                "no part '{part}' in {}; the package holds: {}",
                self.path.display(),
                self.part_paths.join(", ")
            ))
        })?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|err| {
            Error::unreadable(format!(
                "{}: part '{part}' cannot be read: {err}",
                self.path.display()
            ))
        })?;
        Ok(bytes)
    }

    /// One part as text. Every part xlsplice parses is UTF-8 XML, so a part
    /// that is not is unreadable rather than lossily converted.
    pub fn read_part_text(&mut self, part: &str) -> Result<String> {
        let bytes = self.read_part(part)?;
        String::from_utf8(bytes).map_err(|_| {
            Error::unreadable(format!(
                "{}: part '{part}' is not UTF-8 text",
                self.path.display()
            ))
        })
    }
}
