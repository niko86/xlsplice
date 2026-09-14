//! Contract tests for `calc`: reporting the calculation flag, and setting it.
//!
//! The flag belongs to the workbook, not to a cell, so this is the first verb
//! whose operation names no target at all; what the report says about an
//! operation like that is asserted here as well as in `answers.rs`.
//!
//! Excel writes `<calcPr calcId="..."/>`, self-closing and with no flag on
//! it, which is what every fixture carries. The shapes no fixture has — the
//! element written open-close, and a workbook carrying no element at all —
//! are packages these tests write for themselves.

mod support;

use std::path::{Path, PathBuf};

use serde_json::json;
use support::{
    Workspace, assert_only_these_differ, assert_same_bytes, assert_spliced, copy_of, envelope,
    exit_code, fixture, in_text, json, op, part_text, run, stderr, stdout, under_json,
};
use xlsplice::batch::Batch;
use xlsplice::batch::Destination;
use xlsplice::batch::WriteType;
use xlsplice::verb::{self, Trace};

const WORKBOOK: &str = "xl/workbook.xml";
const SHEET1: &str = "xl/worksheets/sheet1.xml";

/// A package of one sheet whose workbook part carries `tail` after its
/// sheets, which is where the calculation element goes and so where
/// [`Workspace::sheet_package`], which writes what comes before them, cannot
/// put one.
fn workbook_ending_with(workspace: &Workspace, name: &str, tail: &str) -> PathBuf {
    let bare = workspace.sheet_package("bare.xlsx", "", r#"<c r="A1"><v>1</v></c>"#);
    let workbook = part_text(&bare, WORKBOOK).replace("</workbook>", &format!("{tail}</workbook>"));
    workspace.zip(
        name,
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (support::WORKBOOK_PART, &workbook),
            (support::WORKBOOK_RELS_PART, support::WORKBOOK_RELS),
            (support::SHEET1_PART, &part_text(&bare, SHEET1)),
        ],
    )
}

/// What `calc FILE --json` reports.
fn reported(package: &Path) -> bool {
    let out = run(&["calc", &package.display().to_string(), "--json"]);
    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    json(&out)["full_calc_on_load"]
        .as_bool()
        .expect("the flag is reported as a boolean")
}

/// Set the flag through a batch of one, in process, and give back the
/// envelope it answered with.
fn set(package: &Path) -> serde_json::Value {
    let out = under_json(verb::run(
        package,
        &Batch {
            operations: vec![op::calculating(true)],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);
    envelope(&out)
}

#[test]
fn reading_reports_the_flag_absent_and_leaves_the_package_alone() {
    let package = copy_of("read-unset", "feature.xlsx");

    assert!(!reported(&package), "no fixture carries the flag");
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// What the tool writes is what it reads back.
#[test]
fn reading_reports_the_flag_the_tool_has_just_set() {
    let package = copy_of("read-set", "feature.xlsx");
    set(&package);

    assert!(reported(&package));
}

#[test]
fn setting_on_the_self_closing_element_excel_writes_moves_nothing_else() {
    let package = copy_of("self-closing", "feature.xlsx");

    let body = set(&package);

    assert_eq!(
        body["parts"],
        json!({"changed": [WORKBOOK], "added": [], "removed": []})
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[WORKBOOK]);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), WORKBOOK),
        &part_text(&package, WORKBOOK),
        r#"<calcPr calcId="191029"/>"#,
        r#"<calcPr calcId="191029" fullCalcOnLoad="1"/>"#,
    );
}

/// The attribute goes on the start tag, so whatever the element holds is
/// still there afterwards, untouched.
#[test]
fn setting_on_an_open_close_element_leaves_what_is_inside_it_alone() {
    let workspace = Workspace::new("open-close");
    let package = workbook_ending_with(
        &workspace,
        "open-close.xlsx",
        r#"<calcPr calcId="191029"><extLst><ext uri="x"/></extLst></calcPr>"#,
    );
    let before = part_text(&package, WORKBOOK);

    set(&package);

    assert_spliced(
        &before,
        &part_text(&package, WORKBOOK),
        r#"<calcPr calcId="191029">"#,
        r#"<calcPr calcId="191029" fullCalcOnLoad="1">"#,
    );
}

/// A workbook with no calculation element gets one, in the place the schema
/// gives it: after the sheets, and before anything the sequence puts later.
#[test]
fn setting_where_there_is_no_element_writes_one_after_the_sheets() {
    let workspace = Workspace::new("no-element");
    let package = workspace.sheet_package("bare.xlsx", "", r#"<c r="A1"><v>1</v></c>"#);
    let before = part_text(&package, WORKBOOK);
    assert!(!before.contains("calcPr"), "{before}");

    let body = set(&package);

    assert_eq!(body["parts"]["changed"], json!([WORKBOOK]));
    assert_spliced(
        &before,
        &part_text(&package, WORKBOOK),
        "</sheets>",
        r#"</sheets><calcPr fullCalcOnLoad="1"/>"#,
    );
}

/// Defined names come between the sheets and the calculation element, so a
/// workbook declaring any takes the element after them instead.
#[test]
fn the_element_goes_after_the_defined_names_where_there_are_any() {
    let workspace = Workspace::new("after-names");
    let package = workbook_ending_with(
        &workspace,
        "named.xlsx",
        r#"<definedNames><definedName name="Rate">Inputs!$A$1</definedName></definedNames>"#,
    );
    let before = part_text(&package, WORKBOOK);

    set(&package);

    assert_spliced(
        &before,
        &part_text(&package, WORKBOOK),
        "</definedNames>",
        r#"</definedNames><calcPr fullCalcOnLoad="1"/>"#,
    );
}

#[test]
fn setting_a_flag_already_set_changes_nothing_and_writes_nothing() {
    let package = copy_of("already-set", "feature.xlsx");
    set(&package);
    let once = std::fs::read(&package).expect("the package the test wrote is readable");

    let body = set(&package);

    assert_eq!(body["operations"][0]["changed"], json!(false));
    assert_eq!(body["parts"]["changed"], json!([]));
    assert_eq!(
        std::fs::read(&package).expect("the package the test wrote is readable"),
        once,
        "a batch that changes nothing writes nothing at all"
    );
}

/// The flag is a boolean, so a batch may ask for it off as well as on. Off is
/// the absence of the attribute, which is how Excel spells a workbook that
/// does not recalculate, so the element goes back to exactly what it was.
#[test]
fn a_batch_may_ask_for_the_flag_off_again_and_the_element_returns_to_itself() {
    let package = copy_of("off-again", "feature.xlsx");
    set(&package);
    assert!(reported(&package));

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::calculating(false)],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(envelope(&out)["parts"]["changed"], json!([WORKBOOK]));
    assert!(!reported(&package));
    assert_eq!(
        part_text(&package, WORKBOOK),
        part_text(&fixture("feature.xlsx"), WORKBOOK),
        "off is the absence of the attribute, not the attribute written 0"
    );
    // Only the entry's own timestamp says the workbook part was ever rewritten,
    // which is why this is the part-level comparison and not a byte one.
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[WORKBOOK]);
}

/// Asking for off where it is already off is a batch that changes nothing.
#[test]
fn asking_for_the_flag_off_where_it_is_already_off_changes_nothing() {
    let package = copy_of("off-already", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::calculating(false)],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(envelope(&out)["operations"][0]["changed"], json!(false));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn the_operation_reports_no_target_and_no_address() {
    let package = copy_of("no-target", "feature.xlsx");

    let body = set(&package);

    assert_eq!(
        body["operations"],
        json!([{
            "target": null,
            "name": null,
            "sheet": null,
            "cell": null,
            "address": null,
            "changed": true,
        }])
    );
}

/// Writing a cell says nothing about calculation, so the element it found is
/// the element it leaves.
#[test]
fn a_cell_write_alone_does_not_touch_the_calculation_element() {
    let package = copy_of("cell-only", "feature.xlsx");

    let out = under_json(verb::set(
        &package,
        "Inputs!A1",
        WriteType::Number,
        "5",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
    assert!(!part_text(&package, WORKBOOK).contains("fullCalcOnLoad"));
}

/// A batch holding both touches both parts, and each for its own reason.
#[test]
fn a_batch_of_a_cell_write_and_a_calc_touches_both_parts() {
    let package = copy_of("both", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::writing("Inputs!A1", WriteType::Number, "5"),
                op::calculating(true),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["parts"]["changed"],
        json!([WORKBOOK, SHEET1])
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[WORKBOOK, SHEET1]);
    assert!(part_text(&package, WORKBOOK).contains(r#"fullCalcOnLoad="1""#));
}

#[test]
fn the_flag_reaches_the_package_from_the_command_line() {
    let package = copy_of("argv", "feature.xlsx");

    let out = run(&[
        "calc",
        &package.display().to_string(),
        "--full-calc-on-load",
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert!(reported(&package));
}

/// Reading writes nothing, so the flags saying where a result goes have
/// nothing to do; saying so beats accepting them and quietly ignoring them.
#[test]
fn reading_with_out_or_dry_run_is_a_usage_error() {
    let package = copy_of("read-with-out", "feature.xlsx");
    let workspace = package.workspace();
    let file = package.display().to_string();
    let elsewhere = workspace.dir().join("elsewhere.xlsx");
    let elsewhere = elsewhere.display().to_string();

    for args in [
        vec!["calc", &file, "--dry-run", "--json"],
        vec!["calc", &file, "--out", &elsewhere, "--json"],
    ] {
        let out = run(&args);

        assert_eq!(exit_code(&out), 2, "{args:?}");
        assert_eq!(json(&out)["error"]["code"], json!("usage"), "{args:?}");
    }
    assert!(!Path::new(&elsewhere).exists());
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn reading_is_one_tab_separated_row_down_a_pipe() {
    let package = copy_of("rows", "feature.xlsx");

    let out = run(&["calc", &package.display().to_string()]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "false\n");
}

/// Setting answers the way every writing verb answers: one row per operation,
/// with the target and address columns empty because there are none.
#[test]
fn setting_is_one_row_per_operation_down_a_pipe() {
    let package = copy_of("write-rows", "feature.xlsx");

    let out = in_text(verb::run(
        &package,
        &Batch {
            operations: vec![op::calculating(true)],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "\t\t\ttrue\n");
}

#[test]
fn the_help_lists_the_operand_and_every_flag_the_verb_takes() {
    let out = run(&["calc", "--help"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    let help = stdout(&out);
    for wanted in ["FILE", "--full-calc-on-load", "--out", "--dry-run"] {
        assert!(
            help.contains(wanted),
            "calc --help must list {wanted}: {help}"
        );
    }
}
