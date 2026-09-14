//! Contract tests for `clear`.
//!
//! Clearing empties a cell the way Excel does when its contents are deleted:
//! the element stays, its style stays, and its value, its type and any inline
//! string go. So the assertions are about what stayed as much as what went.

mod support;

use std::path::Path;

use support::binary::{exit_code, run, stderr};
use support::container::{
    SHEET1, SHEET2, assert_only_these_differ, assert_same_bytes, assert_spliced, part_text,
};
use support::library::{envelope, in_text, op, under_json};
use support::workspace::{copy_of, fixture};
use xlsplice::batch::Batch;
use xlsplice::batch::Destination;
use xlsplice::batch::WriteType;
use xlsplice::verb::{self, Trace};

/// A `clear` that must succeed, and the envelope it answers with.
fn clear(package: &Path, target: &str) -> serde_json::Value {
    let rendered = under_json(verb::clear(
        package,
        target,
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(rendered.exit, 0, "clear {target}: {}", rendered.stdout);
    envelope(&rendered)
}

/// A cell with a style keeps the element and the style and loses everything
/// else. The plain fixture's date cell is the one Excel saved with both.
#[test]
fn clearing_a_styled_cell_keeps_the_element_and_the_style() {
    let package = copy_of("styled", "plain.xlsx");

    let body = clear(&package, "Sheet1!C1");

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(true));
    assert_only_these_differ(&fixture("plain.xlsx"), &package, &[SHEET1]);
    assert_spliced(
        &part_text(&fixture("plain.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="C1" s="1"><v>46276</v></c>"#,
        r#"<c r="C1" s="1"/>"#,
    );
}

/// A shared-string cell loses its type as well as its value, so nothing is
/// left claiming the cell holds a string index it no longer has.
#[test]
fn clearing_a_shared_string_cell_takes_the_type_away_with_the_value() {
    let package = copy_of("shared", "plain.xlsx");

    clear(&package, "Sheet1!B1");

    assert_spliced(
        &part_text(&fixture("plain.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="B1" t="s"><v>0</v></c>"#,
        r#"<c r="B1"/>"#,
    );
}

/// An inline string is held in the cell itself, so clearing has to take the
/// whole of it away. No fixture holds one, so the test writes one first: what
/// `set --type text` produces is exactly what `clear` has to undo.
#[test]
fn clearing_an_inline_string_takes_the_whole_of_it_away() {
    let package = copy_of("inline", "plain.xlsx");

    let out = under_json(verb::set(
        &package,
        "Sheet1!A1",
        WriteType::Text,
        "written",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert!(part_text(&package, SHEET1).contains(r#"<is><t>written</t></is>"#));

    clear(&package, "Sheet1!A1");

    assert_spliced(
        &part_text(&fixture("plain.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="A1"><v>1</v></c>"#,
        r#"<c r="A1"/>"#,
    );
}

/// A cell already empty is already what a clear would make of it, so nothing
/// is written and the report says nothing changed.
#[test]
fn clearing_a_cell_that_is_already_empty_changes_nothing() {
    let package = copy_of("already", "feature.xlsx");

    let body = clear(&package, "Inputs!B2");

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(false));
    assert_eq!(body["parts"]["changed"], serde_json::json!([]));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// A formula is refused until the ticket that licenses replacing one, so a
/// mis-addressed clear cannot destroy a template's formula either.
#[test]
fn clearing_a_formula_cell_is_refused_and_the_package_is_untouched() {
    let package = copy_of("formula", "feature.xlsx");

    for target in ["Inputs!D1", "Inputs!E2", "Inputs!E3"] {
        let out = under_json(verb::clear(
            &package,
            target,
            false,
            &Destination::InPlace,
            false,
            &Trace::Off,
        ));

        assert_eq!(out.exit, 4, "{target}");
        assert_eq!(
            envelope(&out)["error"]["code"],
            serde_json::json!("refused"),
            "{target}"
        );
    }
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn clearing_through_a_defined_name_lands_in_the_anchor() {
    let package = copy_of("by-name", "feature.xlsx");

    let body = clear(&package, "Notes!LocalNote");

    assert_eq!(
        body["operations"],
        serde_json::json!([{
            "target": "Notes!LocalNote",
            "name": "LocalNote",
            "sheet": "Notes",
            "cell": "A5",
            "address": "Notes!A5",
            "changed": true,
        }])
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET2]);
    assert!(part_text(&package, SHEET2).contains(r#"<c r="A5"/>"#));
}

/// The command line and a batch are two ways of asking for the same
/// operation, so they produce the same bytes.
#[test]
fn a_clear_through_a_batch_produces_the_same_bytes_as_the_command_line() {
    let from_command_line = copy_of("clear-cli", "plain.xlsx");
    let from_batch = copy_of("clear-batch", "plain.xlsx");

    clear(&from_command_line, "Sheet1!C1");
    let out = under_json(verb::run(
        &from_batch,
        &Batch {
            operations: vec![op::clearing("Sheet1!C1")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);

    assert_same_bytes(&from_command_line, &from_batch);
}

/// A batch may clear one cell and write another in one invocation.
#[test]
fn a_batch_may_clear_one_cell_and_write_another() {
    let package = copy_of("mixed", "plain.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::clearing("Sheet1!C1"),
                op::writing("Sheet1!A1", WriteType::Number, "9"),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let after = part_text(&package, SHEET1);
    assert!(after.contains(r#"<c r="C1" s="1"/>"#), "{after}");
    assert!(after.contains(r#"<c r="A1"><v>9</v></c>"#), "{after}");
    assert_only_these_differ(&fixture("plain.xlsx"), &package, &[SHEET1]);
}

/// A `set` puts a cell that is not there into the sheet; a `clear` does not,
/// because an absent cell already holds nothing and already shows whatever
/// format its row or its column gives it. Putting an empty element there
/// would be a change with nothing behind it.
#[test]
fn clearing_a_cell_the_sheet_does_not_hold_changes_nothing() {
    let package = copy_of("absent", "plain.xlsx");

    for target in ["Sheet1!Z1", "Sheet1!A9"] {
        let out = under_json(verb::clear(
            &package,
            target,
            false,
            &Destination::InPlace,
            false,
            &Trace::Off,
        ));

        assert_eq!(out.exit, 0, "{target}: {}", out.stdout);
        assert_eq!(
            envelope(&out)["operations"][0]["changed"],
            serde_json::json!(false),
            "{target}"
        );
    }
    assert_same_bytes(&fixture("plain.xlsx"), &package);
}

#[test]
fn the_text_output_is_one_tab_separated_row_per_operation() {
    let package = copy_of("rows", "plain.xlsx");

    let out = in_text(verb::clear(
        &package,
        "Sheet1!C1",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "Sheet1!C1\t\tSheet1!C1\ttrue\n");
}

/// `clear` is a writing verb, so it takes the two flags every writing verb
/// takes, and the help lists them beside its own operands.
#[test]
fn the_help_lists_the_operands_and_the_writing_flags() {
    let out = run(&["clear", "--help"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    let help = support::binary::stdout(&out);
    for wanted in ["FILE", "TARGET", "--out", "--dry-run"] {
        assert!(
            help.contains(wanted),
            "clear --help must list {wanted}: {help}"
        );
    }
}

/// `clear` end to end, argv to the bytes on disk, so its dispatch arm does
/// not go unexercised.
#[test]
fn clear_writes_the_package_from_the_command_line() {
    let package = copy_of("argv", "plain.xlsx");

    let out = run(&["clear", &package.display().to_string(), "Sheet1!C1"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert!(part_text(&package, SHEET1).contains(r#"<c r="C1" s="1"/>"#));
}
