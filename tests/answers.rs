//! The machine contract, in process: what each verb answers, and what the
//! library makes of it.
//!
//! Nothing here spawns anything. A verb's answer is built by calling the
//! library the way the binary calls it, and then rendered by the same
//! [`xlsplice::render`] the binary renders with, so the envelope these assert
//! is the envelope a caller reads off stdout, minus the process.
//!
//! What still needs a process is in `contract.rs`: what clap does before a
//! verb runs, which stream a real run writes to, and the panic hook.

mod support;

use serde_json::json;
use xlsplice::answer::{self, Answer};
use xlsplice::batch::{self, Batch, Destination, Operation};
use xlsplice::cells;
use xlsplice::envelope::JsonStyle;
use xlsplice::package::Package;
use xlsplice::render::{OutputMode, render};
use xlsplice::workbook::Workbook;
use xlsplice::worksheet::Written;

use support::{Workspace, fixture, part_text};

/// The version declared in `Cargo.toml`, read from the manifest rather than
/// from the crate's own constant, so a hardcoded version would be caught.
fn manifest_version() -> String {
    let manifest = include_str!("../Cargo.toml");
    manifest
        .lines()
        .take_while(|line| !line.starts_with("[dependencies]"))
        .find_map(|line| line.strip_prefix("version = "))
        .expect("Cargo.toml must declare a package version")
        .trim_matches('"')
        .to_owned()
}

/// The envelope a successful answer renders into.
///
/// Every verb's envelope leads with the same two members and carries no
/// `error`, so that is asserted here, once, for whichever verb is being
/// looked at, and the test that follows says what the payload holds.
fn envelope(outcome: xlsplice::Result<Answer>) -> serde_json::Value {
    let rendered = render(outcome, OutputMode::Json(JsonStyle::Compact));

    assert_eq!(rendered.exit, 0, "a successful answer exits zero");
    assert_eq!(rendered.stderr, "", "the envelope is the whole response");
    assert!(
        rendered
            .stdout
            .starts_with(r#"{"ok":true,"schema_version":1,"#),
        "the envelope must lead with ok and the schema version: {}",
        rendered.stdout
    );
    let body: serde_json::Value =
        serde_json::from_str(&rendered.stdout).expect("stdout must be one JSON document");
    assert_eq!(body.get("error"), None, "success carries no error");
    body
}

/// The text a successful answer renders into, down a pipe.
fn piped(outcome: xlsplice::Result<Answer>) -> String {
    let rendered = render(outcome, OutputMode::Text(xlsplice::TextStyle::Tsv));

    assert_eq!(rendered.stderr, "");
    rendered.stdout
}

/// The feature package, its workbook read, and the workspace holding it alive
/// for as long as the test needs it.
fn feature(label: &str) -> (Workspace, Package, Workbook) {
    let workspace = Workspace::new(label);
    let path = workspace.feature_package("feature.xlsx");
    let mut package = Package::open(&path).expect("the feature package must open");
    let workbook = Workbook::read(&mut package).expect("its workbook must be readable");
    (workspace, package, workbook)
}

#[test]
fn sheets_answers_every_sheet_with_its_state_in_workbook_order() {
    let (_workspace, _package, workbook) = feature("sheets");

    let body = envelope(answer::sheets(&workbook));

    assert_eq!(
        body["sheets"],
        json!([
            {"name": "Inputs", "state": "visible"},
            {"name": "Notes", "state": "hidden"},
            {"name": "Parameters", "state": "veryHidden"},
        ])
    );
}

#[test]
fn names_answers_the_scope_the_reference_the_anchor_and_the_reason() {
    let (_workspace, _package, workbook) = feature("names");

    let body = envelope(answer::names(&workbook));

    assert_eq!(
        body["names"][0],
        json!({
            "name": "MergedInput",
            "scope": "workbook",
            "scope_sheet": null,
            "refers_to": "Inputs!$B$2:$C$3",
            "anchor": {"sheet": "Inputs", "cell": "B2", "address": "Inputs!B2"},
            "reason": null,
        })
    );
    assert_eq!(body["names"][1]["scope"], json!("sheet"));
    assert_eq!(body["names"][1]["scope_sheet"], json!("Notes"));
    assert_eq!(body["names"][3]["anchor"], json!(null));
    assert_eq!(body["names"][3]["reason"], json!("ref_error"));
}

#[test]
fn get_answers_one_cell_per_target_in_the_order_the_targets_were_given() {
    let (_workspace, mut package, workbook) = feature("get");
    let targets = ["Inputs!A2".to_owned(), "MergedInput".to_owned()];

    let reports =
        cells::read(&mut package, &workbook, &targets).expect("both targets must resolve");
    let body = envelope(answer::cells(&reports));

    assert_eq!(
        body["cells"][0],
        json!({
            "target": "Inputs!A2",
            "name": null,
            "sheet": "Inputs",
            "cell": "A2",
            "address": "Inputs!A2",
            "type": "n",
            "value": 2.5,
            "raw": "2.5",
            "formula": null,
            "style": 0,
        })
    );
    assert_eq!(body["cells"][1]["name"], json!("MergedInput"));
    assert_eq!(body["cells"][1]["value"], json!("merged"));
}

/// A cell carries the same ten members whatever it holds, so a member that
/// does not apply is `null` rather than absent and every member of the list
/// has the same keys: a formula cell, a cell that stores no value and a
/// boolean, side by side.
#[test]
fn every_cell_answers_with_every_member_whether_it_applies_or_not() {
    let (_workspace, mut package, workbook) = feature("members");
    let targets = [
        "Inputs!E2".to_owned(),
        "Inputs!A6".to_owned(),
        "Inputs!D1".to_owned(),
    ];

    let reports = cells::read(&mut package, &workbook, &targets).expect("all three must resolve");
    let body = envelope(answer::cells(&reports));

    assert_eq!(
        body["cells"][0]["formula"],
        json!({"text": "A2*2", "role": "shared_master", "range": "E2:E3", "group": 0})
    );
    assert_eq!(
        body["cells"][1],
        json!({
            "target": "Inputs!A6",
            "name": null,
            "sheet": "Inputs",
            "cell": "A6",
            "address": "Inputs!A6",
            "type": "empty",
            "value": null,
            "raw": null,
            "formula": null,
            "style": 5,
        })
    );
    assert_eq!(body["cells"][2]["type"], json!("b"));
    assert_eq!(body["cells"][2]["value"], json!(true));

    let keys = |cell: &serde_json::Value| -> Vec<String> {
        cell.as_object()
            .expect("a cell is an object")
            .keys()
            .cloned()
            .collect()
    };
    assert_eq!(keys(&body["cells"][0]), keys(&body["cells"][1]));
    assert_eq!(keys(&body["cells"][1]), keys(&body["cells"][2]));
}

#[test]
fn set_answers_what_the_operation_did_and_what_became_of_the_parts() {
    let workspace = Workspace::new("set");
    let package = workspace.copy_of("plain.xlsx");
    let batch = Batch::of(Operation::Set {
        target: "Sheet1!A1".to_owned(),
        value: Written::number("42").expect("42 is a number"),
    });

    let report = batch::run(&package, &batch, &Destination::InPlace, false).expect("the write");
    let body = envelope(answer::written(&report));

    assert_eq!(
        body["operations"],
        json!([{
            "target": "Sheet1!A1",
            "name": null,
            "sheet": "Sheet1",
            "cell": "A1",
            "address": "Sheet1!A1",
            "changed": true,
        }])
    );
    assert_eq!(
        body["parts"],
        json!({"changed": ["xl/worksheets/sheet1.xml"], "added": [], "removed": []})
    );
    assert_eq!(body["output"], json!(package.display().to_string()));
    assert_eq!(body["dry_run"], json!(false));
    support::assert_spliced(
        &part_text(&fixture("plain.xlsx"), "xl/worksheets/sheet1.xml"),
        &part_text(&package, "xl/worksheets/sheet1.xml"),
        r#"<c r="A1"><v>1</v></c>"#,
        r#"<c r="A1"><v>42</v></c>"#,
    );
}

#[test]
fn version_answers_the_crate_version() {
    let body = envelope(answer::version());

    assert_eq!(body["version"], json!(manifest_version()));
    assert_eq!(
        piped(answer::version()),
        format!("xlsplice {}\n", manifest_version())
    );
}

/// The rows and the payload are built together and say the same thing; the
/// exact text of every verb's rows is pinned by the suites that spawn.
#[test]
fn sheets_lays_the_same_sheets_out_tab_separated() {
    let (_workspace, _package, workbook) = feature("rows");

    assert_eq!(
        piped(answer::sheets(&workbook)),
        "Inputs\tvisible\nNotes\thidden\nParameters\tveryHidden\n"
    );
}

#[test]
fn a_verb_that_found_nothing_says_nothing_at_all_down_a_pipe() {
    let workspace = Workspace::new("empty");
    let path = workspace.package("bare.xlsx", &support::workbook_xml("<sheets/>"));
    let mut package = Package::open(&path).expect("the package must open");
    let workbook = Workbook::read(&mut package).expect("its workbook must be readable");

    assert_eq!(piped(answer::sheets(&workbook)), "");
    assert_eq!(envelope(answer::sheets(&workbook))["sheets"], json!([]));
}

/// A failure renders the same way whichever verb raised it, which is what
/// lets the verbs answer with `Result` and say nothing about the shape of a
/// failure. The frozen code-to-exit table itself is held in `render`.
#[test]
fn a_verb_that_fails_is_the_error_envelope_and_the_code_it_carries() {
    let (_workspace, mut package, workbook) = feature("failure");
    let targets = ["Missing!A1".to_owned()];

    let outcome =
        cells::read(&mut package, &workbook, &targets).and_then(|reports| answer::cells(&reports));
    let rendered = render(outcome, OutputMode::Json(JsonStyle::Compact));

    assert_eq!(rendered.exit, 3);
    let body: serde_json::Value =
        serde_json::from_str(&rendered.stdout).expect("stdout must be one JSON document");
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["schema_version"], json!(1));
    assert_eq!(body["error"]["code"], json!("not_found"));
    assert_eq!(rendered.stderr, "");
}
