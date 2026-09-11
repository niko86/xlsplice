//! Contract tests for `set`: everything here observes the built binary from
//! outside, through its argv, its two streams, its exit code, and the bytes of
//! the packages it leaves behind.
//!
//! The packages are the three fixtures Excel saved. A test copies the one it
//! needs, writes to the copy, and then asks two questions of the result: which
//! parts differ from the fixture, answered by the comparator in `support`, and
//! what the worksheet became, answered against bytes the test writes out by
//! hand. Nothing here trusts the tool's own account of what it changed.

mod support;

use std::path::Path;

use support::{
    Workspace, assert_only_these_differ, assert_same_bytes, assert_same_parts, assert_spliced,
    exit_code, files_in, fixture, json, part, part_text, run, stderr, stdout,
};

const SHEET1: &str = "xl/worksheets/sheet1.xml";
const SHARED_STRINGS: &str = "xl/sharedStrings.xml";
const VBA: &str = "xl/vbaProject.bin";

/// A writable copy of a fixture, and the workspace holding it alive.
fn copy(label: &str, name: &str) -> (Workspace, String) {
    let workspace = Workspace::new(label);
    let path = workspace.copy_of(name);
    let path = path
        .to_str()
        .expect("a temporary path must be UTF-8")
        .to_owned();
    (workspace, path)
}

/// Run a `set` that must succeed, and give back its envelope.
fn set(args: &[&str]) -> serde_json::Value {
    // `--json` goes in front, because a test may end its arguments with `--`
    // and an operand, and everything after `--` is an operand.
    let mut argv = vec!["set", "--json"];
    argv.extend_from_slice(args);
    let out = run(&argv);
    assert_eq!(exit_code(&out), 0, "set {args:?}: {}", stderr(&out));
    json(&out)
}

/// The worksheet of a fixture as Excel saved it.
fn fixture_sheet(name: &str) -> String {
    part_text(&fixture(name), SHEET1)
}

/// Every fixture holds `<c r="A1"><v>1</v></c>` on its first sheet, which is
/// what makes one expected splice serve all three.
const CELL_A1: &str = r#"<c r="A1"><v>1</v></c>"#;

#[test]
fn every_type_lands_in_every_fixture_and_moves_nothing_else_in_the_package() {
    for (name, target) in [
        ("plain.xlsx", "Sheet1!A1"),
        ("macros.xlsm", "Sheet1!A1"),
        ("feature.xlsx", "Inputs!A1"),
    ] {
        for (kind, value, expected) in [
            ("number", "42.5", r#"<c r="A1"><v>42.5</v></c>"#),
            (
                "text",
                "written",
                r#"<c r="A1" t="inlineStr"><is><t>written</t></is></c>"#,
            ),
            ("bool", "true", r#"<c r="A1" t="b"><v>1</v></c>"#),
        ] {
            let (_workspace, package) = copy(&format!("{kind}-{name}"), name);

            let body = set(&[&package, target, value, "--type", kind]);

            assert_eq!(
                body["operations"][0]["changed"],
                serde_json::json!(true),
                "{name} {kind}"
            );
            assert_eq!(
                body["parts"]["changed"],
                serde_json::json!([SHEET1]),
                "{name} {kind}: only the targeted worksheet may change"
            );
            assert_only_these_differ(&fixture(name), Path::new(&package), &[SHEET1]);
            assert_spliced(
                &fixture_sheet(name),
                &part_text(Path::new(&package), SHEET1),
                CELL_A1,
                expected,
            );
        }
    }
}

#[test]
fn what_was_written_is_what_get_reads_back() {
    for (kind, given, expected_type, expected_value) in [
        ("number", "42.5", "n", serde_json::json!(42.5)),
        ("text", " kept ", "inlineStr", serde_json::json!(" kept ")),
        ("text", "a<b&c", "inlineStr", serde_json::json!("a<b&c")),
        ("bool", "0", "b", serde_json::json!(false)),
    ] {
        let (_workspace, package) = copy(&format!("read-back-{kind}-{given}"), "plain.xlsx");
        set(&[&package, "Sheet1!A1", given, "--type", kind]);

        let out = run(&["get", "--json", &package, "Sheet1!A1"]);
        assert_eq!(exit_code(&out), 0, "{kind} {given}: {}", stderr(&out));
        let cell = json(&out)["cells"][0].clone();

        assert_eq!(cell["type"], serde_json::json!(expected_type), "{given}");
        assert_eq!(cell["value"], expected_value, "{given}");
    }
}

#[test]
fn text_lands_as_an_inline_string_and_the_shared_string_table_is_not_touched() {
    let (_workspace, package) = copy("text", "plain.xlsx");

    let body = set(&[&package, "Sheet1!B1", "goodbye", "--type", "text"]);

    assert_eq!(body["parts"]["changed"], serde_json::json!([SHEET1]));
    assert_only_these_differ(&fixture("plain.xlsx"), Path::new(&package), &[SHEET1]);
    assert_spliced(
        &fixture_sheet("plain.xlsx"),
        &part_text(Path::new(&package), SHEET1),
        r#"<c r="B1" t="s"><v>0</v></c>"#,
        r#"<c r="B1" t="inlineStr"><is><t>goodbye</t></is></c>"#,
    );
    assert_eq!(
        part_text(Path::new(&package), SHARED_STRINGS),
        part_text(&fixture("plain.xlsx"), SHARED_STRINGS),
        "overwriting a shared-string cell must leave the table alone"
    );
}

#[test]
fn a_boolean_lands_as_a_boolean_cell_keeping_the_style_the_cell_carried() {
    let (_workspace, package) = copy("bool", "plain.xlsx");

    set(&[&package, "Sheet1!C1", "false", "--type", "bool"]);

    assert_only_these_differ(&fixture("plain.xlsx"), Path::new(&package), &[SHEET1]);
    assert_spliced(
        &fixture_sheet("plain.xlsx"),
        &part_text(Path::new(&package), SHEET1),
        r#"<c r="C1" s="1"><v>46276</v></c>"#,
        r#"<c r="C1" s="1" t="b"><v>0</v></c>"#,
    );
}

#[test]
fn a_write_through_a_defined_name_lands_in_the_anchor_of_the_range_it_names() {
    let (_workspace, package) = copy("name", "feature.xlsx");

    let body = set(&[&package, "MergedInput", "  padded  ", "--type", "text"]);

    assert_eq!(
        body["operations"][0]["address"],
        serde_json::json!("Inputs!B2")
    );
    assert_eq!(
        body["operations"][0]["name"],
        serde_json::json!("MergedInput")
    );
    assert_only_these_differ(&fixture("feature.xlsx"), Path::new(&package), &[SHEET1]);
    assert_spliced(
        &fixture_sheet("feature.xlsx"),
        &part_text(Path::new(&package), SHEET1),
        r#"<c r="B2" s="4"/>"#,
        r#"<c r="B2" s="4" t="inlineStr"><is><t xml:space="preserve">  padded  </t></is></c>"#,
    );
}

#[test]
fn a_number_in_the_macro_package_leaves_the_vba_project_byte_for_byte() {
    let (_workspace, package) = copy("macros", "macros.xlsm");

    // A value that starts with a minus is an operand, not a flag, and `--`
    // is what says so.
    set(&[&package, "Sheet1!A1", "--type", "number", "--", "-2.5"]);

    assert_only_these_differ(&fixture("macros.xlsm"), Path::new(&package), &[SHEET1]);
    assert_eq!(
        part(Path::new(&package), VBA),
        part(&fixture("macros.xlsm"), VBA),
        "the VBA project must pass through untouched, raw copy and all"
    );
    assert_spliced(
        &fixture_sheet("macros.xlsm"),
        &part_text(Path::new(&package), SHEET1),
        r#"<c r="A1"><v>1</v></c>"#,
        r#"<c r="A1"><v>-2.5</v></c>"#,
    );
}

#[test]
fn the_spliced_part_keeps_the_method_and_timestamp_its_entry_carried() {
    let (_workspace, package) = copy("entry", "plain.xlsx");

    set(&[&package, "Sheet1!A1", "42", "--type", "number"]);

    let (was, now) = (
        part(&fixture("plain.xlsx"), SHEET1),
        part(Path::new(&package), SHEET1),
    );
    assert_ne!(
        was.bytes, now.bytes,
        "the target part is the one that moved"
    );
    assert_eq!(
        was.method, now.method,
        "the compression method must survive"
    );
    assert_eq!(was.modified, now.modified, "the timestamp must survive");
}

#[test]
fn writing_to_another_path_leaves_the_input_byte_for_byte_as_it_was() {
    let (workspace, package) = copy("out", "plain.xlsx");
    let elsewhere = workspace.dir().join("written.xlsx");

    let body = set(&[
        &package,
        "Sheet1!A2",
        "7",
        "--type",
        "number",
        "--out",
        elsewhere.to_str().expect("a UTF-8 path"),
    ]);

    assert_eq!(
        body["output"],
        serde_json::json!(elsewhere.display().to_string())
    );
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
    assert_only_these_differ(&fixture("plain.xlsx"), &elsewhere, &[SHEET1]);
}

#[test]
fn an_in_place_write_leaves_no_temporary_file_behind() {
    let (workspace, package) = copy("temporary", "plain.xlsx");

    set(&[&package, "Sheet1!A2", "7", "--type", "number"]);

    assert_eq!(
        files_in(workspace.dir()),
        ["plain.xlsx"],
        "only the package may be left behind"
    );
}

#[test]
fn writing_the_value_that_is_already_there_changes_nothing_and_says_so() {
    let (_workspace, package) = copy("unchanged", "plain.xlsx");

    // The cell holds 2.5; 2.50 is the same number in a longer spelling, so
    // the shortest round-trip form of it is the text already in the part.
    let body = set(&[&package, "Sheet1!A2", "2.50", "--type", "number"]);

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(false));
    assert_eq!(body["parts"]["changed"], serde_json::json!([]));
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
}

#[test]
fn a_write_of_the_present_value_to_another_path_copies_the_input_byte_for_byte() {
    let (workspace, package) = copy("unchanged-out", "plain.xlsx");
    let elsewhere = workspace.dir().join("copy.xlsx");

    let body = set(&[
        &package,
        "Sheet1!D1",
        "TRUE",
        "--type",
        "bool",
        "--out",
        elsewhere.to_str().expect("a UTF-8 path"),
    ]);

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(false));
    assert_same_bytes(&fixture("plain.xlsx"), &elsewhere);
}

#[test]
fn two_runs_over_the_same_input_produce_the_same_bytes() {
    let (_first, one) = copy("determinism-one", "feature.xlsx");
    let (_second, two) = copy("determinism-two", "feature.xlsx");

    for package in [&one, &two] {
        set(&[package, "Inputs!A1", "hello", "--type", "text"]);
    }

    assert_same_bytes(Path::new(&one), Path::new(&two));
}

#[test]
fn writing_a_second_time_over_the_same_package_changes_nothing_more() {
    let (_workspace, package) = copy("idempotent", "feature.xlsx");

    set(&[&package, "Inputs!A1", "9", "--type", "number"]);
    let once = std::fs::read(&package).expect("the written package must be readable");
    let body = set(&[&package, "Inputs!A1", "9", "--type", "number"]);

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(false));
    assert_eq!(
        std::fs::read(&package).expect("the package must still be readable"),
        once,
        "a second write of the same value must not move a byte"
    );
}

#[test]
fn a_dry_run_writes_nothing_and_still_reports_everything() {
    let (workspace, package) = copy("dry-run", "plain.xlsx");

    let body = set(&[&package, "Sheet1!A1", "99", "--type", "number", "--dry-run"]);

    assert_eq!(body["dry_run"], serde_json::json!(true));
    assert_eq!(body["operations"][0]["changed"], serde_json::json!(true));
    assert_eq!(body["parts"]["changed"], serde_json::json!([SHEET1]));
    assert_eq!(
        body["output"],
        serde_json::json!(package),
        "a dry run still names where the result would have gone"
    );
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
    assert_eq!(
        files_in(workspace.dir()),
        ["plain.xlsx"],
        "a dry run must write nothing at all"
    );
}

#[test]
fn a_dry_run_to_another_path_writes_nothing_there_either() {
    let (workspace, package) = copy("dry-run-out", "plain.xlsx");
    let elsewhere = workspace.dir().join("never.xlsx");

    set(&[
        &package,
        "Sheet1!A1",
        "99",
        "--type",
        "number",
        "--out",
        elsewhere.to_str().expect("a UTF-8 path"),
        "--dry-run",
    ]);

    assert!(!elsewhere.exists(), "a dry run must create no file");
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
}

#[test]
fn a_destination_that_cannot_be_written_fails_and_leaves_the_input_alone() {
    let (workspace, package) = copy("unwritable", "plain.xlsx");
    let nowhere = workspace.dir().join("no-such-directory").join("out.xlsx");

    let out = run(&[
        "set",
        &package,
        "Sheet1!A1",
        "42",
        "--type",
        "number",
        "--out",
        nowhere.to_str().expect("a UTF-8 path"),
        "--json",
    ]);

    assert_eq!(exit_code(&out), 1);
    assert_eq!(json(&out)["ok"], serde_json::json!(false));
    assert_eq!(json(&out)["error"]["code"], serde_json::json!("internal"));
    assert!(!nowhere.exists());
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
}

#[test]
fn a_destination_that_cannot_be_replaced_leaves_no_temporary_file_behind() {
    let (workspace, package) = copy("unrenameable", "plain.xlsx");
    // A directory cannot be renamed over, so this fails at the rename rather
    // than before it, which is the other side of the landing.
    let occupied = workspace.dir().join("occupied.xlsx");
    std::fs::create_dir(&occupied).expect("a test must be able to make a directory");

    let out = run(&[
        "set",
        "--json",
        &package,
        "Sheet1!A1",
        "42",
        "--type",
        "number",
        "--out",
        occupied.to_str().expect("a UTF-8 path"),
    ]);

    assert_eq!(exit_code(&out), 1);
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
    assert_eq!(
        files_in(workspace.dir()),
        ["occupied.xlsx", "plain.xlsx"],
        "a failed landing must take its temporary file with it"
    );
}

#[test]
fn a_cell_the_sheet_does_not_hold_is_not_found_and_the_package_is_untouched() {
    let (_workspace, package) = copy("absent", "plain.xlsx");

    for target in ["Sheet1!Z1", "Sheet1!A9"] {
        let out = run(&["set", &package, target, "42", "--type", "number", "--json"]);

        assert_eq!(exit_code(&out), 3, "{target}");
        assert_eq!(json(&out)["error"]["code"], serde_json::json!("not_found"));
    }
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
}

#[test]
fn a_cell_holding_a_formula_is_refused_and_the_package_is_untouched() {
    let (_workspace, package) = copy("formula", "feature.xlsx");

    // D1 is a plain formula, E2 the master of a shared group, E3 a child.
    for (target, expected) in [
        ("Inputs!D1", "SUM(A1:A5)"),
        ("Inputs!E2", "E2:E5"),
        ("Inputs!E3", "shared group 0"),
    ] {
        let out = run(&["set", &package, target, "0", "--type", "number", "--json"]);

        assert_eq!(exit_code(&out), 4, "{target}");
        assert_eq!(json(&out)["error"]["code"], serde_json::json!("refused"));
        let message = json(&out)["error"]["message"]
            .as_str()
            .expect("a failed envelope carries a message")
            .to_owned();
        assert!(message.contains(expected), "{target}: {message}");
    }
    assert_same_bytes(&fixture("feature.xlsx"), Path::new(&package));
    assert_same_parts(&fixture("feature.xlsx"), Path::new(&package));
}

#[test]
fn a_value_that_is_not_of_the_type_asked_for_is_a_usage_error() {
    let (_workspace, package) = copy("mistyped", "plain.xlsx");

    for (kind, value) in [("number", "hello"), ("number", "NaN"), ("bool", "maybe")] {
        let out = run(&[
            "set",
            &package,
            "Sheet1!A1",
            value,
            "--type",
            kind,
            "--json",
        ]);

        assert_eq!(exit_code(&out), 2, "--type {kind} {value}");
        assert_eq!(json(&out)["error"]["code"], serde_json::json!("usage"));
    }
    assert_same_bytes(&fixture("plain.xlsx"), Path::new(&package));
}

#[test]
fn a_type_outside_the_three_is_rejected_before_the_package_is_opened() {
    let out = run(&[
        "set",
        "no-such-file.xlsx",
        "A1",
        "1",
        "--type",
        "date",
        "--json",
    ]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(json(&out)["error"]["code"], serde_json::json!("usage"));
}

#[test]
fn the_text_output_is_one_tab_separated_row_per_operation() {
    let (_workspace, package) = copy("rows", "feature.xlsx");

    let out = run(&["set", &package, "MergedInput", "x", "--type", "text"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "MergedInput\tMergedInput\tInputs!B2\ttrue\n",
        "stdout is a pipe here, so the rows are tab-separated and headerless"
    );
}
