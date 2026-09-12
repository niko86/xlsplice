//! Contract tests for `set`.
//!
//! Most of these call the library in process, through the verbs in `support`,
//! and then ask two questions of the package left behind: which parts differ
//! from the fixture, answered by the comparator in `support`, and what the
//! worksheet became, answered against bytes the test writes out by hand.
//! Nothing here trusts the tool's own account of what it changed, and the
//! comparator reads both containers itself, so a process never came into it.
//!
//! The packages are the three fixtures Excel saved. A test copies the one it
//! needs and writes to the copy.
//!
//! Three still spawn, because argv is what they are about: what clap does
//! with a `--type` outside the three, which write type each of the three
//! names reaches, and what `--` does with a value that begins with a minus.
//! The last two take `set` end to end, argv to the bytes on disk, so its
//! dispatch arm does not go unexercised.

mod support;

use std::path::{Path, PathBuf};

use support::{
    Workspace, assert_only_these_differ, assert_same_bytes, assert_same_parts, assert_spliced,
    envelope, exit_code, files_in, fixture, in_text, part, part_text, run, stderr, under_json,
    verb,
};
use xlsplice::batch::WriteType;

const SHEET1: &str = "xl/worksheets/sheet1.xml";
const SHARED_STRINGS: &str = "xl/sharedStrings.xml";
const VBA: &str = "xl/vbaProject.bin";

/// A writable copy of a fixture, and the workspace holding it alive.
fn copy(label: &str, name: &str) -> (Workspace, PathBuf) {
    let workspace = Workspace::new(label);
    let path = workspace.copy_of(name);
    (workspace, path)
}

/// A `set` that must succeed, and the envelope it answers with.
fn set(package: &Path, target: &str, kind: WriteType, value: &str) -> serde_json::Value {
    set_with(package, target, kind, value, None, false)
}

/// The same, with the two flags a writing verb takes.
fn set_with(
    package: &Path,
    target: &str,
    kind: WriteType,
    value: &str,
    out: Option<PathBuf>,
    dry_run: bool,
) -> serde_json::Value {
    let rendered = under_json(verb::set(package, target, kind, value, out, dry_run));
    assert_eq!(
        rendered.exit, 0,
        "set {target} {value}: {}",
        rendered.stdout
    );
    envelope(&rendered)
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
            (WriteType::Number, "42.5", r#"<c r="A1"><v>42.5</v></c>"#),
            (
                WriteType::Text,
                "written",
                r#"<c r="A1" t="inlineStr"><is><t>written</t></is></c>"#,
            ),
            (WriteType::Bool, "true", r#"<c r="A1" t="b"><v>1</v></c>"#),
        ] {
            let kind_name = kind.as_str();
            let (_workspace, package) = copy(&format!("{kind_name}-{name}"), name);

            let body = set(&package, target, kind, value);

            assert_eq!(
                body["operations"][0]["changed"],
                serde_json::json!(true),
                "{name} {kind_name}"
            );
            assert_eq!(
                body["parts"],
                serde_json::json!({"changed": [SHEET1], "added": [], "removed": []}),
                "{name} {kind_name}: the targeted worksheet changed and nothing else did"
            );
            assert_only_these_differ(&fixture(name), &package, &[SHEET1]);
            assert_spliced(
                &fixture_sheet(name),
                &part_text(&package, SHEET1),
                CELL_A1,
                expected,
            );
        }
    }
}

#[test]
fn what_was_written_is_what_get_reads_back() {
    for (kind, given, expected_type, expected_value) in [
        (WriteType::Number, "42.5", "n", serde_json::json!(42.5)),
        (
            WriteType::Text,
            " kept ",
            "inlineStr",
            serde_json::json!(" kept "),
        ),
        (
            WriteType::Text,
            "a<b&c",
            "inlineStr",
            serde_json::json!("a<b&c"),
        ),
        (WriteType::Bool, "0", "b", serde_json::json!(false)),
    ] {
        let kind_name = kind.as_str();
        let (_workspace, package) = copy(&format!("read-back-{kind_name}-{given}"), "plain.xlsx");
        set(&package, "Sheet1!A1", kind, given);

        let out = under_json(verb::get(&package, &["Sheet1!A1"]));
        assert_eq!(out.exit, 0, "{kind_name} {given}: {}", out.stderr);
        let cell = envelope(&out)["cells"][0].clone();

        assert_eq!(cell["type"], serde_json::json!(expected_type), "{given}");
        assert_eq!(cell["value"], expected_value, "{given}");
    }
}

#[test]
fn text_lands_as_an_inline_string_and_the_shared_string_table_is_not_touched() {
    let (_workspace, package) = copy("text", "plain.xlsx");

    let body = set(&package, "Sheet1!B1", WriteType::Text, "goodbye");

    assert_eq!(body["parts"]["changed"], serde_json::json!([SHEET1]));
    assert_only_these_differ(&fixture("plain.xlsx"), &package, &[SHEET1]);
    assert_spliced(
        &fixture_sheet("plain.xlsx"),
        &part_text(&package, SHEET1),
        r#"<c r="B1" t="s"><v>0</v></c>"#,
        r#"<c r="B1" t="inlineStr"><is><t>goodbye</t></is></c>"#,
    );
    assert_eq!(
        part_text(&package, SHARED_STRINGS),
        part_text(&fixture("plain.xlsx"), SHARED_STRINGS),
        "overwriting a shared-string cell must leave the table alone"
    );
}

#[test]
fn a_boolean_lands_as_a_boolean_cell_keeping_the_style_the_cell_carried() {
    let (_workspace, package) = copy("bool", "plain.xlsx");

    set(&package, "Sheet1!C1", WriteType::Bool, "false");

    assert_only_these_differ(&fixture("plain.xlsx"), &package, &[SHEET1]);
    assert_spliced(
        &fixture_sheet("plain.xlsx"),
        &part_text(&package, SHEET1),
        r#"<c r="C1" s="1"><v>46276</v></c>"#,
        r#"<c r="C1" s="1" t="b"><v>0</v></c>"#,
    );
}

#[test]
fn a_write_through_a_defined_name_lands_in_the_anchor_of_the_range_it_names() {
    let (_workspace, package) = copy("name", "feature.xlsx");

    let body = set(&package, "MergedInput", WriteType::Text, "  padded  ");

    assert_eq!(
        body["operations"],
        serde_json::json!([{
            "target": "MergedInput",
            "name": "MergedInput",
            "sheet": "Inputs",
            "cell": "B2",
            "address": "Inputs!B2",
            "changed": true,
        }]),
        "one result per operation, with every member of it"
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
    assert_spliced(
        &fixture_sheet("feature.xlsx"),
        &part_text(&package, SHEET1),
        r#"<c r="B2" s="4"/>"#,
        r#"<c r="B2" s="4" t="inlineStr"><is><t xml:space="preserve">  padded  </t></is></c>"#,
    );
}

#[test]
fn a_number_in_the_macro_package_leaves_the_vba_project_byte_for_byte() {
    let (_workspace, package) = copy("macros", "macros.xlsm");

    set(&package, "Sheet1!A1", WriteType::Number, "-2.5");

    assert_only_these_differ(&fixture("macros.xlsm"), &package, &[SHEET1]);
    assert_eq!(
        part(&package, VBA),
        part(&fixture("macros.xlsm"), VBA),
        "the VBA project must pass through untouched, raw copy and all"
    );
    assert_spliced(
        &fixture_sheet("macros.xlsm"),
        &part_text(&package, SHEET1),
        r#"<c r="A1"><v>1</v></c>"#,
        r#"<c r="A1"><v>-2.5</v></c>"#,
    );
}

/// Which write type a `--type` name reaches is clap's to decide, and only a
/// real argv crosses that. Which bytes each type lands is the business of the
/// tests above; that the name arrives as the right one is the business of
/// this one, so every name the library offers is driven from its own list.
#[test]
fn each_write_type_reads_its_value_the_way_the_command_line_says() {
    for (kind, value, expected) in [
        (WriteType::Number, "42.5", r#"<c r="A1"><v>42.5</v></c>"#),
        (
            WriteType::Text,
            "written",
            r#"<c r="A1" t="inlineStr"><is><t>written</t></is></c>"#,
        ),
        (WriteType::Bool, "true", r#"<c r="A1" t="b"><v>1</v></c>"#),
    ] {
        let kind = kind.as_str();
        let (_workspace, package) = copy(&format!("argv-{kind}"), "plain.xlsx");

        let out = run(&[
            "set",
            package.to_str().expect("a UTF-8 path"),
            "Sheet1!A1",
            value,
            "--type",
            kind,
        ]);

        assert_eq!(exit_code(&out), 0, "{kind}: {}", stderr(&out));
        assert_spliced(
            &fixture_sheet("plain.xlsx"),
            &part_text(&package, SHEET1),
            CELL_A1,
            expected,
        );
    }
}

/// A value that starts with a minus is an operand, not a flag, and `--` is
/// what says so. Only a real argv can be wrong about that, so this one runs
/// the binary, and takes `set` end to end while it is there.
#[test]
fn a_value_beginning_with_a_minus_is_an_operand_after_the_double_dash() {
    let (_workspace, package) = copy("negative", "plain.xlsx");

    let out = run(&[
        "set",
        package.to_str().expect("a UTF-8 path"),
        "Sheet1!A1",
        "--type",
        "number",
        "--",
        "-2.5",
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_spliced(
        &fixture_sheet("plain.xlsx"),
        &part_text(&package, SHEET1),
        CELL_A1,
        r#"<c r="A1"><v>-2.5</v></c>"#,
    );
}

#[test]
fn the_spliced_part_keeps_the_method_and_timestamp_its_entry_carried() {
    let (_workspace, package) = copy("entry", "plain.xlsx");

    set(&package, "Sheet1!A1", WriteType::Number, "42");

    let (was, now) = (part(&fixture("plain.xlsx"), SHEET1), part(&package, SHEET1));
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

    let body = set_with(
        &package,
        "Sheet1!A2",
        WriteType::Number,
        "7",
        Some(elsewhere.clone()),
        false,
    );

    assert_eq!(
        body["operations"],
        serde_json::json!([{
            "target": "Sheet1!A2",
            "name": null,
            "sheet": "Sheet1",
            "cell": "A2",
            "address": "Sheet1!A2",
            "changed": true,
        }]),
        "an address goes through no defined name at all"
    );
    assert_eq!(
        body["output"],
        serde_json::json!(elsewhere.display().to_string())
    );
    assert_eq!(
        body["dry_run"],
        serde_json::json!(false),
        "a write that landed was no dry run"
    );
    assert_same_bytes(&fixture("plain.xlsx"), &package);
    assert_only_these_differ(&fixture("plain.xlsx"), &elsewhere, &[SHEET1]);
}

#[test]
fn an_in_place_write_leaves_no_temporary_file_behind() {
    let (workspace, package) = copy("temporary", "plain.xlsx");

    set(&package, "Sheet1!A2", WriteType::Number, "7");

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
    let body = set(&package, "Sheet1!A2", WriteType::Number, "2.50");

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(false));
    assert_eq!(body["parts"]["changed"], serde_json::json!([]));
    assert_same_bytes(&fixture("plain.xlsx"), &package);
}

#[test]
fn a_write_of_the_present_value_to_another_path_copies_the_input_byte_for_byte() {
    let (workspace, package) = copy("unchanged-out", "plain.xlsx");
    let elsewhere = workspace.dir().join("copy.xlsx");

    let body = set_with(
        &package,
        "Sheet1!D1",
        WriteType::Bool,
        "TRUE",
        Some(elsewhere.clone()),
        false,
    );

    assert_eq!(body["operations"][0]["changed"], serde_json::json!(false));
    assert_same_bytes(&fixture("plain.xlsx"), &elsewhere);
}

#[test]
fn two_runs_over_the_same_input_produce_the_same_bytes() {
    let (_first, one) = copy("determinism-one", "feature.xlsx");
    let (_second, two) = copy("determinism-two", "feature.xlsx");

    for package in [&one, &two] {
        set(package, "Inputs!A1", WriteType::Text, "hello");
    }

    assert_same_bytes(&one, &two);
}

#[test]
fn writing_a_second_time_over_the_same_package_changes_nothing_more() {
    let (_workspace, package) = copy("idempotent", "feature.xlsx");

    set(&package, "Inputs!A1", WriteType::Number, "9");
    let once = std::fs::read(&package).expect("the written package must be readable");
    let body = set(&package, "Inputs!A1", WriteType::Number, "9");

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

    let body = set_with(&package, "Sheet1!A1", WriteType::Number, "99", None, true);

    assert_eq!(body["dry_run"], serde_json::json!(true));
    assert_eq!(body["operations"][0]["changed"], serde_json::json!(true));
    assert_eq!(body["parts"]["changed"], serde_json::json!([SHEET1]));
    assert_eq!(
        body["output"],
        serde_json::json!(package.display().to_string()),
        "a dry run still names where the result would have gone"
    );
    assert_same_bytes(&fixture("plain.xlsx"), &package);
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

    set_with(
        &package,
        "Sheet1!A1",
        WriteType::Number,
        "99",
        Some(elsewhere.clone()),
        true,
    );

    assert!(!elsewhere.exists(), "a dry run must create no file");
    assert_same_bytes(&fixture("plain.xlsx"), &package);
}

#[test]
fn a_destination_that_cannot_be_written_fails_and_leaves_the_input_alone() {
    let (workspace, package) = copy("unwritable", "plain.xlsx");
    let nowhere = workspace.dir().join("no-such-directory").join("out.xlsx");

    let out = under_json(verb::set(
        &package,
        "Sheet1!A1",
        WriteType::Number,
        "42",
        Some(nowhere.clone()),
        false,
    ));

    assert_eq!(out.exit, 1);
    assert_eq!(envelope(&out)["ok"], serde_json::json!(false));
    assert_eq!(
        envelope(&out)["error"]["code"],
        serde_json::json!("internal")
    );
    assert!(!nowhere.exists());
    assert_same_bytes(&fixture("plain.xlsx"), &package);
}

#[test]
fn a_destination_that_cannot_be_replaced_leaves_no_temporary_file_behind() {
    let (workspace, package) = copy("unrenameable", "plain.xlsx");
    // A directory cannot be renamed over, so this fails at the rename rather
    // than before it, which is the other side of the landing.
    let occupied = workspace.dir().join("occupied.xlsx");
    std::fs::create_dir(&occupied).expect("a test must be able to make a directory");

    let out = under_json(verb::set(
        &package,
        "Sheet1!A1",
        WriteType::Number,
        "42",
        Some(occupied.clone()),
        false,
    ));

    assert_eq!(out.exit, 1);
    assert_same_bytes(&fixture("plain.xlsx"), &package);
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
        let out = under_json(verb::set(
            &package,
            target,
            WriteType::Number,
            "42",
            None,
            false,
        ));

        assert_eq!(out.exit, 3, "{target}");
        assert_eq!(
            envelope(&out)["error"]["code"],
            serde_json::json!("not_found")
        );
    }
    assert_same_bytes(&fixture("plain.xlsx"), &package);
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
        let out = under_json(verb::set(
            &package,
            target,
            WriteType::Number,
            "0",
            None,
            false,
        ));

        assert_eq!(out.exit, 4, "{target}");
        assert_eq!(
            envelope(&out)["error"]["code"],
            serde_json::json!("refused")
        );
        let message = envelope(&out)["error"]["message"]
            .as_str()
            .expect("a failed envelope carries a message")
            .to_owned();
        assert!(message.contains(expected), "{target}: {message}");
    }
    assert_same_bytes(&fixture("feature.xlsx"), &package);
    assert_same_parts(&fixture("feature.xlsx"), &package);
}

/// The value is read inside the batch now, so a value that is not of its type
/// is a failure of the run rather than of the command line that built it: the
/// package is opened and nothing in it is touched.
#[test]
fn a_value_that_is_not_of_the_type_asked_for_is_a_usage_error() {
    let (_workspace, package) = copy("mistyped", "plain.xlsx");

    for (kind, value) in [
        (WriteType::Number, "hello"),
        (WriteType::Number, "NaN"),
        (WriteType::Bool, "maybe"),
    ] {
        let out = under_json(verb::set(&package, "Sheet1!A1", kind, value, None, false));

        assert_eq!(out.exit, 2, "--type {} {value}", kind.as_str());
        assert_eq!(envelope(&out)["error"]["code"], serde_json::json!("usage"));
        let message = envelope(&out)["error"]["message"]
            .as_str()
            .expect("a failed envelope carries a message")
            .to_owned();
        assert!(
            message.starts_with("operation at index 0 (set Sheet1!A1): "),
            "the failure must name the operation it came from: {message}"
        );
    }
    assert_same_bytes(&fixture("plain.xlsx"), &package);
}

/// clap reads `--type` before a verb is reached, so a type outside the three
/// never becomes a write at all, and only a real argv can say so.
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
    assert_eq!(
        support::json(&out)["error"]["code"],
        serde_json::json!("usage")
    );
}

#[test]
fn the_text_output_is_one_tab_separated_row_per_operation() {
    let (_workspace, package) = copy("rows", "feature.xlsx");

    let out = in_text(verb::set(
        &package,
        "MergedInput",
        WriteType::Text,
        "x",
        None,
        false,
    ));

    assert_eq!(out.exit, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout, "MergedInput\tMergedInput\tInputs!B2\ttrue\n",
        "stdout is a pipe here, so the rows are tab-separated and headerless"
    );
}
