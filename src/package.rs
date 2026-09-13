//! The package layer: a zip container of parts, opened for reading and
//! rebuilt for writing.
//!
//! Nothing here knows what a part means. Its job is to say which parts a
//! package holds, in container order, to hand one over whole, and to put a
//! package back together with some of them changed. Container order is kept
//! because ADR-0001 says the write path must preserve it, and a part is handed
//! over exactly as stored, byte-order mark and all, because the splice layer
//! takes byte ranges from the same text roxmltree parsed.
//!
//! A part read as text is memoised, so an operation reading a part another
//! operation has already read costs nothing (ADR-0005). What is kept is the
//! decompressed text; the container is never asked for the same part twice.
//!
//! [`Package::rebuild`] copies every part it was given no new [`Content`] for
//! with the container crate's raw copy, which keeps the compressed bytes, the
//! method, the CRC and the timestamp; a replaced part is compressed afresh
//! under the options its own entry carried. A part the container does not
//! hold is created after the ones it does, at the zip epoch, because a new
//! entry has no stamp of its own to keep and a batch run twice must produce
//! the same bytes. A part given [`Content::Gone`] is left out. The
//! container's own bytes may differ from the original's, which ADR-0001
//! records as a known deviation.
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

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{Error, Result};

/// What a rebuild is to make of one part.
///
/// This is a part edit with the meaning taken out of it: by the time an edit
/// reaches the container, a splice has been applied and what is left is the
/// bytes the part is to hold, or nothing at all. Which of the two a byte edit
/// is, a replacement or a creation, is not something the caller has to say:
/// the container knows what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The part holds these bytes.
    Bytes(Vec<u8>),
    /// The part is left out of the rebuilt container.
    Gone,
}

/// An open package: the zip container, the paths of its parts in the order
/// the container lists them, and the text of every part read so far.
pub struct Package {
    path: PathBuf,
    archive: ZipArchive<BufReader<File>>,
    part_paths: Vec<String>,
    text: BTreeMap<String, String>,
    reads: usize,
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
            text: BTreeMap::new(),
            reads: 0,
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

    /// How many parts have been taken out of the container.
    ///
    /// A part read twice as text is decompressed once, so this counts parts
    /// rather than askings. It is here so that the memo is observable: that a
    /// part two operations both read is read once is a promise, and this is
    /// what lets a test hold the tool to it.
    pub fn reads(&self) -> usize {
        self.reads
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
        self.reads += 1;
        Ok(bytes)
    }

    /// One part as text. Every part xlsplice parses is UTF-8 XML, so a part
    /// that is not is unreadable rather than lossily converted.
    ///
    /// The text is kept, so a part asked for again is the same text handed
    /// back rather than the container asked twice. The text of a part is what
    /// a splice's byte ranges are taken from, so two callers reading one part
    /// are reading one string and their ranges mean the same thing.
    pub fn read_part_text(&mut self, part: &str) -> Result<&str> {
        if !self.text.contains_key(part) {
            let bytes = self.read_part(part)?;
            let text = String::from_utf8(bytes).map_err(|_| {
                Error::unreadable(format!(
                    "{}: part '{part}' is not UTF-8 text",
                    self.path.display()
                ))
            })?;
            self.text.insert(part.to_owned(), text);
        }
        Ok(&self.text[part])
    }

    /// The bytes of the whole package, with the parts named in `content`
    /// carrying what it says and every other part copied raw.
    ///
    /// A part the container already holds keeps its place in it; one it does
    /// not is created after them all, in path order, so that a batch run
    /// twice produces the same bytes. Nothing reaches the disk here: the
    /// result is the package a caller may then hand to
    /// [`crate::atomic::replace`].
    pub fn rebuild(&mut self, content: &BTreeMap<String, Content>) -> Result<Vec<u8>> {
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
            let written = match content.get(&part) {
                None => writer.raw_copy_file(entry),
                Some(Content::Gone) => continue,
                Some(Content::Bytes(bytes)) => {
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
        for (part, holds) in content {
            let Content::Bytes(bytes) = holds else {
                continue;
            };
            if self.has_part(part) {
                continue;
            }
            writer
                .start_file(part, created_options())
                .and_then(|()| writer.write_all(bytes).map_err(Into::into))
                .map_err(|err| {
                    Error::internal(format!(
                        "{}: part '{part}' cannot be created: {err}",
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

/// How a created part is stored.
///
/// Deflate is what Excel writes, and the zip epoch of 1980-01-01 is the one
/// stamp that is not the moment the run happened: a created part has no entry
/// of its own to keep the stamp of, and a batch run twice must produce the
/// same bytes.
fn created_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}
