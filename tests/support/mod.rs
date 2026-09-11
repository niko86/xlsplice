//! Packages for the tests to read, and somewhere to keep them.
//!
//! Issue #4 commits three fixtures saved from Excel. Until they land, and
//! afterwards for the shapes Excel cannot be made to save, a test builds the
//! package it needs here: the parts are written out by hand, so what a test
//! asserts is visible in the test rather than buried in a binary.
//!
//! These are not fixtures in this repository's sense. A fixture is a package
//! Excel saved; everything here is scaffolding for a test, thrown away when
//! the test ends.

#![allow(dead_code)]

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

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

pub const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
pub const ROOT_RELS_PART: &str = "_rels/.rels";
pub const WORKBOOK_PART: &str = "xl/workbook.xml";
pub const WORKBOOK_RELS_PART: &str = "xl/_rels/workbook.xml.rels";
pub const SHEET1_PART: &str = "xl/worksheets/sheet1.xml";
pub const SHEET2_PART: &str = "xl/worksheets/sheet2.xml";
pub const SHEET3_PART: &str = "xl/worksheets/sheet3.xml";
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

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

/// Run the binary with `args` and both streams captured, so stdout is a pipe
/// rather than a terminal.
pub fn run(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .args(args)
        .output()
        .expect("the binary under test must be runnable")
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
