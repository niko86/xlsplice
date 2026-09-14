//! Contract tests for `sheets` and `names`.
//!
//! Most of these call the library in process, through the verbs in `support`,
//! and read what `render` put on the two streams: one envelope, or the
//! tab-separated form a pipe gets. The aligned form is not here and never
//! was — `render`'s own unit tests hold it, being able to say what a terminal
//! would have been given.
//!
//! Three still spawn, because the process is what they are about: what clap
//! does with no package to read, what `--` does with the one that follows it,
//! and what `--quiet` and `--verbose` do to stderr. Between them the two verbs
//! are each crossed end to end, argv to stdout, so no dispatch arm goes
//! unexercised.

mod support;

use std::path::Path;

use serde_json::json;
use support::{
    CONTENT_TYPES, CONTENT_TYPES_PART, Copied, ROOT_RELS, ROOT_RELS_PART, WORKBOOK_PART, Workspace,
    built, envelope, exit_code, feature_workbook, in_text, run, stderr, stdout, under_json,
    workbook_xml,
};
use xlsplice::Result;
use xlsplice::answer::Answer;
use xlsplice::verb::{self, Trace};

/// The feature workbook, written to a package, and the workspace holding it
/// alive for as long as the test needs it.
fn feature_package(label: &str) -> Copied {
    built(label, |w| w.package("feature.xlsx", &feature_workbook()))
}

/// A read verb: a package in, an answer out.
type Read = fn(&Path, &Trace) -> Result<Answer>;

/// Both read verbs, to say of each of them what is true of either.
const BOTH: [(&str, Read); 2] = [("sheets", verb::sheets), ("names", verb::names)];

#[test]
fn sheets_reports_every_sheet_with_its_state_in_workbook_order() {
    let package = feature_package("sheets-order");

    let out = in_text(verb::sheets(&package, &Trace::Off));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout,
        "Inputs\tvisible\nNotes\thidden\nParameters\tveryHidden\n"
    );
}

#[test]
fn sheets_json_carries_the_same_sheets_in_the_envelope() {
    let package = feature_package("sheets-json");

    let out = under_json(verb::sheets(&package, &Trace::Off));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stderr, "");
    let body = envelope(&out);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["schema_version"], json!(1));
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
fn names_reports_the_scope_the_reference_the_anchor_and_the_reason() {
    let package = feature_package("names-rows");

    let out = in_text(verb::names(&package, &Trace::Off));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout,
        concat!(
            "MergedInput\tworkbook\tInputs!$B$2:$C$3\tInputs!B2\t\n",
            "LocalNote\tNotes\tNotes!$A$5\tNotes!A5\t\n",
            "LoudCase\tworkbook\tINPUTS!$E$1\tInputs!E1\t\n",
            "Gone\tworkbook\t#REF!\t\tref_error\n",
            "Rate\tworkbook\t0.175\t\tconstant\n",
            "Total\tworkbook\tSUM(Inputs!$D$1:$D$9)\t\tformula\n",
        )
    );
}

#[test]
fn a_workbook_scoped_name_anchors_at_the_merged_ranges_top_left() {
    let package = feature_package("names-merged");

    let body = envelope(&under_json(verb::names(&package, &Trace::Off)));
    let merged = &body["names"][0];

    assert_eq!(merged["name"], json!("MergedInput"));
    assert_eq!(merged["scope"], json!("workbook"));
    assert_eq!(merged["scope_sheet"], json!(null));
    assert_eq!(merged["refers_to"], json!("Inputs!$B$2:$C$3"));
    assert_eq!(
        merged["anchor"],
        json!({"sheet": "Inputs", "cell": "B2", "address": "Inputs!B2"})
    );
    assert_eq!(merged["reason"], json!(null));
}

#[test]
fn a_sheet_scoped_name_reports_the_sheet_it_is_scoped_to() {
    let package = feature_package("names-local");

    let local = envelope(&under_json(verb::names(&package, &Trace::Off)))["names"][1].clone();

    assert_eq!(local["name"], json!("LocalNote"));
    assert_eq!(local["scope"], json!("sheet"));
    assert_eq!(local["scope_sheet"], json!("Notes"));
    assert_eq!(local["anchor"]["address"], json!("Notes!A5"));
}

#[test]
fn a_name_that_resolves_to_no_cell_carries_the_reason_and_no_anchor() {
    let package = feature_package("names-reasons");

    let body = envelope(&under_json(verb::names(&package, &Trace::Off)));
    let reasons: Vec<_> = body["names"]
        .as_array()
        .expect("names is a list")
        .iter()
        .filter(|name| name["anchor"].is_null())
        .map(|name| (name["name"].clone(), name["reason"].clone()))
        .collect();

    assert_eq!(
        reasons,
        [
            (json!("Gone"), json!("ref_error")),
            (json!("Rate"), json!("constant")),
            (json!("Total"), json!("formula")),
        ]
    );
}

#[test]
fn a_reference_spelled_in_another_case_resolves_to_the_packages_spelling() {
    let package = feature_package("names-case");

    let loud = envelope(&under_json(verb::names(&package, &Trace::Off)))["names"][2].clone();

    assert_eq!(
        loud["refers_to"],
        json!("INPUTS!$E$1"),
        "the reference is reported as the package holds it"
    );
    assert_eq!(loud["anchor"]["sheet"], json!("Inputs"));
    assert_eq!(loud["anchor"]["address"], json!("Inputs!E1"));
}

#[test]
fn an_anchor_on_a_sheet_needing_quotes_is_quoted() {
    let workspace = Workspace::new("names-quoted");
    let package = workspace.package(
        "quoted.xlsx",
        &workbook_xml(
            r#"<sheets><sheet name="My Sheet" sheetId="1"/></sheets>
               <definedNames><definedName name="Spaced">'My Sheet'!$A$1</definedName></definedNames>"#,
        ),
    );

    let out = in_text(verb::names(&package, &Trace::Off));

    assert_eq!(out.exit, 0);
    assert!(
        out.stdout.contains("'My Sheet'!A1"),
        "stdout: {}",
        out.stdout
    );
}

#[test]
fn a_package_with_no_defined_names_says_nothing_down_the_pipe() {
    let workspace = Workspace::new("names-empty");
    let package = workspace.package(
        "bare.xlsx",
        &workbook_xml(r#"<sheets><sheet name="Sheet1" sheetId="1"/></sheets>"#),
    );

    let out = in_text(verb::names(&package, &Trace::Off));

    assert_eq!(out.exit, 0);
    assert_eq!(
        out.stdout, "",
        "an empty list is no lines at all, not a blank one"
    );
}

/// A package another tool has mangled can lose its root relationships. The
/// workbook part is still where Excel always puts it, and a read verb has no
/// reason to refuse it.
#[test]
fn a_package_whose_root_relationships_are_missing_is_read_from_the_usual_place() {
    let workspace = Workspace::new("no-rels");
    let workbook = workbook_xml(r#"<sheets><sheet name="Only" sheetId="1"/></sheets>"#);
    let package = workspace.zip(
        "norels.xlsx",
        &[
            (CONTENT_TYPES_PART, CONTENT_TYPES),
            (WORKBOOK_PART, &workbook),
        ],
    );

    let out = in_text(verb::sheets(&package, &Trace::Off));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stdout, "Only\tvisible\n");
}

#[test]
fn a_path_that_is_not_a_package_is_unreadable() {
    let workspace = Workspace::new("unreadable");
    let cases = [
        (
            "a text file",
            workspace.file("notes.txt", b"this is not a package"),
        ),
        ("an empty file", workspace.file("empty.xlsx", b"")),
        (
            "a zip without a workbook",
            workspace.zip("no-workbook.xlsx", &[("readme.txt", "nothing here")]),
        ),
        (
            "a zip whose relationships point at a part it lacks",
            workspace.zip(
                "dangling.xlsx",
                &[
                    (CONTENT_TYPES_PART, CONTENT_TYPES),
                    (ROOT_RELS_PART, ROOT_RELS),
                ],
            ),
        ),
        (
            "a package whose workbook part is not XML",
            workspace.zip(
                "broken.xlsx",
                &[
                    (CONTENT_TYPES_PART, CONTENT_TYPES),
                    (ROOT_RELS_PART, ROOT_RELS),
                    (WORKBOOK_PART, "<workbook><sheets>"),
                ],
            ),
        ),
        (
            "a path that does not exist",
            workspace.dir().join("absent.xlsx"),
        ),
        ("a directory", workspace.dir().to_owned()),
    ];

    for (what, path) in cases {
        for (verb, read) in BOTH {
            let out = under_json(read(&path, &Trace::Off));

            assert_eq!(out.exit, 5, "{verb} on {what}");
            assert_eq!(out.stderr, "", "{verb} on {what}");
            let body = envelope(&out);
            assert_eq!(body["ok"], json!(false), "{verb} on {what}");
            assert_eq!(
                body["error"]["code"],
                json!("unreadable"),
                "{verb} on {what}"
            );
            assert!(
                !body["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .is_empty(),
                "{verb} on {what} must carry a message"
            );
        }
    }
}

#[test]
fn an_unreadable_package_reports_on_stderr_without_json() {
    let workspace = Workspace::new("unreadable-text");
    let path = workspace.file("notes.txt", b"this is not a package");

    let out = in_text(verb::sheets(&path, &Trace::Off));

    assert_eq!(out.exit, 5);
    assert_eq!(out.stdout, "", "a failure leaves the data channel empty");
    assert!(out.stderr.starts_with("error: "), "stderr: {}", out.stderr);
}

#[test]
fn both_verbs_need_a_package_to_read() {
    for (verb, _) in BOTH {
        assert_eq!(exit_code(&run(&[verb])), 2, "{verb}");
    }
}

/// Both verbs, end to end through the process: `--` demotes what follows it
/// to an operand, and what comes back on stdout is what the library answered.
#[test]
fn the_double_dash_still_hands_the_path_over_as_an_operand() {
    let package = feature_package("double-dash");
    let package = package.to_str().expect("a UTF-8 path");
    let expected = [
        (
            "sheets",
            "Inputs\tvisible\nNotes\thidden\nParameters\tveryHidden\n",
        ),
        (
            "names",
            "MergedInput\tworkbook\tInputs!$B$2:$C$3\tInputs!B2\t\n",
        ),
    ];

    for (verb, answered) in expected {
        let out = run(&[verb, "--", package]);

        assert_eq!(exit_code(&out), 0, "{verb}: {}", stderr(&out));
        assert!(
            stdout(&out).starts_with(answered),
            "{verb}: {}",
            stdout(&out)
        );
    }
}

#[test]
fn quiet_and_verbose_move_stderr_only() {
    let package = feature_package("streams");
    let package = package.to_str().expect("a UTF-8 path");

    let plain = run(&["sheets", package]);
    let loud = run(&["sheets", package, "--verbose"]);
    let hushed = run(&["sheets", package, "--quiet"]);

    assert_eq!(stdout(&loud), stdout(&plain));
    assert_eq!(stdout(&hushed), stdout(&plain));
    assert!(!stderr(&loud).is_empty(), "--verbose must trace on stderr");
    assert_eq!(stderr(&hushed), "");
}
