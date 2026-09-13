//! What a batch of more than one operation does to one package.
//!
//! A batch of one is what the command line builds and what `set.rs` asserts.
//! Here is what only several operations at once can show: that two on one
//! part are spliced into it together, and that two on one cell are refused
//! whole, because a batch is a set of edits over the package as it was read
//! rather than a sequence over a document changing under it (ADR-0004).
//!
//! The `apply` verb that will carry such a batch from the command line is
//! #8's. These call the library the way it will.

mod support;

use std::path::{Path, PathBuf};

use support::{
    Workspace, assert_same_bytes, envelope, exit_code, fixture, part_text, run, stdout, under_json,
    verb, workbook_xml,
};
use xlsplice::batch::WriteType;

const SHEET1: &str = "xl/worksheets/sheet1.xml";

/// A package of one sheet holding one row, whose cells the test writes out.
/// The shapes that matter here are ones Excel does not save, so no fixture
/// carries them.
fn package_of(workspace: &Workspace, row: &str) -> PathBuf {
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1" spans="1:2">{row}</row></sheetData>
</worksheet>"#
    );
    workspace.zip(
        "one-sheet.xlsx",
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (
                support::WORKBOOK_PART,
                &workbook_xml(r#"<sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets>"#),
            ),
            (support::WORKBOOK_RELS_PART, support::WORKBOOK_RELS),
            (support::SHEET1_PART, &sheet),
        ],
    )
}

/// The message of a batch that was refused, with the exit code asserted to be
/// the one the frozen table gives `usage`.
fn refused(package: &Path, operations: Vec<xlsplice::batch::Operation>) -> String {
    let out = under_json(verb::batch(package, operations, None, false));

    assert_eq!(out.exit, 2, "a contradictory batch is a usage error");
    let body = envelope(&out);
    assert_eq!(body["error"]["code"], serde_json::json!("usage"));
    body["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned()
}

#[test]
fn two_operations_on_different_cells_of_one_part_are_spliced_into_it_together() {
    let workspace = Workspace::new("together");
    let package = workspace.copy_of("feature.xlsx");
    let before = part_text(&fixture("feature.xlsx"), SHEET1);

    let body = envelope(&under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!A1", WriteType::Number, "11"),
            verb::writing("Inputs!A3", WriteType::Number, "13"),
        ],
        None,
        false,
    )));

    assert_eq!(
        body["parts"]["changed"],
        serde_json::json!([SHEET1]),
        "one part, spliced once"
    );
    assert_eq!(body["operations"][0]["changed"], serde_json::json!(true));
    assert_eq!(body["operations"][1]["changed"], serde_json::json!(true));
    assert_eq!(
        part_text(&package, SHEET1),
        before
            .replacen(r#"<c r="A1"><v>1</v></c>"#, r#"<c r="A1"><v>11</v></c>"#, 1)
            .replacen(r#"<c r="A3"><v>3</v></c>"#, r#"<c r="A3"><v>13</v></c>"#, 1),
        "both edits landed and every other byte stayed"
    );
}

#[test]
fn two_operations_naming_one_cell_are_refused_and_the_package_is_untouched() {
    let workspace = Workspace::new("repeated");
    let package = workspace.copy_of("feature.xlsx");

    let message = refused(
        &package,
        vec![
            verb::writing("Inputs!A1", WriteType::Number, "1"),
            verb::writing("Inputs!A3", WriteType::Number, "3"),
            verb::writing("Inputs!A1", WriteType::Number, "2"),
        ],
    );

    assert!(
        message.contains("index 0 and 2"),
        "the failure must name both operations: {message}"
    );
    assert!(
        message.contains("Inputs!A1"),
        "the failure must name the cell they agree on: {message}"
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// The two are compared on what they resolved to, not on what they said, so
/// an address and a defined name that anchors at it are one cell.
#[test]
fn one_cell_named_twice_two_ways_is_still_one_cell() {
    let workspace = Workspace::new("two-ways");
    let package = workspace.copy_of("feature.xlsx");

    let message = refused(
        &package,
        vec![
            verb::writing("MergedInput", WriteType::Text, "by name"),
            verb::writing("Inputs!B2", WriteType::Text, "by address"),
        ],
    );

    assert!(
        message.contains("index 0 and 1") && message.contains("Inputs!B2"),
        "the anchor of the name is the cell the address names: {message}"
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// A cell written as an empty element with a separate closing tag gives two
/// splices of an empty range at one byte. They do not overlap, so the guard in
/// `splice::apply` never fires on them, and before this check the batch wrote
/// a part carrying `t="inlineStr"` twice, which no longer parses as XML. Excel
/// writes `<c r="A1"/>`, so no fixture reaches this shape and the test names
/// it itself.
#[test]
fn a_cell_written_as_an_empty_element_is_refused_rather_than_given_two_types() {
    let workspace = Workspace::new("empty-element");
    let package = package_of(&workspace, r#"<c r="A1"></c><c r="B1" s="4"></c>"#);
    let before = part_text(&package, SHEET1);

    for target in ["Inputs!A1", "Inputs!B1"] {
        let message = refused(
            &package,
            vec![
                verb::writing(target, WriteType::Text, "first"),
                verb::writing(target, WriteType::Text, "second"),
            ],
        );

        assert!(message.contains("index 0 and 1"), "{target}: {message}");
        let after = part_text(&package, SHEET1);
        assert_eq!(after, before, "{target}: the part was not touched");
        assert!(
            !after.contains(r#"t="inlineStr" t="inlineStr""#),
            "{target}: the part must never carry one attribute twice"
        );
    }
}

/// A batch of one still writes, and two operations on two cells still write:
/// the check refuses a repeated cell, not a repeated part.
#[test]
fn the_check_refuses_a_repeated_cell_rather_than_a_repeated_part() {
    let workspace = Workspace::new("not-a-part");
    let package = package_of(
        &workspace,
        r#"<c r="A1"><v>1</v></c><c r="B1"><v>2</v></c>"#,
    );

    let body = envelope(&under_json(verb::batch(
        &package,
        vec![
            verb::writing("Inputs!A1", WriteType::Number, "10"),
            verb::writing("Inputs!B1", WriteType::Number, "20"),
        ],
        None,
        false,
    )));

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(true));
    assert_eq!(body["operations"][1]["changed"], serde_json::json!(true));
    assert!(part_text(&package, SHEET1).contains(r#"<c r="A1"><v>10</v></c>"#));
    assert!(part_text(&package, SHEET1).contains(r#"<c r="B1"><v>20</v></c>"#));
}

/// The command line builds a batch of one, so it cannot reach the check at
/// all: `set` writes the same cell twice over two invocations, which is two
/// batches and not one.
#[test]
fn two_invocations_of_set_on_one_cell_are_two_batches_and_both_land() {
    let workspace = Workspace::new("two-runs");
    let package = workspace.copy_of("plain.xlsx");
    let file = package.display().to_string();

    for value in ["5", "6"] {
        let out = run(&["set", &file, "Sheet1!A1", value, "--type", "number"]);
        assert_eq!(exit_code(&out), 0, "{}", stdout(&out));
    }

    assert!(part_text(&package, SHEET1).contains(r#"<c r="A1"><v>6</v></c>"#));
}
