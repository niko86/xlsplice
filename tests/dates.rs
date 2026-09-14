//! Writing a date: `set --type date`, and the same through a batch.
//!
//! A workbook holds no dates, only numbers under a format that shows one, so
//! writing a date means writing the serial Excel would have written. Which
//! serial that is depends on the workbook, which is why this is asserted
//! against packages rather than against the conversion alone; the conversion
//! itself is unit-tested in `src/date.rs`.

mod support;

use std::path::Path;

use support::container::{SHEET1, assert_same_bytes, part_text};
use support::library::{envelope, op, targets, under_json};
use support::workspace::{Workspace, copy_of, fixture};
use xlsplice::batch::Batch;
use xlsplice::batch::Destination;
use xlsplice::batch::WriteType;
use xlsplice::verb::{self, Trace};

/// The date cell of the plain fixture, which Excel saved holding the serial
/// for 2026-09-11 under a date format.
const DATE_CELL: &str = r#"<c r="C1" s="1"><v>46276</v></c>"#;

/// Write `value` as a date into the plain fixture's date cell, and give back
/// the text of the worksheet it produced.
fn written(label: &str, value: &str) -> String {
    let package = copy_of(label, "plain.xlsx");
    let out = under_json(verb::set(
        &package,
        "Sheet1!C1",
        WriteType::Date,
        value,
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{value}: {}", out.stdout);
    part_text(&package, SHEET1)
}

/// The cell as it reads after writing `value`, with every other byte of the
/// part asserted to be the byte that was there.
fn cell_after(label: &str, value: &str) -> String {
    let after = written(label, value);
    let before = part_text(&fixture("plain.xlsx"), SHEET1);
    let (head, rest) = before
        .split_once(DATE_CELL)
        .expect("the fixture holds the date cell");
    assert!(
        after.starts_with(head),
        "{value}: the part moved before the cell"
    );
    assert!(
        after.ends_with(rest),
        "{value}: the part moved after the cell"
    );
    after[head.len()..after.len() - rest.len()].to_owned()
}

#[test]
fn an_iso_date_and_its_serial_produce_the_same_bytes() {
    let from_iso = cell_after("iso", "2026-09-13");
    let from_serial = cell_after("serial", "46278");

    assert_eq!(from_iso, r#"<c r="C1" s="1"><v>46278</v></c>"#);
    assert_eq!(from_iso, from_serial, "one date, one serial, one cell");
}

#[test]
fn a_datetime_produces_a_fractional_serial() {
    assert_eq!(
        cell_after("noon", "2026-09-13T12:00:00"),
        r#"<c r="C1" s="1"><v>46278.5</v></c>"#
    );
}

/// The style is the cell's own and a write never touches it, so the date
/// still renders as a date.
#[test]
fn writing_a_date_keeps_the_format_that_makes_it_look_like_one() {
    assert!(
        cell_after("style", "2026-09-13").contains(r#"s="1""#),
        "the date format is the style the cell carried"
    );
}

#[test]
fn get_reads_the_serial_back() {
    let package = copy_of("read-back", "plain.xlsx");

    let out = under_json(verb::set(
        &package,
        "Sheet1!C1",
        WriteType::Date,
        "2026-09-13T12:00:00",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);

    let body = envelope(&under_json(verb::get(
        &package,
        &targets(&["Sheet1!C1"]),
        &Trace::Off,
    )));
    assert_eq!(body["cells"][0]["type"], serde_json::json!("n"));
    assert_eq!(body["cells"][0]["value"], serde_json::json!(46_278.5));
    assert_eq!(body["cells"][0]["raw"], serde_json::json!("46278.5"));
}

/// A workbook on the 1904 system counts from a different day, so the same
/// date is a different serial in it. No fixture is on that system, so the
/// test builds a package that is.
#[test]
fn a_1904_workbook_yields_the_shifted_serial() {
    let workspace = Workspace::new("1904");
    let package = workspace.sheet_package(
        "mac.xlsx",
        r#"<workbookPr date1904="1"/>"#,
        r#"<c r="A1"><v>0</v></c>"#,
    );

    let out = under_json(verb::set(
        &package,
        "Inputs!A1",
        WriteType::Date,
        "2026-09-13",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert!(
        part_text(&package, SHEET1).contains(r#"<c r="A1"><v>44816</v></c>"#),
        "1462 days lower than the 46278 the 1900 system gives: {}",
        part_text(&package, SHEET1)
    );
}

/// The same package on the default system, so that what the 1904 flag changed
/// is the flag and nothing else about the test.
#[test]
fn the_same_package_without_the_flag_yields_the_unshifted_serial() {
    let workspace = Workspace::new("1900");
    let package = workspace.sheet_package("pc.xlsx", "", r#"<c r="A1"><v>0</v></c>"#);

    let out = under_json(verb::set(
        &package,
        "Inputs!A1",
        WriteType::Date,
        "2026-09-13",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert!(part_text(&package, SHEET1).contains(r#"<c r="A1"><v>46278</v></c>"#));
}

/// The 1900 system counts a 29th of February 1900 that never happened, so
/// nothing below 1900-03-01 names the day a calendar would. Rather than pick
/// an answer, the write is refused and says why.
#[test]
fn a_date_before_the_phantom_leap_day_exits_2_and_leaves_the_package_alone() {
    for value in ["1900-01-01", "1899-12-31", "59"] {
        let package = copy_of("early", "plain.xlsx");

        let out = under_json(verb::set(
            &package,
            "Sheet1!C1",
            WriteType::Date,
            value,
            false,
            &Destination::InPlace,
            false,
            &Trace::Off,
        ));

        assert_eq!(out.exit, 2, "{value}");
        let body = envelope(&out);
        assert_eq!(body["error"]["code"], serde_json::json!("usage"), "{value}");
        let message = body["error"]["message"]
            .as_str()
            .expect("a failed envelope carries a message");
        assert!(
            message.contains("29th of February 1900"),
            "{value}: the message must say why: {message}"
        );
        assert_same_bytes(&fixture("plain.xlsx"), &package);
    }
}

#[test]
fn a_spelling_that_is_not_a_date_exits_2_and_names_the_operation() {
    let package = copy_of("not-a-date", "plain.xlsx");

    let out = under_json(verb::set(
        &package,
        "Sheet1!C1",
        WriteType::Date,
        "the thirteenth",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 2);
    let message = envelope(&out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned();
    assert!(
        message.starts_with("operation at index 0 (set Sheet1!C1): "),
        "{message}"
    );
    assert!(message.contains("ISO 8601"), "{message}");
    assert_same_bytes(&fixture("plain.xlsx"), &package);
}

/// The command line and a batch are two ways of asking for the same
/// operation, so they produce the same bytes.
#[test]
fn a_date_through_a_batch_produces_the_same_bytes_as_the_command_line() {
    let from_command_line = copy_of("date-cli", "plain.xlsx");
    let from_batch = copy_of("date-batch", "plain.xlsx");

    let out = under_json(verb::set(
        &from_command_line,
        "Sheet1!C1",
        WriteType::Date,
        "2026-09-13T06:00:00",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);
    let out = under_json(verb::run(
        &from_batch,
        &Batch {
            operations: vec![op::writing(
                "Sheet1!C1",
                WriteType::Date,
                "2026-09-13T06:00:00",
            )],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);

    assert_same_bytes(&from_command_line, &from_batch);
    assert!(
        part_text(&from_batch, SHEET1).contains(r#"<c r="C1" s="1"><v>46278.25</v></c>"#),
        "{}",
        part_text(&from_batch, SHEET1)
    );
}

/// Nothing here writes to a fixture.
#[test]
fn the_fixture_is_the_baseline_and_stays_put() {
    assert!(Path::new(&fixture("plain.xlsx")).exists());
}
