//! Somewhere to put a package, and the packages a test builds itself.
//!
//! Four fixtures saved from Excel live in `tests/fixtures/`. [`fixture`] names
//! one and [`copy_of`] takes a writable copy, because a fixture's bytes are
//! the baseline and nothing may write over them. For the shapes Excel cannot
//! be made to save, a test builds the package it needs here: the parts are
//! written out by hand, so what a test asserts is visible in the test rather
//! than buried in a binary.
//!
//! The packages this module builds are not fixtures in this repository's
//! sense. A fixture is a package Excel saved; everything built here is
//! scaffolding for a test, thrown away when the test ends.
//!
//! The constants are the *text* of a part. What a part is *called* is in
//! [`super::container`], which is where a part is named to be read back.

// Every suite compiles the whole of this and uses the part of it that suits
// what it is asking about, so what one suite does not reach is not dead.
#![allow(dead_code)]

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::container::{
    CALC_CHAIN, CONTENT_TYPES, ROOT_RELS, SHARED_STRINGS, SHEET1, SHEET2, SHEET3, WORKBOOK,
    WORKBOOK_RELS,
};

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
                (CONTENT_TYPES, CONTENT_TYPES_XML),
                (ROOT_RELS, ROOT_RELS_XML),
                (WORKBOOK, workbook),
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
                (CONTENT_TYPES, FEATURE_CONTENT_TYPES_XML),
                (ROOT_RELS, ROOT_RELS_XML),
                (WORKBOOK, &workbook),
                (WORKBOOK_RELS, WORKBOOK_RELS_XML),
                (SHEET1, INPUTS_SHEET_XML),
                (SHEET2, NOTES_SHEET_XML),
                (SHEET3, PARAMETERS_SHEET_XML),
                (SHARED_STRINGS, SHARED_STRINGS_XML),
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
                (CONTENT_TYPES, FEATURE_CONTENT_TYPES_XML),
                (ROOT_RELS, ROOT_RELS_XML),
                (WORKBOOK, &workbook),
                (WORKBOOK_RELS, WORKBOOK_RELS_XML),
                (SHEET1, &sheet),
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
                (CONTENT_TYPES, CHAINED_CONTENT_TYPES_XML),
                (ROOT_RELS, ROOT_RELS_XML),
                (
                    WORKBOOK,
                    &workbook_xml(
                        r#"<sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets>"#,
                    ),
                ),
                (WORKBOOK_RELS, CHAINED_WORKBOOK_RELS_XML),
                (SHEET1, &sheet),
                (CALC_CHAIN, &chain),
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

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A writable copy of a package, and the directory holding it, as one value.
///
/// The directory goes when this does, which is the whole point. A copy that
/// hands back a bare path leaves the removal to whoever still holds the
/// workspace, so every suite invented the same tuple and every test carried a
/// binding whose only job was to stay alive — and a test that wrote
/// `copy(..).1` deleted the package before its own assertion ran. Here the
/// value a test uses is the value that owns the directory: nothing to
/// remember, and nothing to hold wrongly.
///
/// It stands in for the path it is. It derefs to one, and it is `AsRef<Path>`
/// and `AsRef<OsStr>` besides, for the generic callers — a `Command`
/// argument, say — that deref coercion does not reach.
pub struct Copied {
    workspace: Workspace,
    path: PathBuf,
}

impl Copied {
    /// The workspace the copy lives in, for a test that wants a second file
    /// beside it.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// A path beside this one, in the same directory and with the same
    /// lifetime — what a test comparing two packages writes its second to.
    pub fn beside(&self, name: &str) -> PathBuf {
        self.workspace.dir().join(name)
    }
}

impl std::ops::Deref for Copied {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Copied {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<std::ffi::OsStr> for Copied {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.path.as_os_str()
    }
}

/// A writable copy of the committed fixture called `name`, in a workspace of
/// its own named after the test that asked.
pub fn copy_of(label: &str, name: &str) -> Copied {
    copy_from(label, &fixture(name))
}

/// The same for a package that is not a fixture: what a corpus suite writes
/// to, a corpus package being vendor material that is read and never written.
pub fn copy_from(label: &str, path: &Path) -> Copied {
    let workspace = Workspace::new(label);
    let path = workspace.copy_from(path);
    Copied { workspace, path }
}

/// A package built in a workspace of its own, owning both.
///
/// The copies above start from bytes Excel saved; this starts from bytes a
/// test writes, which is what the shapes no fixture carries are made of. What
/// it answers is the same value, so a built package is no more to hold than a
/// copied one.
pub fn built(label: &str, build: impl FnOnce(&Workspace) -> PathBuf) -> Copied {
    let workspace = Workspace::new(label);
    let path = build(&workspace);
    Copied { workspace, path }
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

/// The content types of a [`Workspace::chained_package`]: the three parts it
/// holds, and nothing that is not there.
const CHAINED_CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/>
</Types>"#;

/// What that package's workbook reaches: its one sheet, and the chain.
const CHAINED_WORKBOOK_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>
</Relationships>"#;

pub const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"#;

pub const ROOT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
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
pub const FEATURE_CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
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
pub const WORKBOOK_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;

/// The Inputs sheet: one cell of every stored type, a plain formula, a shared
/// formula with a child, the anchor of the merged range, a cell holding a
/// style and no value, and a gap where row 4 and row 5 would be.
pub const INPUTS_SHEET_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
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
pub const NOTES_SHEET_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="5" spans="1:1"><c r="A5" t="inlineStr"><is><t>note</t></is></c></row>
  </sheetData>
</worksheet>"#;

/// The Parameters sheet, which holds no cells at all.
const PARAMETERS_SHEET_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData/>
</worksheet>"#;

/// The shared string table: a plain string, one built of rich-text runs, the
/// string in the merged range's anchor, and one carrying phonetic text that
/// is no part of its value.
pub const SHARED_STRINGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="5" uniqueCount="4">
  <si><t>hello</t></si>
  <si><r><rPr><b/></rPr><t>rich</t></r><r><t> text</t></r></si>
  <si><t>merged</t></si>
  <si><t>東京</t><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><phoneticPr fontId="1"/></si>
</sst>"#;
