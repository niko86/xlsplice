//! Packages for the tests to read and to write, somewhere to keep them, and
//! the comparison that holds the byte-level guarantee.
//!
//! Four fixtures saved from Excel live in `tests/fixtures/`. [`fixture`]
//! names one and [`Workspace::copy_of`] takes a writable copy, because a
//! fixture's bytes are the baseline and nothing may write over them. For the
//! shapes Excel cannot be made to save, a test builds the package it needs
//! here: the parts are written out by hand, so what a test asserts is visible
//! in the test rather than buried in a binary.
//!
//! What every write test asserts comes from [`compare`], which reads two
//! packages part by part with the container crate and is no part of the tool
//! under test: the tool's own `diff` must not be the judge of the tool's own
//! guarantee.
//!
//! The packages this module builds are not fixtures in this repository's
//! sense. A fixture is a package Excel saved; everything built here is
//! scaffolding for a test, thrown away when the test ends.

#![allow(dead_code)]

/// The corpus, and the fixed operation set every package in it is put
/// through.
pub mod corpus;
/// A real Excel, for the one question the tool must not answer about itself.
pub mod oracle;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use xlsplice::answer::Answer;
use xlsplice::render::{OutputMode, Rendered, render};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A directory of one test's files, removed when the test ends.
pub struct Workspace {
    dir: PathBuf,
}

/// Enough to keep two tests running at once out of each other's way.
static NEXT: AtomicU32 = AtomicU32::new(0);

impl Workspace {
    /// A fresh directory, named after the test that asked for it.
    pub fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "xlsplice-{}-{}-{label}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("a test must be able to write a temporary directory");
        Workspace { dir }
    }

    /// Write a file of arbitrary bytes, and give back its path.
    pub fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        File::create(&path)
            .and_then(|mut file| file.write_all(bytes))
            .expect("a test must be able to write its own files");
        path
    }

    /// Write a zip container of `parts`, in the order given, and give back
    /// its path. Deflate is what Excel writes, so it is what the tests read.
    pub fn zip(&self, name: &str, parts: &[(&str, &str)]) -> PathBuf {
        let path = self.dir.join(name);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut writer = ZipWriter::new(
            File::create(&path).expect("a test must be able to write its own packages"),
        );
        for (part, text) in parts {
            writer
                .start_file(*part, options)
                .and_then(|()| writer.write_all(text.as_bytes()).map_err(Into::into))
                .expect("a test must be able to write a part");
        }
        writer.finish().expect("the container must close");
        path
    }

    /// Write a package whose workbook part is `workbook`, with the content
    /// types and root relationships that make it one.
    pub fn package(&self, name: &str, workbook: &str) -> PathBuf {
        self.zip(
            name,
            &[
                (CONTENT_TYPES_PART, CONTENT_TYPES),
                (ROOT_RELS_PART, ROOT_RELS),
                (WORKBOOK_PART, workbook),
            ],
        )
    }

    /// Write the whole feature package: the workbook of [`feature_workbook`]
    /// with the relationships, worksheets and shared strings that make its
    /// sheets readable.
    pub fn feature_package(&self, name: &str) -> PathBuf {
        let workbook = feature_workbook();
        self.zip(
            name,
            &[
                (CONTENT_TYPES_PART, FEATURE_CONTENT_TYPES),
                (ROOT_RELS_PART, ROOT_RELS),
                (WORKBOOK_PART, &workbook),
                (WORKBOOK_RELS_PART, WORKBOOK_RELS),
                (SHEET1_PART, INPUTS_SHEET),
                (SHEET2_PART, NOTES_SHEET),
                (SHEET3_PART, PARAMETERS_SHEET),
                (SHARED_STRINGS_PART, SHARED_STRINGS),
            ],
        )
    }

    /// A package of one visible sheet called Inputs, whose workbook part
    /// carries `properties` before its sheets and whose sheet holds one row
    /// of `cells`.
    ///
    /// For the shapes Excel cannot be made to save, or can only be made to
    /// save by hand: a workbook on the 1904 date system, a cell written as an
    /// empty element. What such a test asserts is visible in the test.
    pub fn sheet_package(&self, name: &str, properties: &str, cells: &str) -> PathBuf {
        let workbook = workbook_xml(&format!(
            r#"{properties}<sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets>"#
        ));
        let sheet = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1" spans="1:3">{cells}</row></sheetData>
</worksheet>"#
        );
        self.zip(
            name,
            &[
                (CONTENT_TYPES_PART, FEATURE_CONTENT_TYPES),
                (ROOT_RELS_PART, ROOT_RELS),
                (WORKBOOK_PART, &workbook),
                (WORKBOOK_RELS_PART, WORKBOOK_RELS),
                (SHEET1_PART, &sheet),
            ],
        )
    }

    /// A package of one sheet holding `cells` and a calc chain holding
    /// `entries`, with the relationship that reaches the chain and its
    /// content-type override.
    ///
    /// No fixture carries a chain short enough to empty, so the packages that
    /// say what happens when one does are written out here: what a test
    /// asserts goes is what the test put there.
    ///
    /// Its declarations name the parts it holds and no others, because this
    /// package is also put in front of a real Excel, and a package naming a
    /// part it does not hold is what Excel offers to repair.
    pub fn chained_package(&self, name: &str, cells: &str, entries: &str) -> PathBuf {
        const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        let sheet = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{NS}"><sheetData><row r="1" spans="1:2">{cells}</row></sheetData></worksheet>"#
        );
        let chain = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<calcChain xmlns="{NS}">{entries}</calcChain>"#
        );
        self.zip(
            name,
            &[
                (CONTENT_TYPES_PART, CHAINED_CONTENT_TYPES),
                (ROOT_RELS_PART, ROOT_RELS),
                (
                    WORKBOOK_PART,
                    &workbook_xml(
                        r#"<sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets>"#,
                    ),
                ),
                (WORKBOOK_RELS_PART, CHAINED_WORKBOOK_RELS),
                (SHEET1_PART, &sheet),
                (CALC_CHAIN_PART, &chain),
            ],
        )
    }

    /// A writable copy of the committed fixture called `name`, so that a test
    /// may write to it without touching the baseline its bytes are.
    pub fn copy_of(&self, name: &str) -> PathBuf {
        self.copy_from(&fixture(name))
    }

    /// A writable copy of the package at `path`, under its own name.
    ///
    /// What a corpus suite writes to: a corpus package is vendor material
    /// that is read and never written, and a workspace is under the
    /// temporary directory, so neither the corpus nor the repository is
    /// anywhere near what a case writes.
    pub fn copy_from(&self, path: &Path) -> PathBuf {
        let name = path
            .file_name()
            .unwrap_or_else(|| panic!("{} must name a file", path.display()));
        let copy = self.dir.join(name);
        fs::copy(path, &copy)
            .unwrap_or_else(|err| panic!("{} must be copyable into a test: {err}", path.display()));
        copy
    }

    /// The directory itself, for a test that needs to name a path in it.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// The committed fixture called `name`, in `tests/fixtures/`.
///
/// A fixture is a package Excel saved and its bytes are the baseline every
/// byte-preservation test compares against, so nothing ever writes here: take
/// a [`Workspace::copy_of`] instead.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

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

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
pub const ROOT_RELS_PART: &str = "_rels/.rels";
pub const WORKBOOK_PART: &str = "xl/workbook.xml";
pub const WORKBOOK_RELS_PART: &str = "xl/_rels/workbook.xml.rels";
pub const SHEET1_PART: &str = "xl/worksheets/sheet1.xml";
pub const SHEET2_PART: &str = "xl/worksheets/sheet2.xml";
pub const SHEET3_PART: &str = "xl/worksheets/sheet3.xml";
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";
pub const CALC_CHAIN_PART: &str = "xl/calcChain.xml";

/// The content types of a [`Workspace::chained_package`]: the three parts it
/// holds, and nothing that is not there.
pub const CHAINED_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/>
</Types>"#;

/// What that package's workbook reaches: its one sheet, and the chain.
pub const CHAINED_WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>
</Relationships>"#;

pub const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"#;

pub const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

/// A workbook part around `body`, namespaced the way Excel writes it.
pub fn workbook_xml(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{body}</workbook>"#
    )
}

/// A workbook part shaped like the feature fixture of issue #4, as far as
/// sheets and defined names go: a visible, a hidden and a very hidden sheet;
/// a workbook-scoped name on a merged range; a sheet-scoped name; a name
/// whose sheet is spelled in another case; and one name each for the three
/// reasons a name resolves to no cell.
pub fn feature_workbook() -> String {
    workbook_xml(
        r#"<sheets>
    <sheet name="Inputs" sheetId="1" r:id="rId1"/>
    <sheet name="Notes" sheetId="2" state="hidden" r:id="rId2"/>
    <sheet name="Parameters" sheetId="3" state="veryHidden" r:id="rId3"/>
  </sheets>
  <definedNames>
    <definedName name="MergedInput">Inputs!$B$2:$C$3</definedName>
    <definedName name="LocalNote" localSheetId="1">Notes!$A$5</definedName>
    <definedName name="LoudCase">INPUTS!$E$1</definedName>
    <definedName name="Gone">#REF!</definedName>
    <definedName name="Rate">0.175</definedName>
    <definedName name="Total">SUM(Inputs!$D$1:$D$9)</definedName>
  </definedNames>"#,
    )
}

/// The content types of the feature package, declaring every part in it.
pub const FEATURE_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#;

/// What the workbook's relationship ids point at: one per sheet, in the order
/// `feature_workbook` declares them, and the shared string table.
pub const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;

/// The Inputs sheet: one cell of every stored type, a plain formula, a shared
/// formula with a child, the anchor of the merged range, a cell holding a
/// style and no value, and a gap where row 4 and row 5 would be.
pub const INPUTS_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <dimension ref="A1:K6"/>
  <sheetData>
    <row r="1" spans="1:11">
      <c r="A1"><v>1</v></c>
      <c r="B1" s="1" t="s"><v>0</v></c>
      <c r="C1" s="2" t="d"><v>2026-09-11T00:00:00</v></c>
      <c r="D1" t="b"><v>1</v></c>
      <c r="E1" t="e"><v>#DIV/0!</v></c>
      <c r="F1" t="str"><f>CONCATENATE("a","b")</f><v>ab</v></c>
      <c r="G1" t="inlineStr"><is><t xml:space="preserve">inline </t></is></c>
      <c r="H1" t="inlineStr"><is><r><t>in</t></r><r><t>line</t></r></is></c>
      <c r="I1" t="s"><v>1</v></c>
      <c r="J1" t="s"><v>3</v></c>
      <c r="K1" t="b"><v>0</v></c>
    </row>
    <row r="2" spans="1:5">
      <c r="A2"><v>2.5</v></c>
      <c r="B2" s="4" t="s"><v>2</v></c>
      <c r="D2"><f>SUM(A1:A5)</f><v>15</v></c>
      <c r="E2"><f t="shared" ref="E2:E3" si="0">A2*2</f><v>5</v></c>
    </row>
    <row r="3" spans="1:5">
      <c r="A3"><v>-3</v></c>
      <c r="E3"><f t="shared" si="0"/><v>-6</v></c>
    </row>
    <row r="6" spans="1:1"><c r="A6" s="5"/></row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="B2:C3"/></mergeCells>
</worksheet>"#;

/// The Notes sheet, holding the cell the sheet-scoped name points at.
pub const NOTES_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="5" spans="1:1"><c r="A5" t="inlineStr"><is><t>note</t></is></c></row>
  </sheetData>
</worksheet>"#;

/// The Parameters sheet, which holds no cells at all.
pub const PARAMETERS_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData/>
</worksheet>"#;

/// The shared string table: a plain string, one built of rich-text runs, the
/// string in the merged range's anchor, and one carrying phonetic text that
/// is no part of its value.
pub const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="5" uniqueCount="4">
  <si><t>hello</t></si>
  <si><r><rPr><b/></rPr><t>rich</t></r><r><t> text</t></r></si>
  <si><t>merged</t></si>
  <si><t>東京</t><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><phoneticPr fontId="1"/></si>
</sst>"#;

/// The verbs, called the way the binary calls them, without a process.
///
/// A test that is not about argv, a real stream or a real exit calls these
/// instead of [`run`]: the same library calls in the same order, answered with
/// the same `Answer`, and rendered by the same `render`. What the binary keeps
/// to itself is the little that only a process has, so what these leave
/// untested is the wiring of one dispatch arm to one verb, which the tests
/// that do spawn still cross.
///
/// `set` takes the write type and the caller's text, which is what an
/// operation carries: reading the one as the other is the batch's, with the
/// workbook open.
pub mod verb {
    use std::path::{Path, PathBuf};

    use xlsplice::Result;
    use xlsplice::answer::{self, Answer};
    use xlsplice::batch::{self, Batch, Destination, Operation, WriteType};
    use xlsplice::cells;
    use xlsplice::package::Package;
    use xlsplice::workbook::Workbook;

    /// Open a package and read its workbook: what every read verb starts
    /// with. The package comes back too, because a verb that reads cells goes
    /// on to read more of its parts.
    fn open(path: &Path) -> Result<(Package, Workbook)> {
        let mut package = Package::open(path)?;
        let workbook = Workbook::read(&mut package)?;
        Ok((package, workbook))
    }

    /// `xlsplice sheets FILE`.
    pub fn sheets(path: &Path) -> Result<Answer> {
        answer::sheets(&open(path)?.1)
    }

    /// `xlsplice names FILE`.
    pub fn names(path: &Path) -> Result<Answer> {
        answer::names(&open(path)?.1)
    }

    /// `xlsplice get FILE TARGET...`.
    pub fn get(path: &Path, targets: &[&str]) -> Result<Answer> {
        let (mut package, workbook) = open(path)?;
        let targets: Vec<String> = targets.iter().map(|target| (*target).to_owned()).collect();
        answer::cells(&cells::read(&mut package, &workbook, &targets)?)
    }

    /// A batch of more than one operation, run the way a writing verb runs
    /// one. The `apply` verb that will carry such a batch from the command
    /// line is #8's; this is the library call it will make.
    pub fn batch(
        path: &Path,
        operations: Vec<Operation>,
        out: Option<PathBuf>,
        dry_run: bool,
    ) -> Result<Answer> {
        let batch = Batch { operations };
        answer::written(&batch::run(path, &batch, &Destination::from(out), dry_run)?)
    }

    /// One `set` operation, for a batch built by [`batch`].
    pub fn writing(target: &str, write_type: WriteType, value: &str) -> Operation {
        Operation::Set {
            target: target.to_owned(),
            write_type,
            value: value.to_owned(),
            replace_formula: false,
        }
    }

    /// `xlsplice clear FILE TARGET [--out PATH] [--dry-run]`.
    pub fn clear(path: &Path, target: &str, out: Option<PathBuf>, dry_run: bool) -> Result<Answer> {
        let batch = Batch::of(Operation::Clear {
            target: target.to_owned(),
            replace_formula: false,
        });
        answer::written(&batch::run(path, &batch, &Destination::from(out), dry_run)?)
    }

    /// One `calc` operation, for a batch built by [`batch`].
    pub fn calculating(full_calc_on_load: bool) -> Operation {
        Operation::Calc { full_calc_on_load }
    }

    /// One `props.set` operation, for a batch built by [`batch`].
    pub fn stamping(name: &str, write_type: WriteType, value: &str) -> Operation {
        Operation::PropsSet {
            name: name.to_owned(),
            write_type,
            value: value.to_owned(),
        }
    }

    /// One `props.unset` operation, for a batch built by [`batch`].
    pub fn unstamping(name: &str) -> Operation {
        Operation::PropsUnset {
            name: name.to_owned(),
        }
    }

    /// One `clear` operation, for a batch built by [`batch`].
    pub fn clearing(target: &str) -> Operation {
        Operation::Clear {
            target: target.to_owned(),
            replace_formula: false,
        }
    }

    /// The same operation, licensed to replace a formula it finds.
    pub fn replacing(operation: Operation) -> Operation {
        match operation {
            Operation::Set {
                target,
                write_type,
                value,
                ..
            } => Operation::Set {
                target,
                write_type,
                value,
                replace_formula: true,
            },
            Operation::Clear { target, .. } => Operation::Clear {
                target,
                replace_formula: true,
            },
            other => other,
        }
    }

    /// `xlsplice set FILE TARGET VALUE --type TYPE [--out PATH] [--dry-run]`.
    pub fn set(
        path: &Path,
        target: &str,
        write_type: WriteType,
        value: &str,
        out: Option<PathBuf>,
        dry_run: bool,
    ) -> Result<Answer> {
        let batch = Batch::of(Operation::Set {
            target: target.to_owned(),
            write_type,
            value: value.to_owned(),
            replace_formula: false,
        });
        answer::written(&batch::run(path, &batch, &Destination::from(out), dry_run)?)
    }
}

/// What a verb writes down a pipe under `--json`: one envelope on stdout, a
/// silent stderr, and the exit code.
pub fn under_json(outcome: xlsplice::Result<Answer>) -> Rendered {
    render(outcome, OutputMode::new(true, false))
}

/// What a verb writes down a pipe without `--json`: tab-separated fields on
/// stdout, or a failure on stderr.
pub fn in_text(outcome: xlsplice::Result<Answer>) -> Rendered {
    render(outcome, OutputMode::new(false, false))
}

/// The JSON envelope a verb put on stdout, parsed: what [`json`] gives for a
/// verb that went round through a process.
pub fn envelope(rendered: &Rendered) -> serde_json::Value {
    serde_json::from_str(&rendered.stdout)
        .unwrap_or_else(|err| panic!("stdout under --json must be one JSON document: {err}"))
}

/// Run the binary with `args` and both streams captured, so stdout is a pipe
/// rather than a terminal.
///
/// `output` gives the child an empty pipe on stdin, which is what a verb
/// reading stdin sees when nothing was piped in: not a terminal, and nothing
/// there.
pub fn run(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .args(args)
        .output()
        .expect("the binary under test must be runnable")
}

/// The same, from inside `dir`, for a test about how an operand is spelled
/// rather than about what it names.
pub fn run_in(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary under test must be runnable")
}

/// The same, with `input` piped to the child's stdin.
pub fn run_with_stdin(args: &[&str], input: &str) -> std::process::Output {
    use std::process::Stdio;

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary under test must be runnable");
    child
        .stdin
        .take()
        .expect("the child was given a pipe on stdin")
        .write_all(input.as_bytes())
        .expect("the child must take what is piped to it");
    child
        .wait_with_output()
        .expect("the child must finish and be waited for")
}

pub fn stdout(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout must be UTF-8")
}

pub fn stderr(out: &std::process::Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr must be UTF-8")
}

pub fn exit_code(out: &std::process::Output) -> i32 {
    out.status
        .code()
        .expect("the binary must exit, not die on a signal")
}

pub fn json(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&stdout(out)).expect("stdout under --json must be one JSON document")
}
