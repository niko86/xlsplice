//! Contract tests for `cells` (#50, ADR-0008).
//!
//! `get` answers about the cells a caller named; `cells` answers about what a
//! sheet holds. So what is asserted here is which cells come back as much as
//! what each holds: every occupied cell, in the order the part holds them,
//! and nothing that carries only a style.
//!
//! These call the library in process, as `get.rs` does. One spawns, for what
//! clap does with a sheet missing.

mod support;

use serde_json::json;
use support::binary::{exit_code, run};
use support::container::SHARED_STRINGS;
use support::library::{envelope, in_text, targets, under_json};
use support::workspace::{Copied, Workspace, built};
use xlsplice::verb::{self, Trace};

/// The feature package, and the workspace holding it alive for as long as the
/// test needs it.
fn feature(label: &str) -> Copied {
    built(label, |w| w.feature_package("feature.xlsx"))
}

/// The envelope `cells` answers with for `sheets` of `package`, which must
/// succeed.
fn cells(package: &std::path::Path, sheets: &[&str]) -> serde_json::Value {
    let out = under_json(verb::cells(package, &targets(sheets), &Trace::Off));
    assert_eq!(out.exit, 0, "cells {sheets:?}: {}", out.stdout);
    envelope(&out)
}

/// The addresses of one sheet's block, in the order they came back.
fn addresses(block: &serde_json::Value) -> Vec<String> {
    block["cells"]
        .as_array()
        .expect("cells is a list")
        .iter()
        .map(|cell| cell["address"].as_str().expect("an address").to_owned())
        .collect()
}

/// The feature sheet holds a cell of every stored type, formulas of every
/// role, and A6, which carries a style and nothing else. A6 is not occupied,
/// so it is not a row and the extent stops at row 3.
#[test]
fn every_occupied_cell_comes_back_in_part_order_and_a_style_alone_is_left_out() {
    let package = feature("occupied");

    let body = cells(&package, &["Inputs"]);

    let block = &body["sheets"][0];
    assert_eq!(block["sheet"], json!("Inputs"));
    assert_eq!(
        addresses(block),
        [
            "Inputs!A1",
            "Inputs!B1",
            "Inputs!C1",
            "Inputs!D1",
            "Inputs!E1",
            "Inputs!F1",
            "Inputs!G1",
            "Inputs!H1",
            "Inputs!I1",
            "Inputs!J1",
            "Inputs!K1",
            "Inputs!A2",
            "Inputs!B2",
            "Inputs!D2",
            "Inputs!E2",
            "Inputs!A3",
            "Inputs!E3",
        ]
    );
    assert_eq!(block["extent"], json!("A1:K3"));
}

/// The error cell is the one python-calamine reads as `""`, and the reason
/// #50 was asked for.
#[test]
fn an_error_cell_keeps_its_error_as_its_value() {
    let package = feature("error");

    let body = cells(&package, &["Inputs"]);

    let error = &body["sheets"][0]["cells"][4];
    assert_eq!(error["address"], json!("Inputs!E1"));
    assert_eq!(error["type"], json!("e"));
    assert_eq!(error["value"], json!("#DIV/0!"));
}

/// One parser reads both verbs: every cell `cells` answers with is the cell
/// `get` answers with for its address, field for field, without the two
/// fields only a target has.
#[test]
fn each_cell_is_the_cell_get_reads_at_its_address_without_target_and_name() {
    let package = feature("same-as-get");

    let body = cells(&package, &["Inputs"]);

    for listed in body["sheets"][0]["cells"]
        .as_array()
        .expect("cells is a list")
    {
        let address = listed["address"].as_str().expect("an address");
        let got = envelope(&under_json(verb::get(
            &package,
            &targets(&[address]),
            &Trace::Off,
        )))["cells"][0]
            .clone();
        let mut got = got.as_object().expect("a cell is an object").clone();
        got.remove("target");
        got.remove("name");
        assert_eq!(listed, &json!(got), "{address}");
    }
}

/// A formula Excel never calculated holds no value, and is occupied all the
/// same: `raw: null` on a formula cell is information (ADR-0007).
#[test]
fn a_formula_never_calculated_is_occupied_and_reads_as_empty() {
    let workspace = Workspace::new("uncalculated");
    let package = workspace.sheet_package(
        "uncalculated.xlsx",
        "",
        r#"<c r="A1"><f>SUM(B1:B9)</f><v></v></c><c r="B1" s="3"/><c r="C1"><v>7</v></c>"#,
    );

    let body = cells(&package, &["Inputs"]);

    let block = &body["sheets"][0];
    assert_eq!(addresses(block), ["Inputs!A1", "Inputs!C1"]);
    assert_eq!(block["cells"][0]["type"], json!("empty"));
    assert_eq!(block["cells"][0]["raw"], json!(null));
    assert_eq!(block["cells"][0]["formula"]["text"], json!("SUM(B1:B9)"));
    assert_eq!(block["extent"], json!("A1:C1"));
}

#[test]
fn a_sheet_holding_no_occupied_cell_has_no_cells_and_no_extent() {
    let package = feature("empty-sheet");

    let body = cells(&package, &["Parameters"]);

    assert_eq!(
        body["sheets"][0],
        json!({"sheet": "Parameters", "extent": null, "cells": []})
    );
}

#[test]
fn one_occupied_cell_is_an_extent_of_that_one_cell() {
    let package = feature("one-cell");

    let body = cells(&package, &["Notes"]);

    assert_eq!(body["sheets"][0]["extent"], json!("A5"));
}

/// Sheets come back one block each, in the order named and in the package's
/// own spelling, and a sheet named twice is answered twice, as a target
/// given twice is.
#[test]
fn several_sheets_come_back_one_block_each_in_the_order_named() {
    let package = feature("several");

    let body = cells(&package, &["notes", "Inputs", "NOTES"]);

    let sheets: Vec<_> = body["sheets"]
        .as_array()
        .expect("sheets is a list")
        .iter()
        .map(|block| block["sheet"].clone())
        .collect();
    assert_eq!(sheets, [json!("Notes"), json!("Inputs"), json!("Notes")]);
    assert_eq!(body["sheets"][0], body["sheets"][2]);
}

/// The rows are the cells alone, under `get`'s columns without the first
/// two: the extent has no row, and the address says which sheet a cell is on.
#[test]
fn the_rows_are_the_cells_one_after_another_with_every_field_in_its_place() {
    let package = feature("rows");

    let out = in_text(verb::cells(
        &package,
        &targets(&["Notes", "Parameters"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stderr, "");
    assert_eq!(out.stdout, "Notes!A5\tinlineStr\tnote\tnote\t0\t\t\t\t\n");
}

/// A sheet with no shared-string cell never needs the table, so a table that
/// cannot be read does not stop it being read: the table is only opened when
/// a cell turns out to want it.
#[test]
fn the_shared_string_table_is_read_only_when_a_cell_needs_it() {
    let package = built("unread-table", |w| {
        let base = w.feature_package("base.xlsx");
        w.like("broken-table.xlsx", &base)
            .with_part(SHARED_STRINGS, "<sst")
            .written()
    });

    cells(&package, &["Notes"]);

    let out = under_json(verb::cells(&package, &targets(&["Inputs"]), &Trace::Off));
    assert_eq!(envelope(&out)["error"]["code"], json!("unreadable"));
}

/// A number cell holding no number is the part disagreeing with itself, and
/// one such cell fails the whole read rather than leaving a hole in it.
#[test]
fn one_unreadable_cell_fails_the_whole_read_and_names_the_cell() {
    let workspace = Workspace::new("unreadable");
    let package = workspace.sheet_package(
        "unreadable.xlsx",
        "",
        r#"<c r="A1"><v>1</v></c><c r="B1"><v>abc</v></c>"#,
    );

    let out = under_json(verb::cells(&package, &targets(&["Inputs"]), &Trace::Off));

    let body = envelope(&out);
    assert_eq!(body["error"]["code"], json!("unreadable"));
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("Inputs!B1"), "{message}");
    assert_eq!(body.get("sheets"), None);
}

#[test]
fn an_unknown_sheet_is_not_found_and_the_message_lists_the_sheets() {
    let package = feature("unknown");

    let out = under_json(verb::cells(
        &package,
        &targets(&["Inputs", "Nowhere"]),
        &Trace::Off,
    ));

    let body = envelope(&out);
    assert_eq!(body["error"]["code"], json!("not_found"));
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("Nowhere"), "{message}");
    assert!(message.contains("Inputs, Notes, Parameters"), "{message}");
}

/// Cells and rows both carry a sheet's position by its own numbers, so cells
/// and rows that leave theirs out follow the one before them.
#[test]
fn cells_and_rows_that_declare_no_address_follow_the_one_before() {
    let workspace = Workspace::new("unaddressed");
    let package = workspace.sheet_package(
        "unaddressed.xlsx",
        "",
        r#"<c r="B1"><v>1</v></c><c><v>2</v></c>"#,
    );

    let body = cells(&package, &["Inputs"]);

    assert_eq!(addresses(&body["sheets"][0]), ["Inputs!B1", "Inputs!C1"]);
    assert_eq!(body["sheets"][0]["extent"], json!("B1:C1"));
}

#[test]
fn cells_needs_a_package_and_at_least_one_sheet() {
    let out = run(&["cells", "book.xlsx"]);

    assert_eq!(exit_code(&out), 2);
}
