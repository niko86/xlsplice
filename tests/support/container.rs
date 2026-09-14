//! Reading a package back, part by part, and the comparison that holds the
//! byte-level guarantee.
//!
//! What every write test asserts comes from [`compare`], which reads two
//! packages with the container crate and is no part of the tool under test:
//! the tool's own `diff` must not be the judge of the tool's own guarantee.
//!
//! ADR-0001 promises an untouched part is copied raw, so a part is compared on
//! its compressed size, its method and its timestamp as well as on the bytes
//! it decompresses to.
//!
//! The constants here are what a part is *called*. The text a part holds is in
//! [`super::workspace`], which is where a package is built out of it.

// Every suite compiles the whole of this and uses the part of it that suits
// what it is asking about, so what one suite does not reach is not dead.
#![allow(dead_code)]

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

pub const CONTENT_TYPES: &str = "[Content_Types].xml";
pub const ROOT_RELS: &str = "_rels/.rels";
pub const WORKBOOK: &str = "xl/workbook.xml";
pub const WORKBOOK_RELS: &str = "xl/_rels/workbook.xml.rels";
pub const SHEET1: &str = "xl/worksheets/sheet1.xml";
pub const SHEET2: &str = "xl/worksheets/sheet2.xml";
pub const SHEET3: &str = "xl/worksheets/sheet3.xml";
pub const SHARED_STRINGS: &str = "xl/sharedStrings.xml";
pub const CALC_CHAIN: &str = "xl/calcChain.xml";
pub const CUSTOM_PROPERTIES: &str = "docProps/custom.xml";

/// One part of a package, with everything the guarantee is about.
///
/// ADR-0001 promises an untouched part is copied raw, so its compressed size,
/// its method and its timestamp are as much a part of what must not move as
/// the bytes it decompresses to. All of it is compared, so a part quietly
/// recompressed at another level fails even though its contents match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The part's path, which names it.
    pub path: String,
    /// What it decompresses to.
    pub bytes: Vec<u8>,
    /// How it is stored: deflate, for everything Excel writes.
    pub method: String,
    /// How many bytes it takes up stored.
    pub compressed: u64,
    /// The timestamp its entry carries.
    pub modified: String,
}

/// Every part of the package at `path`, in container order.
pub fn parts(path: &Path) -> Vec<Part> {
    let file =
        File::open(path).unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    let mut archive = zip::ZipArchive::new(file)
        .unwrap_or_else(|err| panic!("{} must be a package: {err}", path.display()));
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).expect("a part of the package");
            let described = Part {
                path: entry.name().to_owned(),
                bytes: Vec::new(),
                method: format!("{:?}", entry.compression()),
                compressed: entry.compressed_size(),
                modified: format!("{:?}", entry.last_modified()),
            };
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("a part must be read");
            Part { bytes, ..described }
        })
        .collect()
}

/// One part of the package at `path`.
pub fn part(path: &Path, wanted: &str) -> Part {
    parts(path)
        .into_iter()
        .find(|part| part.path == wanted)
        .unwrap_or_else(|| panic!("{} must hold {wanted}", path.display()))
}

/// The names of the files directly in `dir`, sorted.
pub fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", dir.display()))
        .map(|entry| {
            entry
                .expect("an entry of the directory")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// The moment the entry of one part carries, spelled out.
///
/// A [`Part`] holds its timestamp as the container crate's own debug form,
/// which says whether two parts carry the same moment but not which moment
/// either is. A created part has no entry of its own to take one from, so
/// what it takes instead is asserted in full, here.
pub fn timestamp(path: &Path, wanted: &str) -> String {
    let file =
        File::open(path).unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    let mut archive = zip::ZipArchive::new(file)
        .unwrap_or_else(|err| panic!("{} must be a package: {err}", path.display()));
    let entry = archive
        .by_name(wanted)
        .unwrap_or_else(|err| panic!("{} must hold {wanted}: {err}", path.display()));
    let at = entry
        .last_modified()
        .unwrap_or_else(|| panic!("{wanted} must carry a timestamp"));
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        at.year(),
        at.month(),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// The text of one part of the package at `path`.
pub fn part_text(path: &Path, wanted: &str) -> String {
    String::from_utf8(part(path, wanted).bytes).unwrap_or_else(|_| panic!("{wanted} must be UTF-8"))
}

/// Two packages, compared part by part.
///
/// This is the comparator every write test judges the byte-level guarantee
/// with. It reads both containers itself rather than asking the tool, so a
/// tool that is wrong about what it changed cannot also be the witness.
///
/// No suite names the type — they name the fields of what [`compare`] hands
/// back — but it is what `compare` answers with, so it is as public as the
/// function is.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Parts in both packages, byte for byte the same.
    pub identical: Vec<String>,
    /// Parts in both packages whose bytes differ.
    pub differs: Vec<String>,
    /// Parts the second package has and the first does not.
    pub added: Vec<String>,
    /// Parts the first package has and the second does not.
    pub removed: Vec<String>,
    /// Whether the parts both packages hold are still in the order the first
    /// package held them in.
    pub order_kept: bool,
}

/// Compare the packages at `before` and `after` part by part.
pub fn compare(before: &Path, after: &Path) -> Comparison {
    let (was, now) = (parts(before), parts(after));
    let mut comparison = Comparison {
        order_kept: kept(&was, &now),
        ..Comparison::default()
    };
    for part in &was {
        match now.iter().find(|other| other.path == part.path) {
            None => comparison.removed.push(part.path.clone()),
            Some(other) if other == part => comparison.identical.push(part.path.clone()),
            Some(_) => comparison.differs.push(part.path.clone()),
        }
    }
    for part in &now {
        if !was.iter().any(|other| other.path == part.path) {
            comparison.added.push(part.path.clone());
        }
    }
    comparison
}

/// Whether the parts both packages hold appear in the same relative order.
fn kept(was: &[Part], now: &[Part]) -> bool {
    let shared = |parts: &[Part], other: &[Part]| -> Vec<String> {
        parts
            .iter()
            .map(|part| part.path.clone())
            .filter(|path| other.iter().any(|other| &other.path == path))
            .collect()
    };
    shared(was, now) == shared(now, was)
}

/// Assert that `after` is `before` with exactly the named parts changed: every
/// other part identical, none added, none removed, and the order kept.
///
/// The positive half is the one that matters, so it is asserted as such: these
/// are the parts that must still be there, raw copy and all.
pub fn assert_only_these_differ(before: &Path, after: &Path, expected: &[&str]) {
    let comparison = compare(before, after);
    let untouched: Vec<String> = parts(before)
        .into_iter()
        .map(|part| part.path)
        .filter(|path| !expected.contains(&path.as_str()))
        .collect();

    assert_eq!(comparison.differs, expected, "the wrong parts differ");
    assert_eq!(
        comparison.identical, untouched,
        "a part outside the target did not survive"
    );
    assert_eq!(comparison.added, Vec::<String>::new(), "a part was added");
    assert_eq!(
        comparison.removed,
        Vec::<String>::new(),
        "a part was removed"
    );
    assert!(comparison.order_kept, "the parts were reordered");
}

/// Assert that nothing outside `allowed` moved: every part that differs or
/// was added is one of them, none was removed, the order was kept, and every
/// other part is byte for byte what it was.
///
/// The loose half of [`assert_only_these_differ`], for a package whose parts
/// are not known in advance: a corpus template is not a fixture whose bytes a
/// test can spell out, so what is asserted of it is what the operation was
/// allowed to touch rather than what it did touch. A part inside `allowed`
/// need not have moved, because a write of the value already there moves
/// nothing.
pub fn assert_nothing_outside(before: &Path, after: &Path, allowed: &[String], what: &str) {
    let comparison = compare(before, after);

    for part in comparison.differs.iter().chain(comparison.added.iter()) {
        assert!(
            allowed.contains(part),
            "{what}: {part} moved, and only {allowed:?} may"
        );
    }
    assert_eq!(
        comparison.removed,
        Vec::<String>::new(),
        "{what}: a part was removed"
    );
    assert!(comparison.order_kept, "{what}: the parts were reordered");

    let untouched: Vec<String> = parts(before)
        .into_iter()
        .map(|part| part.path)
        .filter(|path| !comparison.differs.contains(path))
        .collect();
    assert_eq!(
        comparison.identical, untouched,
        "{what}: a part outside the operation did not survive"
    );
}

/// Assert that the two packages hold the same parts, in the same order, with
/// the same bytes.
pub fn assert_same_parts(before: &Path, after: &Path) {
    assert_only_these_differ(before, after, &[]);
}

/// Assert that the two files are the same file, byte for byte.
pub fn assert_same_bytes(before: &Path, after: &Path) {
    let (was, now) = (
        fs::read(before).expect("the first file must be readable"),
        fs::read(after).expect("the second file must be readable"),
    );

    assert_eq!(
        was.len(),
        now.len(),
        "{} is {} bytes and {} is {}",
        before.display(),
        was.len(),
        after.display(),
        now.len()
    );
    assert!(was == now, "the two files differ somewhere in their bytes");
}

/// Assert that `written` is `original` with `before` become `after` and every
/// other byte the byte that was there.
///
/// Both sides of the splice are written out by the test, so what a write is
/// expected to produce is pinned in the test rather than derived from what it
/// produced.
pub fn assert_spliced(original: &str, written: &str, before: &str, after: &str) {
    assert_eq!(
        original.matches(before).count(),
        1,
        "the test's own `before` must name one place in the part, not {}",
        original.matches(before).count()
    );
    assert_eq!(written, original.replacen(before, after, 1));
}
