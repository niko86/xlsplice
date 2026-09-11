//! The package layer: a zip container of parts, opened for reading and
//! rebuilt for writing.
//!
//! Nothing here knows what a part means. Its job is to say which parts a
//! package holds, in container order, to hand one over whole, and to put a
//! package back together with some of them replaced. Container order is kept
//! because ADR-0001 says the write path must preserve it, and a part is handed
//! over exactly as stored, byte-order mark and all, because the splice layer
//! takes byte ranges from the same text roxmltree parsed.
//!
//! [`Package::rebuild`] copies every part it was not given new bytes for with
//! the container crate's raw copy, which keeps the compressed bytes, the
//! method, the CRC and the timestamp; a replaced part is compressed afresh
//! under the options its own entry carried. The container's own bytes may
//! differ from the original's, which ADR-0001 records as a known deviation.
//!
//! Rebuilding gives back bytes and touches no disk. Putting those bytes
//! somewhere safely is another level altogether, and lives in
//! [`crate::atomic`].
//!
//! Failures here are [`unreadable`](crate::error::ErrorCode::Unreadable), or
//! [`not_found`](crate::error::ErrorCode::NotFound) for a part the caller
//! named and the package does not have.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};

use zip::{ZipArchive, ZipWriter};

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

    /// The bytes of the whole package, with the parts named in `replaced`
    /// carrying their new contents and every other part copied raw.
    ///
    /// Nothing reaches the disk here: the result is the package a caller may
    /// then hand to [`crate::atomic::replace`].
    pub fn rebuild(&mut self, replaced: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
        let file = self.path.clone();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..self.archive.len() {
            let entry = self.archive.by_index(index).map_err(|err| {
                Error::unreadable(format!(
                    "{}: part {index} cannot be read: {err}",
                    file.display()
                ))
            })?;
            let part = entry.name().to_owned();
            let written = match replaced.get(&part) {
                None => writer.raw_copy_file(entry),
                Some(bytes) => {
                    // The entry's own options carry its method, timestamp and
                    // permissions, so a spliced part keeps everything about
                    // its place in the container but its length.
                    let options = entry.options();
                    writer
                        .start_file(&part, options)
                        .and_then(|()| writer.write_all(bytes).map_err(Into::into))
                }
            };
            written.map_err(|err| {
                Error::internal(format!(
                    "{}: part '{part}' cannot be written: {err}",
                    file.display()
                ))
            })?;
        }
        Ok(writer
            .finish()
            .map_err(|err| {
                Error::internal(format!(
                    "{}: the rebuilt container will not close: {err}",
                    file.display()
                ))
            })?
            .into_inner())
    }
}
