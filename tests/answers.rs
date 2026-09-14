//! What every verb's answer has in common, in process.
//!
//! What each verb answers with is asserted beside that verb, in
//! `read_verbs.rs`, `get.rs` and `set.rs`. Here is what is true of all of them
//! at once and of none of them in particular: the two members every envelope
//! leads with, the absence of `error` on success, what a failure puts there
//! instead, what an answer with nothing in it says, and the rule that keeps a
//! list's members the same shape as each other.
//!
//! Nothing here spawns anything. `version` has no suite of its own, so its
//! answer is asserted here too.

mod support;

use std::path::PathBuf;

use serde_json::json;
use support::{Workspace, envelope, in_text, targets, under_json};
use xlsplice::answer;
use xlsplice::batch::Destination;
use xlsplice::batch::{OperationReport, Parts, Report, WriteType};
use xlsplice::reference::{Address, Cell};
use xlsplice::verb::{self, Trace};

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

#[test]
fn every_verb_leads_with_ok_and_the_schema_version_and_carries_no_error() {
    let workspace = Workspace::new("lead");
    let package = workspace.feature_package("feature.xlsx");
    let writable = workspace.feature_package("writable.xlsx");
    let answers = [
        ("sheets", verb::sheets(&package, &Trace::Off)),
        ("names", verb::names(&package, &Trace::Off)),
        (
            "get",
            verb::get(&package, &targets(&["Inputs!A1"]), &Trace::Off),
        ),
        ("version", answer::version()),
        (
            "set",
            verb::set(
                &writable,
                "Inputs!A1",
                WriteType::Number,
                "42",
                false,
                &Destination::InPlace,
                false,
                &Trace::Off,
            ),
        ),
    ];

    for (verb, outcome) in answers {
        let out = under_json(outcome);

        assert_eq!(out.exit, 0, "{verb}");
        assert_eq!(out.stderr, "", "{verb}: the envelope is the whole response");
        assert!(
            out.stdout.starts_with(r#"{"ok":true,"schema_version":1,"#),
            "{verb} must lead with ok and the schema version: {}",
            out.stdout
        );
        assert_eq!(envelope(&out).get("error"), None, "{verb} succeeded");
    }
}

/// A field that does not apply is `null` rather than absent, so a consumer
/// reading the second member of a list finds the keys the first one had. The
/// cells of one `get` say it: a shared-formula master, a cell that stores no
/// value, and a boolean.
#[test]
fn every_member_of_a_list_carries_the_keys_the_others_carry() {
    let workspace = Workspace::new("members");
    let package = workspace.feature_package("feature.xlsx");

    let body = envelope(&under_json(verb::get(
        &package,
        &targets(&["Inputs!E2", "Inputs!A6", "Inputs!D1"]),
        &Trace::Off,
    )));

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
        }),
        "a cell holding nothing still answers with every member"
    );
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

/// An operation that names no cell answers with a null target and a null
/// address, and one that names a cell answers as it always did. `calc` is the
/// first operation that names none; the report is still written out here, so
/// that the shape is asserted whatever verbs happen to exist.
#[test]
fn an_operation_naming_no_cell_reports_a_null_address_and_one_naming_one_does_not() {
    let report = Report {
        operations: vec![
            OperationReport {
                target: Some("Inputs!A1".to_owned()),
                name: None,
                address: Some(Address {
                    sheet: "Inputs".to_owned(),
                    cell: Cell::parse("A1").expect("the test asks for a cell"),
                }),
                changed: true,
            },
            OperationReport {
                target: None,
                name: None,
                address: None,
                changed: true,
            },
        ],
        parts: Parts {
            changed: vec!["xl/worksheets/sheet1.xml".to_owned()],
            added: Vec::new(),
            removed: vec!["xl/calcChain.xml".to_owned()],
        },
        output: PathBuf::from("book.xlsx"),
        dry_run: false,
    };

    let body = envelope(&under_json(answer::written(&report)));

    assert_eq!(
        body["operations"][1],
        json!({
            "target": null,
            "name": null,
            "sheet": null,
            "cell": null,
            "address": null,
            "changed": true,
        }),
        "an operation that names no cell still answers with every member"
    );
    assert_eq!(
        body["operations"][0],
        json!({
            "target": "Inputs!A1",
            "name": null,
            "sheet": "Inputs",
            "cell": "A1",
            "address": "Inputs!A1",
            "changed": true,
        }),
        "an operation that names a cell answers as it always did"
    );
    assert_eq!(
        body["parts"],
        json!({
            "changed": ["xl/worksheets/sheet1.xml"],
            "added": [],
            "removed": ["xl/calcChain.xml"],
        })
    );
    assert_eq!(
        in_text(answer::written(&report)).stdout,
        "Inputs!A1\t\tInputs!A1\ttrue\n\t\t\ttrue\n",
        "the target and address columns are empty for an operation naming no cell"
    );
}

#[test]
fn version_answers_the_crate_version_in_both_shapes() {
    assert_eq!(
        envelope(&under_json(answer::version()))["version"],
        json!(manifest_version())
    );
    assert_eq!(
        in_text(answer::version()).stdout,
        format!("xlsplice {}\n", manifest_version())
    );
}

#[test]
fn a_verb_that_found_nothing_says_nothing_at_all_down_a_pipe() {
    let workspace = Workspace::new("empty");
    let package = workspace.package("bare.xlsx", &support::workbook_xml("<sheets/>"));

    assert_eq!(in_text(verb::sheets(&package, &Trace::Off)).stdout, "");
    assert_eq!(
        envelope(&under_json(verb::sheets(&package, &Trace::Off)))["sheets"],
        json!([])
    );
}

/// A failure renders the same way whichever verb raised it, which is what
/// lets every verb answer with a `Result` and say nothing about the shape of
/// one. The frozen code-to-exit table itself is held in `render`.
#[test]
fn a_verb_that_fails_is_the_error_envelope_and_the_code_it_carries() {
    let workspace = Workspace::new("failure");
    let package = workspace.feature_package("feature.xlsx");

    let out = under_json(verb::get(&package, &targets(&["Missing!A1"]), &Trace::Off));

    assert_eq!(out.exit, 3);
    let body = envelope(&out);
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["schema_version"], json!(1));
    assert_eq!(body["error"]["code"], json!("not_found"));
    assert_eq!(
        out.stderr, "",
        "under --json the envelope is the whole of it"
    );
}
