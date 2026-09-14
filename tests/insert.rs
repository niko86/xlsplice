//! Contract tests for putting a cell that is not there into a sheet.
//!
//! A hydration writes into a template, and a template does not carry an
//! element for every cell it will one day hold: Excel writes a cell only once
//! something is in it. So a write to a cell the part does not hold puts it
//! there, and a write to a row the part does not hold puts the row there too.
//!
//! Every assertion here is the same shape: the part afterwards is the part
//! before it with exactly one substitution and no other byte moved, which is
//! what says the dimension element and the rows' spans were left alone.

mod support;

use std::path::PathBuf;

use serde_json::json;
use support::{
    Workspace, assert_only_these_differ, assert_same_bytes, assert_spliced, copy_of, envelope,
    exit_code, fixture, json, part_text, run, stderr, under_json, verb,
};
use xlsplice::batch::WriteType;

const SHEET1: &str = "xl/worksheets/sheet1.xml";

/// Write a number into `target` and give back the worksheet part as it was
/// left, having asserted the write landed.
fn writing(package: &std::path::Path, target: &str) -> String {
    let out = under_json(verb::set(
        package,
        target,
        WriteType::Number,
        "9",
        None,
        false,
    ));
    assert_eq!(out.exit, 0, "{target}: {}", out.stdout);
    assert_eq!(
        envelope(&out)["operations"][0]["changed"],
        json!(true),
        "{target}"
    );
    part_text(package, SHEET1)
}

/// A package of one sheet whose worksheet part is `sheet`, for the shapes no
/// fixture has: a sheet whose first row is not row 1, and one with no rows at
/// all.
fn package_with(workspace: &Workspace, name: &str, sheet: &str) -> PathBuf {
    let bare = workspace.sheet_package("bare.xlsx", "", r#"<c r="A1"><v>1</v></c>"#);
    workspace.zip(
        name,
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (support::WORKBOOK_PART, &part_text(&bare, "xl/workbook.xml")),
            (support::WORKBOOK_RELS_PART, support::WORKBOOK_RELS),
            (support::SHEET1_PART, sheet),
        ],
    )
}

/// A worksheet part holding `body` as its sheet data.
fn worksheet(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:C9"/>{body}<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
    )
}

/// The feature fixture's first row holds A1, D1 and E1, so a cell goes in
/// front of the first, between two, and after the last of them.
#[test]
fn a_cell_goes_into_its_row_in_column_order() {
    for (target, before, after) in [
        (
            "Inputs!B1",
            r#"<c r="D1">"#,
            r#"<c r="B1"><v>9</v></c><c r="D1">"#,
        ),
        (
            "Inputs!F1",
            r#"<v>2</v></c></row>"#,
            r#"<v>2</v></c><c r="F1"><v>9</v></c></row>"#,
        ),
    ] {
        let package = copy_of("in-row", "feature.xlsx");

        let written = writing(&package, target);

        assert_spliced(
            &part_text(&fixture("feature.xlsx"), SHEET1),
            &written,
            before,
            after,
        );
        assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
    }
}

/// Row 7 of the feature fixture holds only G7, so a cell goes in front of the
/// first cell the row has.
#[test]
fn a_cell_goes_in_front_of_the_first_cell_a_row_holds() {
    let package = copy_of("first-in-row", "feature.xlsx");

    let written = writing(&package, "Inputs!A7");

    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &written,
        r#"<c r="G7" s="3"/>"#,
        r#"<c r="A7" s="1"><v>9</v></c><c r="G7" s="3"/>"#,
    );
}

/// The fixture holds rows 1 to 5 and row 7, so a row goes between two and
/// after the last.
#[test]
fn a_row_goes_into_the_sheet_data_in_row_order() {
    for (target, before, after) in [
        (
            "Inputs!B6",
            r#"<row r="7""#,
            r#"<row r="6"><c r="B6"><v>9</v></c></row><row r="7""#,
        ),
        (
            "Inputs!A9",
            r#"<c r="G7" s="3"/></row></sheetData>"#,
            r#"<c r="G7" s="3"/></row><row r="9"><c r="A9"><v>9</v></c></row></sheetData>"#,
        ),
    ] {
        let package = copy_of("in-data", "feature.xlsx");

        let written = writing(&package, target);

        assert_spliced(
            &part_text(&fixture("feature.xlsx"), SHEET1),
            &written,
            before,
            after,
        );
        assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
    }
}

#[test]
fn a_row_goes_in_front_of_the_first_row_the_sheet_holds() {
    let workspace = Workspace::new("first-row");
    let sheet =
        worksheet(r#"<sheetData><row r="5" spans="1:1"><c r="A5"><v>5</v></c></row></sheetData>"#);
    let package = package_with(&workspace, "later.xlsx", &sheet);

    let written = writing(&package, "Inputs!A2");

    assert_spliced(
        &sheet,
        &written,
        r#"<row r="5""#,
        r#"<row r="2"><c r="A2"><v>9</v></c></row><row r="5""#,
    );
}

/// A sheet data element written self-closing is opened to take the first row,
/// and one written open-close simply takes it.
#[test]
fn the_first_row_of_a_sheet_with_none_opens_an_empty_sheet_data_element() {
    for (label, empty) in [
        ("closed", "<sheetData/>"),
        ("open", "<sheetData></sheetData>"),
    ] {
        let workspace = Workspace::new(label);
        let sheet = worksheet(empty);
        let package = package_with(&workspace, "empty.xlsx", &sheet);

        let written = writing(&package, "Inputs!B2");

        assert_spliced(
            &sheet,
            &written,
            empty,
            r#"<sheetData><row r="2"><c r="B2"><v>9</v></c></row></sheetData>"#,
        );
    }
}

/// Row 7 of the fixture declares a custom format, so a cell put into it takes
/// the row's style; column G is styled, so a cell put under it takes the
/// column's; a cell with neither carries none.
#[test]
fn an_inserted_cell_takes_the_style_excel_would_give_it() {
    for (target, expected) in [
        ("Inputs!B7", r#"<c r="B7" s="1"><v>9</v></c>"#),
        ("Inputs!G1", r#"<c r="G1" s="2"><v>9</v></c>"#),
        ("Inputs!B1", r#"<c r="B1"><v>9</v></c>"#),
    ] {
        let package = copy_of("styles", "feature.xlsx");

        let written = writing(&package, target);

        assert!(written.contains(expected), "{target}: {written}");
    }
}

/// A new row carries no style of its own even where a column would give its
/// cell one: the row is not custom-formatted, because nothing said it was.
#[test]
fn an_inserted_row_carries_its_number_and_nothing_else() {
    let package = copy_of("new-row", "feature.xlsx");

    let written = writing(&package, "Inputs!G9");

    assert!(
        written.contains(r#"<row r="9"><c r="G9" s="2"><v>9</v></c></row>"#),
        "{written}"
    );
}

/// Each write type goes into a cell that was not there the way it goes into
/// one that was.
#[test]
fn every_write_type_goes_into_a_cell_that_was_not_there() {
    for (target, write_type, value, expected) in [
        (
            "Inputs!B1",
            WriteType::Number,
            "2.5",
            r#"<c r="B1"><v>2.5</v></c>"#,
        ),
        (
            "Inputs!B1",
            WriteType::Text,
            "hello",
            r#"<c r="B1" t="inlineStr"><is><t>hello</t></is></c>"#,
        ),
        (
            "Inputs!B1",
            WriteType::Bool,
            "true",
            r#"<c r="B1" t="b"><v>1</v></c>"#,
        ),
        (
            "Inputs!B1",
            WriteType::Date,
            "2026-09-11",
            r#"<c r="B1"><v>46276</v></c>"#,
        ),
    ] {
        let package = copy_of("types", "feature.xlsx");

        let out = under_json(verb::set(&package, target, write_type, value, None, false));

        assert_eq!(out.exit, 0, "{value}: {}", out.stdout);
        assert!(
            part_text(&package, SHEET1).contains(expected),
            "{value}: {}",
            part_text(&package, SHEET1)
        );
    }
}

/// A worksheet part with no sheet data element holds no rows and has nowhere
/// to put one, which is a package xlsplice has nothing to say about beyond
/// saying so.
#[test]
fn a_sheet_with_no_sheet_data_element_is_not_found_and_the_package_is_untouched() {
    let workspace = Workspace::new("no-data");
    let package = package_with(&workspace, "dataless.xlsx", &worksheet(""));
    let before = std::fs::read(&package).expect("the package the test wrote is readable");

    let out = under_json(verb::set(
        &package,
        "Inputs!A1",
        WriteType::Number,
        "9",
        None,
        false,
    ));

    assert_eq!(out.exit, 3, "{}", out.stdout);
    let body = envelope(&out);
    assert_eq!(body["error"]["code"], json!("not_found"));
    let message = body["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message");
    assert!(message.contains("sheet data"), "{message}");
    assert_eq!(
        std::fs::read(&package).expect("the package the test wrote is readable"),
        before
    );
}

/// A batch may put a cell in and write one that was already there, and the
/// two land in the one part without treading on each other.
#[test]
fn a_batch_may_insert_a_cell_and_write_an_existing_one() {
    let package = copy_of("mixed", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!A1", WriteType::Number, "100"),
            verb::writing("Inputs!B1", WriteType::Number, "9"),
            verb::writing("Inputs!C6", WriteType::Text, "new row"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let written = part_text(&package, SHEET1);
    assert!(written.contains(r#"<c r="A1"><v>100</v></c>"#), "{written}");
    assert!(written.contains(r#"<c r="B1"><v>9</v></c>"#), "{written}");
    assert!(
        written.contains(r#"<row r="6"><c r="C6" t="inlineStr"><is><t>new row</t></is></c></row>"#),
        "{written}"
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
}

/// Two cells put into one row both land, each in its own place, which is what
/// says two insertions in one batch do not collide.
#[test]
fn two_cells_put_into_one_row_land_in_column_order() {
    let package = copy_of("two-in-row", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!C1", WriteType::Number, "3"),
            verb::writing("Inputs!B1", WriteType::Number, "2"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="D1">"#,
        r#"<c r="B1"><v>2</v></c><c r="C1"><v>3</v></c><c r="D1">"#,
    );
}

/// Two cells of a row the sheet does not hold go into one row, in column
/// order whatever order the batch named them in, because that is the order a
/// row has to read in.
#[test]
fn two_cells_of_one_row_the_sheet_does_not_hold_go_into_the_one_row() {
    let package = copy_of("row-once", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!C6", WriteType::Number, "3"),
            verb::writing("Inputs!A6", WriteType::Number, "1"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let body = envelope(&out);
    assert_eq!(body["operations"][0]["changed"], json!(true));
    assert_eq!(body["operations"][1]["changed"], json!(true));
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<row r="7""#,
        concat!(
            r#"<row r="6"><c r="A6"><v>1</v></c><c r="C6"><v>3</v></c></row>"#,
            r#"<row r="7""#
        ),
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
}

/// Each cell takes the style it would have taken alone: a row being put in
/// has no custom format, so only the columns have anything to say.
#[test]
fn each_cell_of_a_row_being_put_in_takes_its_own_style() {
    let package = copy_of("row-styles", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!G9", WriteType::Number, "7"),
            verb::writing("Inputs!A9", WriteType::Number, "1"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert!(
        part_text(&package, SHEET1)
            .contains(r#"<row r="9"><c r="A9"><v>1</v></c><c r="G9" s="2"><v>7</v></c></row>"#),
        "{}",
        part_text(&package, SHEET1)
    );
}

/// Cells of several rows the sheet does not hold put each row in once, in row
/// order.
#[test]
fn cells_of_several_rows_the_sheet_does_not_hold_put_each_row_in_once() {
    let package = copy_of("rows-once", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!B9", WriteType::Number, "9"),
            verb::writing("Inputs!A8", WriteType::Number, "8"),
            verb::writing("Inputs!A9", WriteType::Number, "7"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="G7" s="3"/></row></sheetData>"#,
        concat!(
            r#"<c r="G7" s="3"/></row>"#,
            r#"<row r="8"><c r="A8"><v>8</v></c></row>"#,
            r#"<row r="9"><c r="A9"><v>7</v></c><c r="B9"><v>9</v></c></row>"#,
            r#"</sheetData>"#
        ),
    );
}

/// A cell put into a row that is there and a cell put into a row that is not
/// both land, in the one part.
#[test]
fn a_new_row_and_a_cell_in_an_existing_row_land_together() {
    let package = copy_of("mixed-rows", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!B1", WriteType::Number, "1"),
            verb::writing("Inputs!A6", WriteType::Number, "6"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let written = part_text(&package, SHEET1);
    assert!(written.contains(r#"<c r="B1"><v>1</v></c>"#), "{written}");
    assert!(
        written.contains(r#"<row r="6"><c r="A6"><v>6</v></c></row>"#),
        "{written}"
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
}

/// Two cells of two different rows the sheet does not hold are two rows, and
/// both go in, in row order.
#[test]
fn two_cells_of_two_rows_the_sheet_does_not_hold_both_go_in() {
    let package = copy_of("two-rows", "feature.xlsx");

    let out = under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!A9", WriteType::Number, "9"),
            verb::writing("Inputs!A8", WriteType::Number, "8"),
        ],
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="G7" s="3"/></row></sheetData>"#,
        concat!(
            r#"<c r="G7" s="3"/></row><row r="8"><c r="A8"><v>8</v></c></row>"#,
            r#"<row r="9"><c r="A9"><v>9</v></c></row></sheetData>"#
        ),
    );
}

#[test]
fn insertion_works_through_apply() {
    let workspace = Workspace::new("through-apply");
    let package = workspace.copy_of("feature.xlsx");
    let batch = workspace.file(
        "batch.json",
        br#"[
            {"op": "set", "target": "Inputs!B1", "type": "number", "value": "9"},
            {"op": "set", "target": "Inputs!A8", "type": "text", "value": "eight"}
        ]"#,
    );

    let out = run(&[
        "apply",
        &package.display().to_string(),
        &batch.display().to_string(),
        "--json",
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["parts"]["changed"], json!([SHEET1]));
    let written = part_text(&package, SHEET1);
    assert!(written.contains(r#"<c r="B1"><v>9</v></c>"#), "{written}");
    assert!(
        written.contains(r#"<row r="8"><c r="A8" t="inlineStr"><is><t>eight</t></is></c></row>"#),
        "{written}"
    );
}

/// A defined name anchoring at a cell the sheet does not hold is a target
/// like any other, and the cell goes in where it anchors.
#[test]
fn a_write_through_a_defined_name_puts_the_anchor_in() {
    let package = copy_of("via-name", "feature.xlsx");

    let out = under_json(verb::set(
        &package,
        "MergedInput",
        WriteType::Number,
        "9",
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["operations"][0]["address"],
        json!("Inputs!B2")
    );
    assert!(
        part_text(&package, SHEET1).contains(r#"<c r="B2" s="4"><v>9</v></c>"#),
        "the cell was already there, style and all"
    );
}

/// Putting a cell in is a write like any other, so the flags that say where
/// the result goes mean what they always mean.
#[test]
fn a_dry_run_reports_the_insertion_and_writes_nothing() {
    let package = copy_of("dry", "feature.xlsx");

    let out = under_json(verb::set(
        &package,
        "Inputs!B1",
        WriteType::Number,
        "9",
        None,
        true,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let body = envelope(&out);
    assert_eq!(body["dry_run"], json!(true));
    assert_eq!(body["parts"]["changed"], json!([SHEET1]));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}
