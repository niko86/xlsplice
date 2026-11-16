//! Contract tests for `merged` (#52).
//!
//! A caller reading values laid along a row needs to know which cells are
//! merged, because the value of a merged area sits in its top-left cell and
//! the cells beside it hold nothing. `merged` answers with each sheet's
//! merged areas as the part declares them: one block per sheet named, every
//! area in part order, and nothing about the cells inside.
//!
//! These call the library in process, as `cells.rs` does. One spawns, for
//! what clap does with a sheet missing.

mod support;

use serde_json::json;
use support::binary::{exit_code, run};
use support::container::{SHEET1, SHEET2, WORKBOOK_RELS};
use support::library::{envelope, in_text, targets, under_json};
use support::workspace::{Copied, WORKBOOK_RELS_XML, built};
use xlsplice::verb::{self, Trace};

/// A worksheet part holding one occupied cell and `merges`, which is put
/// where Excel puts `<mergeCells>`: after the sheet data.
fn sheet_with(merges: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
  {merges}
  <pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>
</worksheet>"#
    )
}

/// The feature package with merged areas on two of its three sheets: three
/// on Inputs, spelled as other writers spell them, and one on Notes.
/// Parameters has none, and no `<mergeCells>` element either. Its workbook
/// reaches its worksheets through `rels`.
fn merged_package_reached_by(label: &str, rels: &str) -> Copied {
    built(label, |w| {
        let base = w.feature_package("base.xlsx");
        w.like("merged.xlsx", &base)
            .with_part(WORKBOOK_RELS, rels)
            .with_part(
                SHEET1,
                &sheet_with(
                    r#"<mergeCells count="3"><mergeCell ref="D7:F8"/><mergeCell ref="$g$7:$i$8"/><mergeCell ref="b3:C3"/></mergeCells>"#,
                ),
            )
            .with_part(
                SHEET2,
                &sheet_with(r#"<mergeCells count="1"><mergeCell ref="A10:A12"/></mergeCells>"#),
            )
            .written()
    })
}

/// The same, reached the way Excel writes it: targets beside the workbook
/// part.
fn merged_package(label: &str) -> Copied {
    merged_package_reached_by(label, WORKBOOK_RELS_XML)
}

/// The envelope `merged` answers with for `sheets` of `package`, which must
/// succeed.
fn merged(package: &std::path::Path, sheets: &[&str]) -> serde_json::Value {
    let out = under_json(verb::merged(package, &targets(sheets), &Trace::Off));
    assert_eq!(out.exit, 0, "merged {sheets:?}: {}", out.stdout);
    envelope(&out)
}

/// The envelope of a `merged` that must fail, for the sheets named.
fn failed(package: &std::path::Path, sheets: &[&str]) -> serde_json::Value {
    let out = under_json(verb::merged(package, &targets(sheets), &Trace::Off));
    assert_ne!(out.exit, 0, "merged {sheets:?}: {}", out.stdout);
    envelope(&out)
}

/// One sheet with one `<mergeCells>` holding `merges`, in a package of its
/// own.
fn one_sheet(label: &str, merges: &str) -> Copied {
    built(label, |w| {
        let base = w.feature_package("base.xlsx");
        w.like("one.xlsx", &base)
            .with_part(SHEET1, &sheet_with(merges))
            .written()
    })
}

#[test]
fn every_area_comes_back_in_part_order_upper_cased_and_without_dollars() {
    let package = merged_package("several");

    let body = merged(&package, &["Inputs", "Notes"]);

    assert_eq!(
        body["sheets"],
        json!([
            {"sheet": "Inputs", "areas": ["D7:F8", "G7:I8", "B3:C3"]},
            {"sheet": "Notes", "areas": ["A10:A12"]},
        ])
    );
}

#[test]
fn a_sheet_without_merged_areas_answers_with_none() {
    let package = merged_package("none");

    let body = merged(&package, &["Parameters"]);

    assert_eq!(
        body["sheets"],
        json!([{"sheet": "Parameters", "areas": []}])
    );
}

/// An element declaring no areas is the same answer as no element at all.
#[test]
fn an_empty_merge_cells_element_answers_with_none_too() {
    let package = one_sheet("empty-element", r#"<mergeCells count="0"/>"#);

    let body = merged(&package, &["Inputs"]);

    assert_eq!(body["sheets"][0]["areas"], json!([]));
}

/// Sheets come back one block each, in the order named and in the package's
/// own spelling, and a sheet named twice is answered twice, as `cells`
/// answers it.
#[test]
fn a_sheet_named_twice_is_answered_twice_in_the_packages_spelling() {
    let package = merged_package("twice");

    let body = merged(&package, &["notes", "INPUTS", "Notes"]);

    let sheets: Vec<_> = body["sheets"]
        .as_array()
        .expect("sheets is a list")
        .iter()
        .map(|block| block["sheet"].clone())
        .collect();
    assert_eq!(sheets, [json!("Notes"), json!("Inputs"), json!("Notes")]);
    assert_eq!(body["sheets"][0], body["sheets"][2]);
    assert_eq!(body["sheets"][0]["areas"], json!(["A10:A12"]));
}

/// The rows are one per area, the sheet and the area, so a sheet without
/// any contributes no row and a sheet named twice contributes its rows
/// twice.
#[test]
fn the_rows_are_one_per_area_with_the_sheet_beside_it() {
    let package = merged_package("rows");

    let out = in_text(verb::merged(
        &package,
        &targets(&["Notes", "Parameters", "inputs", "NOTES"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout,
        "Notes\tA10:A12\n\
         Inputs\tD7:F8\n\
         Inputs\tG7:I8\n\
         Inputs\tB3:C3\n\
         Notes\tA10:A12\n"
    );
}

#[test]
fn a_sheet_without_merged_areas_has_no_rows() {
    let package = merged_package("no-rows");

    let out = in_text(verb::merged(
        &package,
        &targets(&["Parameters"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
}

#[test]
fn an_unknown_sheet_is_not_found_and_the_message_lists_the_sheets() {
    let package = merged_package("unknown");

    let body = failed(&package, &["Inputs", "Nowhere"]);

    assert_eq!(body["error"]["code"], json!("not_found"));
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("Nowhere"), "{message}");
    assert!(message.contains("Inputs, Notes, Parameters"), "{message}");
    assert_eq!(body.get("sheets"), None);
}

#[test]
fn an_unknown_sheet_in_text_is_said_on_stderr_and_nothing_is_on_stdout() {
    let package = merged_package("unknown-text");

    let out = in_text(verb::merged(
        &package,
        &targets(&["Inputs", "Nowhere"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 3);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("Nowhere"), "{}", out.stderr);
}

/// What the package names a worksheet by is its relationship, and a target
/// may be written from the package root as readily as from beside the
/// workbook part.
#[test]
fn a_worksheet_reached_by_an_absolute_target_is_read_the_same() {
    let package = merged_package_reached_by(
        "absolute",
        &WORKBOOK_RELS_XML
            .replace(
                r#"Target="worksheets/sheet1.xml""#,
                r#"Target="/xl/worksheets/sheet1.xml""#,
            )
            .replace(
                r#"Target="worksheets/sheet2.xml""#,
                r#"Target="/xl/worksheets/sheet2.xml""#,
            ),
    );

    let body = merged(&package, &["Inputs", "Notes"]);

    assert_eq!(
        body["sheets"],
        json!([
            {"sheet": "Inputs", "areas": ["D7:F8", "G7:I8", "B3:C3"]},
            {"sheet": "Notes", "areas": ["A10:A12"]},
        ])
    );
}

/// A merged area of one cell is what the schema's reference type spells
/// for a rectangle of one cell, as `cells` spells a one-cell extent: it is
/// answered as written rather than refused or widened.
#[test]
fn an_area_of_one_cell_is_answered_as_that_cell() {
    let package = one_sheet(
        "one-cell",
        r#"<mergeCells count="2"><mergeCell ref="e5"/><mergeCell ref="H2:H2"/></mergeCells>"#,
    );

    let body = merged(&package, &["Inputs"]);

    assert_eq!(body["sheets"][0]["areas"], json!(["E5", "H2:H2"]));
}

/// A ref that is not a range of cells is the part disagreeing with itself,
/// and fails the whole read, naming the sheet and the ref.
#[test]
fn a_ref_that_is_not_an_a1_range_is_unreadable_and_named_with_its_sheet() {
    for bad in [
        "C4:", "C:E", "4:9", "C4:E6:F7", "C4-E6", "ZZZZ1:A1", "A0:B2", " C4:E6", "",
    ] {
        let package = one_sheet(
            "bad-ref",
            &format!(
                r#"<mergeCells count="2"><mergeCell ref="A1:B1"/><mergeCell ref="{bad}"/></mergeCells>"#
            ),
        );

        let body = failed(&package, &["inputs"]);

        assert_eq!(body["error"]["code"], json!("unreadable"), "{bad:?}");
        let message = body["error"]["message"].as_str().expect("a message");
        assert!(message.contains("'Inputs'"), "{bad:?}: {message}");
        assert!(message.contains(&format!("'{bad}'")), "{bad:?}: {message}");
        assert_eq!(body.get("sheets"), None);
    }
}

#[test]
fn a_merge_cell_with_no_ref_is_unreadable_and_names_its_sheet() {
    let package = one_sheet(
        "no-ref",
        r#"<mergeCells count="1"><mergeCell/></mergeCells>"#,
    );

    let body = failed(&package, &["Inputs"]);

    assert_eq!(body["error"]["code"], json!("unreadable"));
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("'Inputs'"), "{message}");
}

/// `merged` reads the merged areas and nothing else, so a cell `cells` would
/// refuse as unreadable does not stop it.
#[test]
fn a_cell_that_cannot_be_read_does_not_stop_the_areas_being_read() {
    let package = built("unread-cell", |w| {
        let base = w.feature_package("base.xlsx");
        w.like("unread-cell.xlsx", &base)
            .with_part(
                SHEET1,
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>abc</v></c></row></sheetData>
  <mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells>
</worksheet>"#,
            )
            .written()
    });

    let cells = under_json(verb::cells(&package, &targets(&["Inputs"]), &Trace::Off));
    assert_eq!(envelope(&cells)["error"]["code"], json!("unreadable"));

    let body = merged(&package, &["Inputs"]);
    assert_eq!(body["sheets"][0]["areas"], json!(["A1:B1"]));
}

/// A part a re-serialising tool wrote carries a prefix on every element,
/// and reads the same as one without.
#[test]
fn a_prefixed_worksheet_reads_the_same_as_an_unprefixed_one() {
    let package = built("prefixed", |w| {
        let base = w.feature_package("base.xlsx");
        w.like("prefixed.xlsx", &base)
            .with_part(
                SHEET1,
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetData/><x:mergeCells count="1"><x:mergeCell ref="C2:D2"/></x:mergeCells></x:worksheet>"#,
            )
            .written()
    });

    let body = merged(&package, &["Inputs"]);

    assert_eq!(body["sheets"][0]["areas"], json!(["C2:D2"]));
}

#[test]
fn merged_needs_a_package_and_at_least_one_sheet() {
    let out = run(&["merged", "book.xlsx"]);

    assert_eq!(exit_code(&out), 2);
}
