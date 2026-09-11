//! Contract tests for `sheets` and `names`: everything here observes the
//! built binary from outside, through its argv, its two streams and its exit
//! code. stdout is a pipe in every one of them, so the text these assert is
//! the tab-separated form; the aligned form is held by the binary's own unit
//! tests over `render_rows` and `TextStyle::for_terminal`, because a spawned
//! process has no terminal to give it.

mod support;

use support::{
    CONTENT_TYPES, CONTENT_TYPES_PART, ROOT_RELS, ROOT_RELS_PART, WORKBOOK_PART, Workspace,
    exit_code, feature_workbook, json, run, stderr, stdout, workbook_xml,
};

/// The feature workbook, written to a package, and the workspace holding it
/// alive for as long as the test needs it.
fn feature_package(label: &str) -> (Workspace, String) {
    let workspace = Workspace::new(label);
    let path = workspace.package("feature.xlsx", &feature_workbook());
    let path = path
        .to_str()
        .expect("a temporary path must be UTF-8")
        .to_owned();
    (workspace, path)
}

#[test]
fn sheets_reports_every_sheet_with_its_state_in_workbook_order() {
    let (_workspace, package) = feature_package("sheets-order");

    let out = run(&["sheets", &package]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(stderr(&out), "");
    assert_eq!(
        stdout(&out),
        "Inputs\tvisible\nNotes\thidden\nParameters\tveryHidden\n"
    );
}

#[test]
fn sheets_json_carries_the_same_sheets_in_the_envelope() {
    let (_workspace, package) = feature_package("sheets-json");

    let out = run(&["sheets", &package, "--json"]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(stderr(&out), "");
    let body = json(&out);
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["schema_version"], serde_json::json!(1));
    assert_eq!(
        body["sheets"],
        serde_json::json!([
            {"name": "Inputs", "state": "visible"},
            {"name": "Notes", "state": "hidden"},
            {"name": "Parameters", "state": "veryHidden"},
        ])
    );
}

#[test]
fn names_reports_the_scope_the_reference_the_anchor_and_the_reason() {
    let (_workspace, package) = feature_package("names-rows");

    let out = run(&["names", &package]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(stderr(&out), "");
    assert_eq!(
        stdout(&out),
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
    let (_workspace, package) = feature_package("names-merged");

    let body = json(&run(&["names", &package, "--json"]));
    let merged = &body["names"][0];

    assert_eq!(merged["name"], serde_json::json!("MergedInput"));
    assert_eq!(merged["scope"], serde_json::json!("workbook"));
    assert_eq!(merged["scope_sheet"], serde_json::json!(null));
    assert_eq!(merged["refers_to"], serde_json::json!("Inputs!$B$2:$C$3"));
    assert_eq!(
        merged["anchor"],
        serde_json::json!({"sheet": "Inputs", "cell": "B2", "address": "Inputs!B2"})
    );
    assert_eq!(merged["reason"], serde_json::json!(null));
}

#[test]
fn a_sheet_scoped_name_reports_the_sheet_it_is_scoped_to() {
    let (_workspace, package) = feature_package("names-local");

    let local = json(&run(&["names", &package, "--json"]))["names"][1].clone();

    assert_eq!(local["name"], serde_json::json!("LocalNote"));
    assert_eq!(local["scope"], serde_json::json!("sheet"));
    assert_eq!(local["scope_sheet"], serde_json::json!("Notes"));
    assert_eq!(local["anchor"]["address"], serde_json::json!("Notes!A5"));
}

#[test]
fn a_name_that_resolves_to_no_cell_carries_the_reason_and_no_anchor() {
    let (_workspace, package) = feature_package("names-reasons");

    let body = json(&run(&["names", &package, "--json"]));
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
            (serde_json::json!("Gone"), serde_json::json!("ref_error")),
            (serde_json::json!("Rate"), serde_json::json!("constant")),
            (serde_json::json!("Total"), serde_json::json!("formula")),
        ]
    );
}

#[test]
fn a_reference_spelled_in_another_case_resolves_to_the_packages_spelling() {
    let (_workspace, package) = feature_package("names-case");

    let loud = json(&run(&["names", &package, "--json"]))["names"][2].clone();

    assert_eq!(
        loud["refers_to"],
        serde_json::json!("INPUTS!$E$1"),
        "the reference is reported as the package holds it"
    );
    assert_eq!(loud["anchor"]["sheet"], serde_json::json!("Inputs"));
    assert_eq!(loud["anchor"]["address"], serde_json::json!("Inputs!E1"));
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

    let out = run(&["names", package.to_str().expect("a UTF-8 path")]);

    assert_eq!(exit_code(&out), 0);
    assert!(
        stdout(&out).contains("'My Sheet'!A1"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn a_package_with_no_defined_names_says_nothing_down_the_pipe() {
    let workspace = Workspace::new("names-empty");
    let package = workspace.package(
        "bare.xlsx",
        &workbook_xml(r#"<sheets><sheet name="Sheet1" sheetId="1"/></sheets>"#),
    );

    let out = run(&["names", package.to_str().expect("a UTF-8 path")]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(
        stdout(&out),
        "",
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

    let out = run(&["sheets", package.to_str().expect("a UTF-8 path")]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(stdout(&out), "Only\tvisible\n");
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
        let path = path.to_str().expect("a UTF-8 path");
        for verb in ["sheets", "names"] {
            let out = run(&[verb, path, "--json"]);

            assert_eq!(exit_code(&out), 5, "{verb} on {what}");
            assert_eq!(stderr(&out), "", "{verb} on {what}");
            let body = json(&out);
            assert_eq!(body["ok"], serde_json::json!(false), "{verb} on {what}");
            assert_eq!(
                body["error"]["code"],
                serde_json::json!("unreadable"),
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

    let out = run(&["sheets", path.to_str().expect("a UTF-8 path")]);

    assert_eq!(exit_code(&out), 5);
    assert_eq!(stdout(&out), "", "a failure leaves the data channel empty");
    assert!(
        stderr(&out).starts_with("error: "),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn both_verbs_need_a_package_to_read() {
    for verb in ["sheets", "names"] {
        assert_eq!(exit_code(&run(&[verb])), 2, "{verb}");
    }
}

#[test]
fn the_double_dash_still_hands_the_path_over_as_an_operand() {
    let (_workspace, package) = feature_package("double-dash");

    let out = run(&["sheets", "--", &package]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(
        stdout(&out),
        "Inputs\tvisible\nNotes\thidden\nParameters\tveryHidden\n"
    );
}

#[test]
fn quiet_and_verbose_move_stderr_only() {
    let (_workspace, package) = feature_package("streams");

    let plain = run(&["sheets", &package]);
    let loud = run(&["sheets", &package, "--verbose"]);
    let hushed = run(&["sheets", &package, "--quiet"]);

    assert_eq!(stdout(&loud), stdout(&plain));
    assert_eq!(stdout(&hushed), stdout(&plain));
    assert!(!stderr(&loud).is_empty(), "--verbose must trace on stderr");
    assert_eq!(stderr(&hushed), "");
}
