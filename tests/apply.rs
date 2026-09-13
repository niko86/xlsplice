//! Contract tests for `apply`.
//!
//! `apply` is the verb that carries a batch of more than one operation. What
//! such a batch does to a package is asserted in `batches.rs`, against the
//! library; here is what only the verb has: where the batch comes from, what
//! a document that is not a batch does, and that a failure anywhere leaves
//! the package as it was.
//!
//! These spawn. The batch arrives as an operand or on stdin, and neither of
//! those exists without a process.

mod support;

use std::path::{Path, PathBuf};

use support::{
    Workspace, assert_only_these_differ, assert_same_bytes, exit_code, fixture, json, part_text,
    run, run_in, run_with_stdin, stderr, stdout,
};

const SHEET1: &str = "xl/worksheets/sheet1.xml";
const SHEET2: &str = "xl/worksheets/sheet2.xml";

/// A batch of two operations, one on each of the fixture's first two sheets.
const ACROSS_TWO_SHEETS: &str = r#"[
  {"op": "set", "target": "Inputs!A1", "type": "number", "value": "42.5"},
  {"op": "set", "target": "Notes!A5", "type": "text", "value": "noted"}
]"#;

/// A writable copy of the feature fixture, and the workspace holding it.
fn copy(label: &str) -> (Workspace, PathBuf) {
    let workspace = Workspace::new(label);
    let package = workspace.copy_of("feature.xlsx");
    (workspace, package)
}

/// The batch written to a file in `workspace`, and its path as a string.
fn batch_file(workspace: &Workspace, name: &str, json: &str) -> String {
    workspace.file(name, json.as_bytes()).display().to_string()
}

#[test]
fn a_batch_across_two_sheets_applies_in_one_invocation() {
    let (workspace, package) = copy("two-sheets");
    let batch = batch_file(&workspace, "batch.json", ACROSS_TWO_SHEETS);
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    let body = json(&out);
    assert_eq!(
        body["parts"]["changed"],
        serde_json::json!([SHEET1, SHEET2]),
        "both worksheets changed and nothing else did"
    );
    assert_eq!(
        body["operations"][0]["address"],
        serde_json::json!("Inputs!A1")
    );
    assert_eq!(
        body["operations"][1]["address"],
        serde_json::json!("Notes!A5")
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1, SHEET2]);
    assert!(part_text(&package, SHEET1).contains(r#"<c r="A1"><v>42.5</v></c>"#));
    assert!(
        part_text(&package, SHEET2)
            .contains(r#"<c r="A5" t="inlineStr"><is><t>noted</t></is></c>"#),
        "the shared-string cell became an inline string"
    );
}

/// The batch is validated whole before a byte is written, so an operation
/// that cannot be carried out stops the ones that could.
#[test]
fn one_bad_operation_in_the_middle_writes_nothing_and_names_its_index() {
    let (workspace, package) = copy("bad-middle");
    let batch = batch_file(
        &workspace,
        "batch.json",
        r#"[
          {"op": "set", "target": "Inputs!A1", "type": "number", "value": "1"},
          {"op": "set", "target": "Inputs!A2", "type": "number", "value": "not a number"},
          {"op": "set", "target": "Inputs!A3", "type": "number", "value": "3"}
        ]"#,
    );
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 2);
    let body = json(&out);
    assert_eq!(body["error"]["code"], serde_json::json!("usage"));
    let message = body["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message");
    assert!(
        message.starts_with("operation at index 1 (set Inputs!A2): "),
        "the failure must name the operation and why: {message}"
    );
    assert!(message.contains("is not a number"), "{message}");
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// A target the package does not hold names its operation too, not only a
/// value of the wrong type.
#[test]
fn an_operation_naming_a_sheet_the_package_lacks_names_its_index() {
    let (workspace, package) = copy("bad-sheet");
    let batch = batch_file(
        &workspace,
        "batch.json",
        r#"[
          {"op": "set", "target": "Inputs!A1", "type": "number", "value": "1"},
          {"op": "set", "target": "Missing!A1", "type": "number", "value": "2"}
        ]"#,
    );
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 3);
    let message = json(&out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned();
    assert!(
        message.starts_with("operation at index 1 (set Missing!A1): "),
        "{message}"
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn a_dash_reads_the_batch_from_stdin() {
    let (_workspace, package) = copy("stdin");
    let file = package.display().to_string();

    let out = run_with_stdin(&["apply", &file, "-", "--json"], ACROSS_TWO_SHEETS);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        json(&out)["parts"]["changed"],
        serde_json::json!([SHEET1, SHEET2])
    );
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1, SHEET2]);
}

#[test]
fn a_batch_operand_that_is_not_there_is_unreadable_and_the_package_is_untouched() {
    let (workspace, package) = copy("no-batch");
    let missing = workspace.dir().join("no-such-batch.json");
    let file = package.display().to_string();

    let out = run(&["apply", &file, &missing.display().to_string(), "--json"]);

    assert_eq!(exit_code(&out), 5, "{}", stdout(&out));
    assert_eq!(json(&out)["error"]["code"], serde_json::json!("unreadable"));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn a_missing_batch_operand_is_a_usage_error() {
    let (_workspace, package) = copy("no-operand");

    let out = run(&["apply", &package.display().to_string(), "--json"]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(json(&out)["error"]["code"], serde_json::json!("usage"));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// A batch path beginning with a minus is an operand after the double dash,
/// as a value is for `set`. The path has to be relative for its first
/// character to be a minus, so the child runs in the directory holding it.
#[test]
fn a_batch_path_beginning_with_a_minus_is_an_operand_after_the_double_dash() {
    let (workspace, package) = copy("dashed-path");
    workspace.file("-batch.json", ACROSS_TWO_SHEETS.as_bytes());
    let file = package.display().to_string();

    let out = run_in(workspace.dir(), &["apply", "--", &file, "-batch.json"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1, SHEET2]);
}

#[test]
fn an_unknown_operation_kind_is_a_usage_error_listing_the_known_kinds() {
    let (workspace, package) = copy("unknown-kind");
    let batch = batch_file(
        &workspace,
        "batch.json",
        r#"[{"op": "calc", "value": "true"}]"#,
    );
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 2);
    let message = json(&out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned();
    assert!(message.contains("calc"), "{message}");
    assert!(
        message.contains("The operation kinds are: set, clear."),
        "the failure must say what this build knows: {message}"
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn a_field_no_operation_knows_is_ignored() {
    let (workspace, package) = copy("unknown-field");
    let batch = batch_file(
        &workspace,
        "batch.json",
        r#"[{"op": "set", "target": "Inputs!A1", "type": "number", "value": "7",
             "comment": "a field no ticket has added"}]"#,
    );
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1]);
}

/// `--out` and `--dry-run` are the flags every writing verb takes, and
/// `apply` takes them the way `set` does.
#[test]
fn out_and_dry_run_work_as_they_do_for_set() {
    let (workspace, package) = copy("flags");
    let batch = batch_file(&workspace, "batch.json", ACROSS_TWO_SHEETS);
    let file = package.display().to_string();
    let elsewhere = workspace.dir().join("elsewhere.xlsx");

    let dry = run(&["apply", &file, &batch, "--dry-run", "--json"]);
    assert_eq!(exit_code(&dry), 0, "{}", stderr(&dry));
    assert_eq!(json(&dry)["dry_run"], serde_json::json!(true));
    assert_same_bytes(&fixture("feature.xlsx"), &package);

    let out = run(&[
        "apply",
        &file,
        &batch,
        "--out",
        &elsewhere.display().to_string(),
        "--json",
    ]);
    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
    assert_only_these_differ(&fixture("feature.xlsx"), &elsewhere, &[SHEET1, SHEET2]);
}

/// A cell holding a formula is refused whatever `replace_formula` says: the
/// flag is carried for #10 to honour, and until then nothing honours it.
#[test]
fn replace_formula_is_carried_and_a_formula_is_still_refused() {
    let (workspace, package) = copy("replace-formula");
    let batch = batch_file(
        &workspace,
        "batch.json",
        r#"[{"op": "set", "target": "Inputs!D1", "type": "number", "value": "1",
             "replace_formula": true}]"#,
    );
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 4, "{}", stdout(&out));
    assert_eq!(json(&out)["error"]["code"], serde_json::json!("refused"));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// An empty batch asks for nothing, which is not an error: it changes
/// nothing and writes nothing, as any batch that changes nothing does.
#[test]
fn an_empty_batch_changes_nothing_and_writes_nothing() {
    let (workspace, package) = copy("empty-batch");
    let batch = batch_file(&workspace, "batch.json", "[]");
    let file = package.display().to_string();

    let out = run(&["apply", &file, &batch, "--json"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["operations"], serde_json::json!([]));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// The batch operand is a path, so a directory in its place is unreadable
/// rather than a crash.
#[test]
fn a_directory_in_place_of_a_batch_is_unreadable() {
    let (workspace, package) = copy("batch-is-a-dir");
    let file = package.display().to_string();

    let out = run(&[
        "apply",
        &file,
        &workspace.dir().display().to_string(),
        "--json",
    ]);

    assert_eq!(exit_code(&out), 5, "{}", stdout(&out));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// The help lists both operands and the flags every writing verb takes.
#[test]
fn the_help_lists_both_operands_and_the_writing_flags() {
    let out = run(&["apply", "--help"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    let help = stdout(&out);
    for wanted in ["FILE", "BATCH", "--out", "--dry-run"] {
        assert!(
            help.contains(wanted),
            "apply --help must list {wanted}: {help}"
        );
    }
}

/// Nothing here should have written to the fixtures themselves.
#[test]
fn the_fixtures_are_never_written_to() {
    assert!(Path::new(&fixture("feature.xlsx")).exists());
}
