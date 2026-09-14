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
//! What a package sits on is the caller's to choose: the container is read
//! through anything that reads and seeks. [`Package::open`] puts one on a
//! file, which is what every verb does; [`Package::of`] puts one on bytes in
//! hand, which is how a test builds the package it means to exercise with no
//! file to write out or clean up. Nothing above this layer changes with the
//! choice: a package over bytes reads, memoises and rebuilds exactly as a
//! package over a file does.
//!
//! Failures here are [`unreadable`](crate::error::ErrorCode::Unreadable), or
//! [`not_found`](crate::error::ErrorCode::NotFound) for a part the caller
//! named and the package does not have.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, Write};
use std::path::Path;

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

/// What a package with no path of its own is called in a message.
const IN_MEMORY: &str = "<memory>";

/// Where a package opened from a file reads its bytes.
pub type FromFile = BufReader<File>;

/// Where a package made from bytes in hand reads them.
pub type FromBytes = Cursor<Vec<u8>>;

/// An open package: the zip container, the paths of its parts in the order
/// the container lists them, and the text of every part read so far.
///
/// `R` is where the container's bytes are: a file, for a package a verb
/// opened, or a [`Cursor`] over bytes, for one a caller had in hand. The
/// container crate reads either the same way, so nothing here is written
/// twice for the two of them.
pub struct Package<R> {
    name: String,
    archive: ZipArchive<R>,
    part_paths: Vec<String>,
    text: BTreeMap<String, String>,
    reads: usize,
}

impl Package<FromFile> {
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
        Package::over(BufReader::new(file), path.display().to_string())
    }
}

impl Package<FromBytes> {
    /// A package over bytes already in hand.
    ///
    /// The bytes are a whole container, the sort [`Package::rebuild`] gives
    /// back, and no file is involved at any point. A caller that has just
    /// rebuilt a package can read the result with this, and a test can build
    /// the package it means to exercise rather than write one out first.
    pub fn of(bytes: Vec<u8>) -> Result<Self> {
        Package::over(Cursor::new(bytes), IN_MEMORY.to_owned())
    }
}

impl<R: Read + Seek> Package<R> {
    /// A package over `source`, called `name` in whatever it has to say.
    fn over(source: R, name: String) -> Result<Self> {
        let archive = ZipArchive::new(source).map_err(|err| {
            Error::unreadable(format!(
                "{name} is not a package: {err}. An .xlsx or .xlsm package is a zip container."
            ))
        })?;
        let part_paths = archive.file_names().map(str::to_owned).collect();
        Ok(Package {
            name,
            archive,
            part_paths,
            text: BTreeMap::new(),
            reads: 0,
        })
    }

    /// What to call the package in a message: the path it was opened from,
    /// or that it was never a file.
    pub fn name(&self) -> &str {
        &self.name
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
                self.name,
                self.part_paths.join(", ")
            ))
        })?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|err| {
            Error::unreadable(format!(
                "{}: part '{part}' cannot be read: {err}",
                self.name
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
                Error::unreadable(format!("{}: part '{part}' is not UTF-8 text", self.name))
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
        let file = self.name.clone();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..self.archive.len() {
            let entry = self.archive.by_index(index).map_err(|err| {
                Error::unreadable(format!("{file}: part {index} cannot be read: {err}"))
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
                Error::internal(format!("{file}: part '{part}' cannot be written: {err}"))
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
                    Error::internal(format!("{file}: part '{part}' cannot be created: {err}"))
                })?;
        }
        Ok(writer
            .finish()
            .map_err(|err| {
                Error::internal(format!(
                    "{file}: the rebuilt container will not close: {err}"
                ))
            })?
            .into_inner())
    }
}

/// The bytes of a container holding `parts`, in the order given.
///
/// This is what a test hands [`Package::of`] when the package is what it
/// means to exercise: the parts it cares about and nothing else, built where
/// they are read and gone when the test is.
#[cfg(test)]
pub(crate) fn container_of(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (part, text) in parts {
        writer
            .start_file(*part, SimpleFileOptions::default())
            .expect("a part must start");
        writer
            .write_all(text.as_bytes())
            .expect("a part must be written");
    }
    writer
        .finish()
        .expect("the container must close")
        .into_inner()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The three parts these tests are about, in an order nobody would have
    /// sorted them into, because container order is a promise.
    const PARTS: [(&str, &str); 3] = [
        ("xl/workbook.xml", "<workbook/>"),
        ("[Content_Types].xml", "<Types/>"),
        ("xl/worksheets/sheet1.xml", "<worksheet/>"),
    ];

    /// A package over those parts, built where it is read.
    fn package() -> Package<FromBytes> {
        Package::of(container_of(&PARTS)).expect("the bytes are a container")
    }

    /// What a rebuilt package holds, in the order it holds it.
    fn parts_of(bytes: Vec<u8>) -> Vec<(String, String)> {
        let mut rebuilt = Package::of(bytes).expect("a rebuilt package is a package");
        rebuilt
            .part_paths()
            .to_vec()
            .into_iter()
            .map(|part| {
                let text = rebuilt
                    .read_part_text(&part)
                    .expect("a part it lists is a part it holds")
                    .to_owned();
                (part, text)
            })
            .collect()
    }

    #[test]
    fn a_package_lists_its_parts_in_the_order_the_container_holds_them() {
        assert_eq!(
            package().part_paths(),
            [
                "xl/workbook.xml",
                "[Content_Types].xml",
                "xl/worksheets/sheet1.xml"
            ],
            "container order, not path order"
        );
    }

    #[test]
    fn a_package_holds_the_parts_it_lists_and_no_others() {
        let package = package();

        assert!(package.has_part("xl/workbook.xml"));
        assert!(!package.has_part("xl/styles.xml"));
        assert!(
            !package.has_part("xl/workbook.XML"),
            "a part path is compared against the container's own listing, which is exact"
        );
    }

    #[test]
    fn a_part_comes_back_exactly_as_it_was_stored() {
        let mut package = package();

        assert_eq!(
            package
                .read_part("xl/workbook.xml")
                .expect("the package holds it"),
            b"<workbook/>"
        );
    }

    /// The memo ADR-0005 rests on: an operation owns its own reading, and the
    /// second operation to want a part pays nothing for it.
    #[test]
    fn a_part_read_as_text_twice_is_taken_out_of_the_container_once() {
        let mut package = package();

        let first = package
            .read_part_text("xl/workbook.xml")
            .expect("the package holds it")
            .to_owned();
        assert_eq!(package.reads(), 1);
        let again = package
            .read_part_text("xl/workbook.xml")
            .expect("the package holds it");

        assert_eq!(first, again);
        assert_eq!(
            package.reads(),
            1,
            "the second asking is the first answer handed back"
        );
    }

    #[test]
    fn two_parts_read_as_text_are_two_reads() {
        let mut package = package();

        package.read_part_text("xl/workbook.xml").expect("held");
        package.read_part_text("[Content_Types].xml").expect("held");

        assert_eq!(package.reads(), 2, "the memo is per part, not a lid on all");
    }

    #[test]
    fn a_part_the_package_does_not_hold_is_not_found_and_says_what_it_holds() {
        let mut package = package();

        let error = package
            .read_part("xl/styles.xml")
            .expect_err("the package holds no styles");

        assert_eq!(error.code(), crate::ErrorCode::NotFound);
        assert!(
            error.message().contains("xl/worksheets/sheet1.xml"),
            "the message lists the parts there are: {}",
            error.message()
        );
    }

    #[test]
    fn a_part_that_is_not_utf8_text_is_unreadable() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("xl/media/image1.png", SimpleFileOptions::default())
            .expect("a part must start");
        writer
            .write_all(&[0x89, 0x50, 0x4e, 0x47, 0xff])
            .expect("a part must be written");
        let bytes = writer
            .finish()
            .expect("the container must close")
            .into_inner();
        let mut package = Package::of(bytes).expect("the bytes are a container");

        assert!(
            package.read_part("xl/media/image1.png").is_ok(),
            "as bytes it is just a part"
        );
        let error = package
            .read_part_text("xl/media/image1.png")
            .expect_err("as text it is not one");

        assert_eq!(error.code(), crate::ErrorCode::Unreadable);
    }

    #[test]
    fn bytes_that_are_not_a_container_are_unreadable() {
        let Err(error) = Package::of(b"this is not a package".to_vec()) else {
            panic!("a package is a zip container");
        };

        assert_eq!(error.code(), crate::ErrorCode::Unreadable);
    }

    /// A package with no path of its own still has to be nameable, because
    /// every message this layer writes names the package it is about.
    #[test]
    fn a_package_over_bytes_is_named_for_having_no_file() {
        let mut package = package();

        assert_eq!(package.name(), "<memory>");
        assert!(
            package
                .read_part("nowhere.xml")
                .expect_err("no such part")
                .message()
                .contains("<memory>")
        );
    }

    #[test]
    fn a_rebuild_of_nothing_holds_every_part_the_package_held() {
        let mut package = package();

        let bytes = package
            .rebuild(&BTreeMap::new())
            .expect("a package rebuilds");

        assert_eq!(
            parts_of(bytes),
            PARTS.map(|(part, text)| (part.to_owned(), text.to_owned())),
            "every part, in the order it was in"
        );
    }

    #[test]
    fn a_replaced_part_carries_the_new_bytes_and_keeps_its_place() {
        let mut package = package();
        let content = BTreeMap::from([(
            "[Content_Types].xml".to_owned(),
            Content::Bytes(b"<Types count=\"1\"/>".to_vec()),
        )]);

        let bytes = package.rebuild(&content).expect("a package rebuilds");

        assert_eq!(
            parts_of(bytes),
            [
                ("xl/workbook.xml".to_owned(), "<workbook/>".to_owned()),
                (
                    "[Content_Types].xml".to_owned(),
                    "<Types count=\"1\"/>".to_owned()
                ),
                (
                    "xl/worksheets/sheet1.xml".to_owned(),
                    "<worksheet/>".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn a_part_that_is_gone_is_left_out_and_the_rest_keep_their_order() {
        let mut package = package();
        let content = BTreeMap::from([("[Content_Types].xml".to_owned(), Content::Gone)]);

        let bytes = package.rebuild(&content).expect("a package rebuilds");

        assert_eq!(
            parts_of(bytes),
            [
                ("xl/workbook.xml".to_owned(), "<workbook/>".to_owned()),
                (
                    "xl/worksheets/sheet1.xml".to_owned(),
                    "<worksheet/>".to_owned()
                ),
            ]
        );
    }

    /// A created part has no place of its own in the container, so it goes
    /// after the ones that have, in path order, and a batch run twice
    /// produces the same bytes.
    #[test]
    fn parts_the_package_does_not_hold_are_created_after_the_ones_it_does() {
        let (mut once, mut twice) = (package(), package());
        let content = BTreeMap::from([
            (
                "docProps/custom.xml".to_owned(),
                Content::Bytes(b"<Properties/>".to_vec()),
            ),
            (
                "_rels/.rels".to_owned(),
                Content::Bytes(b"<Relationships/>".to_vec()),
            ),
        ]);

        let bytes = once.rebuild(&content).expect("a package rebuilds");

        assert_eq!(
            parts_of(bytes.clone())
                .into_iter()
                .map(|(part, _)| part)
                .collect::<Vec<_>>(),
            [
                "xl/workbook.xml",
                "[Content_Types].xml",
                "xl/worksheets/sheet1.xml",
                "_rels/.rels",
                "docProps/custom.xml",
            ]
        );
        assert_eq!(
            bytes,
            twice.rebuild(&content).expect("a package rebuilds"),
            "the same batch twice is the same bytes"
        );
    }

    #[test]
    fn rebuilding_asks_the_container_for_nothing_it_has_already_read() {
        let mut package = package();
        package.read_part_text("xl/workbook.xml").expect("held");
        let before = package.reads();

        package
            .rebuild(&BTreeMap::new())
            .expect("a package rebuilds");

        assert_eq!(
            package.reads(),
            before,
            "a raw copy is not a read: the parts go across compressed"
        );
    }
}
