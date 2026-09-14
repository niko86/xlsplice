//! Contract tests for `get`.
//!
//! Most of these call the library in process, through the verbs in `support`,
//! and read what `render` put on the two streams: one envelope, or the
//! tab-separated form a pipe gets.
//!
//! Three still spawn, because the process is what they are about: what clap
//! does with a target missing, what `--` does with the operands after it, and
//! what `--quiet` and `--verbose` do to stderr. The second of those takes
//! `get` end to end, argv to stdout, so its dispatch arm does not go
//! unexercised.

mod support;

use std::path::{Path, PathBuf};

use serde_json::json;
use support::{
    Copied, Workspace, built, envelope, exit_code, in_text, run, stderr, targets, under_json,
    workbook_xml,
};
use xlsplice::render::Rendered;
use xlsplice::verb::{self, Trace};

/// The feature package, and the workspace holding it alive for as long as the
/// test needs it.
fn feature(label: &str) -> Copied {
    built(label, |w| w.feature_package("feature.xlsx"))
}

/// The message of a failed envelope.
fn error_message(out: &Rendered) -> String {
    envelope(out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned()
}

/// The one cell `target` names, out of the envelope.
fn cell(package: &Path, target: &str) -> serde_json::Value {
    let out = under_json(verb::get(package, &targets(&[target]), &Trace::Off));
    assert_eq!(out.exit, 0, "get {target}: {}", out.stderr);
    envelope(&out)["cells"][0].clone()
}

#[test]
fn every_stored_type_comes_back_with_its_type_its_value_and_its_raw_text() {
    let package = feature("types");

    let seen: Vec<_> = [
        "Inputs!A1",
        "Inputs!A2",
        "Inputs!B1",
        "Inputs!C1",
        "Inputs!D1",
        "Inputs!E1",
        "Inputs!F1",
        "Inputs!G1",
        "Inputs!I1",
        "Inputs!A6",
    ]
    .into_iter()
    .map(|target| {
        let cell = cell(&package, target);
        (
            cell["type"].clone(),
            cell["value"].clone(),
            cell["raw"].clone(),
        )
    })
    .collect();

    assert_eq!(
        seen,
        [
            (json!("n"), json!(1), json!("1")),
            (json!("n"), json!(2.5), json!("2.5")),
            (json!("s"), json!("hello"), json!("0")),
            (
                json!("d"),
                json!("2026-09-11T00:00:00"),
                json!("2026-09-11T00:00:00")
            ),
            (json!("b"), json!(true), json!("1")),
            (json!("e"), json!("#DIV/0!"), json!("#DIV/0!")),
            (json!("str"), json!("ab"), json!("ab")),
            (json!("inlineStr"), json!("inline "), json!("inline ")),
            (json!("s"), json!("rich text"), json!("1")),
            (json!("empty"), json!(null), json!(null)),
        ]
    );
}

#[test]
fn a_boolean_cell_reports_a_boolean_either_way_round() {
    let package = feature("booleans");

    assert_eq!(cell(&package, "Inputs!D1")["value"], json!(true));
    let no = cell(&package, "Inputs!K1");
    assert_eq!(no["type"], json!("b"));
    assert_eq!(no["value"], json!(false));
    assert_eq!(no["raw"], json!("0"));
}

#[test]
fn a_shared_string_cell_reports_the_text_and_keeps_the_index_as_its_raw() {
    let package = feature("shared");

    let hello = cell(&package, "Inputs!B1");

    assert_eq!(hello["type"], json!("s"));
    assert_eq!(hello["value"], json!("hello"));
    assert_eq!(
        hello["raw"],
        json!("0"),
        "the raw text of a shared-string cell is its index"
    );
}

#[test]
fn rich_text_runs_are_concatenated_and_phonetic_text_is_left_out() {
    let package = feature("runs");

    assert_eq!(cell(&package, "Inputs!I1")["value"], json!("rich text"));
    assert_eq!(
        cell(&package, "Inputs!H1")["value"],
        json!("inline"),
        "an inline string's runs are concatenated too"
    );
    assert_eq!(
        cell(&package, "Inputs!J1")["value"],
        json!("東京"),
        "phonetic text is a reading guide, not part of the string"
    );
}

/// A shared-string cell keeps its index as its raw text, because the index is
/// what the cell stores. An inline string stores the text itself, so its raw
/// text is that text: there is no separate stored form to report, and the two
/// fields agreeing is the honest answer rather than an oversight.
#[test]
fn an_inline_strings_raw_text_is_the_text_the_cell_itself_holds() {
    let package = feature("inline-raw");

    let runs = cell(&package, "Inputs!H1");

    assert_eq!(runs["value"], json!("inline"));
    assert_eq!(runs["raw"], json!("inline"));
}

#[test]
fn an_absent_cell_is_empty_and_carries_neither_value_nor_style() {
    let package = feature("absent");

    let absent = cell(&package, "Inputs!Z1");

    assert_eq!(absent["type"], json!("empty"));
    assert_eq!(absent["value"], json!(null));
    assert_eq!(absent["raw"], json!(null));
    assert_eq!(absent["style"], json!(null));
    assert_eq!(absent["formula"], json!(null));
    assert_eq!(absent["address"], json!("Inputs!Z1"));
}

/// The sheet data of the feature package holds rows 1, 2, 3 and 6, so row 4
/// is one the sheet does not hold at all. A cell there is as absent as one in
/// a row that is there, and reads the same: `SKILL.md` promises that a cell
/// the sheet does not hold reads as `empty` rather than as a failure, and
/// which of the two reasons it is not there is no business of the caller's.
#[test]
fn a_cell_in_a_row_the_sheet_does_not_hold_is_empty_too() {
    let package = feature("no-row");

    let absent = cell(&package, "Inputs!C4");

    assert_eq!(absent["type"], json!("empty"));
    assert_eq!(absent["value"], json!(null));
    assert_eq!(absent["raw"], json!(null));
    assert_eq!(absent["style"], json!(null));
    assert_eq!(absent["address"], json!("Inputs!C4"));
}

/// And a sheet whose part holds no sheet data element holds no cells at all,
/// so every cell of it is absent. A write there is still refused — there is
/// nowhere to put a row — but that is a question about writing.
#[test]
fn a_cell_on_a_sheet_holding_no_sheet_data_at_all_is_empty() {
    let workspace = Workspace::new("dataless");
    let package = dataless(&workspace);

    let absent = cell(&package, "Inputs!A1");

    assert_eq!(absent["type"], json!("empty"));
    assert_eq!(absent["value"], json!(null));
    assert_eq!(absent["style"], json!(null));
}

/// A package of one sheet whose worksheet part has no sheet data element,
/// which no fixture has because Excel writes one whether or not the sheet
/// holds anything.
fn dataless(workspace: &Workspace) -> PathBuf {
    let bare = workspace.sheet_package("bare.xlsx", "", r#"<c r="A1"><v>1</v></c>"#);
    workspace.zip(
        "dataless.xlsx",
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (
                support::WORKBOOK_PART,
                &support::part_text(&bare, support::WORKBOOK_PART),
            ),
            (support::WORKBOOK_RELS_PART, support::WORKBOOK_RELS),
            (
                support::SHEET1_PART,
                &format!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{}"><dimension ref="A1"/></worksheet>"#,
                    "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                ),
            ),
        ],
    )
}

#[test]
fn a_cell_holding_a_style_and_no_value_is_empty_and_still_reports_the_style() {
    let package = feature("styled-empty");

    let styled = cell(&package, "Inputs!A6");

    assert_eq!(styled["type"], json!("empty"));
    assert_eq!(styled["value"], json!(null));
    assert_eq!(styled["style"], json!(5));
}

#[test]
fn every_cell_reports_the_style_index_it_carries() {
    let package = feature("styles");

    assert_eq!(cell(&package, "Inputs!B1")["style"], json!(1));
    assert_eq!(cell(&package, "Inputs!C1")["style"], json!(2));
    assert_eq!(
        cell(&package, "Inputs!A1")["style"],
        json!(0),
        "a cell with no style attribute carries the default format"
    );
}

#[test]
fn a_plain_formula_reports_its_text_and_nothing_else() {
    let package = feature("formula-plain");

    let formula = cell(&package, "Inputs!D2")["formula"].clone();

    assert_eq!(
        formula,
        json!({"text": "SUM(A1:A5)", "role": "plain", "range": null, "group": null})
    );
}

#[test]
fn a_shared_master_reports_its_range_and_a_child_reports_its_group() {
    let package = feature("formula-shared");

    assert_eq!(
        cell(&package, "Inputs!E2")["formula"],
        json!({"text": "A2*2", "role": "shared_master", "range": "E2:E3", "group": 0})
    );
    assert_eq!(
        cell(&package, "Inputs!E3")["formula"],
        json!({"text": "", "role": "shared_child", "range": null, "group": 0})
    );
}

#[test]
fn a_formula_cell_still_reports_its_cached_value() {
    let package = feature("cached");

    let cached = cell(&package, "Inputs!D2");

    assert_eq!(cached["type"], json!("n"));
    assert_eq!(cached["value"], json!(15));
    assert_eq!(cached["raw"], json!("15"));
}

#[test]
fn reading_through_the_merged_ranges_name_gives_the_anchors_content() {
    let package = feature("merged");

    let anchor = cell(&package, "MergedInput");

    assert_eq!(anchor["target"], json!("MergedInput"));
    assert_eq!(anchor["name"], json!("MergedInput"));
    assert_eq!(anchor["sheet"], json!("Inputs"));
    assert_eq!(anchor["cell"], json!("B2"));
    assert_eq!(anchor["address"], json!("Inputs!B2"));
    assert_eq!(anchor["value"], json!("merged"));
    assert_eq!(anchor["style"], json!(4));
}

#[test]
fn an_address_goes_through_no_name_at_all() {
    let package = feature("no-name");

    assert_eq!(cell(&package, "Inputs!B2")["name"], json!(null));
}

#[test]
fn a_sheet_scoped_name_is_read_through_the_sheet_it_is_scoped_to() {
    let package = feature("sheet-scoped");

    let note = cell(&package, "Notes!LocalNote");

    assert_eq!(note["name"], json!("LocalNote"));
    assert_eq!(note["address"], json!("Notes!A5"));
    assert_eq!(note["value"], json!("note"));
}

#[test]
fn a_target_is_matched_without_regard_to_case_and_answers_in_the_packages_spelling() {
    let package = feature("case");

    for target in ["INPUTS!b1", "inputs!B1"] {
        assert_eq!(
            cell(&package, target)["address"],
            json!("Inputs!B1"),
            "{target}"
        );
    }
    let loud = cell(&package, "loudcase");
    assert_eq!(loud["name"], json!("LoudCase"));
    assert_eq!(loud["address"], json!("Inputs!E1"));
    assert_eq!(
        cell(&package, "notes!localnote")["name"],
        json!("LocalNote")
    );
}

#[test]
fn dollars_in_an_address_are_no_more_significant_than_in_a_reference() {
    let package = feature("dollars");

    assert_eq!(cell(&package, "Inputs!$B$1")["address"], json!("Inputs!B1"));
}

#[test]
fn several_targets_come_back_one_per_target_in_the_order_they_were_given() {
    let package = feature("several");

    let out = under_json(verb::get(
        &package,
        &targets(&["Inputs!A3", "MergedInput", "Notes!LocalNote", "Inputs!A1"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0);
    let cells = envelope(&out)["cells"].clone();
    let targets: Vec<_> = cells
        .as_array()
        .expect("cells is a list")
        .iter()
        .map(|cell| cell["target"].clone())
        .collect();

    assert_eq!(
        targets,
        [
            json!("Inputs!A3"),
            json!("MergedInput"),
            json!("Notes!LocalNote"),
            json!("Inputs!A1"),
        ]
    );
    assert_eq!(cells[0]["value"], json!(-3));
    assert_eq!(cells[3]["value"], json!(1));
}

#[test]
fn one_target_is_one_tab_separated_line_with_every_field_in_its_place() {
    let package = feature("rows");

    let out = in_text(verb::get(
        &package,
        &targets(&["MergedInput", "Inputs!E2", "Inputs!Z1"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0);
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout,
        concat!(
            "MergedInput\tMergedInput\tInputs!B2\ts\tmerged\t2\t4\t\t\t\t\n",
            "Inputs!E2\t\tInputs!E2\tn\t5\t5\t0\tA2*2\tshared_master\tE2:E3\t0\n",
            "Inputs!Z1\t\tInputs!Z1\tempty\t\t\t\t\t\t\t\n",
        )
    );
}

#[test]
fn the_envelope_leads_with_ok_and_the_schema_version() {
    let package = feature("envelope");

    let body = envelope(&under_json(verb::get(
        &package,
        &targets(&["Inputs!A1"]),
        &Trace::Off,
    )));

    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["schema_version"], json!(1));
    assert_eq!(body.get("error"), None);
}

#[test]
fn an_unknown_sheet_is_not_found_and_the_message_lists_the_sheets() {
    let package = feature("no-sheet");

    let out = under_json(verb::get(&package, &targets(&["Missing!A1"]), &Trace::Off));

    assert_eq!(out.exit, 3);
    let message = error_message(&out);
    assert_eq!(envelope(&out)["error"]["code"], json!("not_found"));
    assert!(message.contains("Missing"), "{message}");
    for sheet in ["Inputs", "Notes", "Parameters"] {
        assert!(message.contains(sheet), "{message} must name {sheet}");
    }
}

#[test]
fn an_unknown_name_is_not_found_and_the_message_lists_the_names_of_its_scope() {
    let package = feature("no-name-found");

    let out = under_json(verb::get(&package, &targets(&["Absent"]), &Trace::Off));

    assert_eq!(out.exit, 3);
    let message = error_message(&out);
    assert_eq!(envelope(&out)["error"]["code"], json!("not_found"));
    assert!(message.contains("Absent"), "{message}");
    assert!(message.contains("MergedInput"), "{message}");
    assert!(
        !message.contains("LocalNote"),
        "a sheet-scoped name is in another scope: {message}"
    );
}

#[test]
fn an_unknown_sheet_scoped_name_names_the_sheet_it_was_looked_for_on() {
    let package = feature("no-local-name");

    let out = under_json(verb::get(
        &package,
        &targets(&["Notes!Absent"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 3);
    let message = error_message(&out);
    assert!(message.contains("Notes"), "{message}");
    assert!(message.contains("LocalNote"), "{message}");
}

#[test]
fn a_name_that_is_not_a_reference_is_refused_with_what_it_refers_to() {
    let package = feature("refused");
    let cases = [
        ("Rate", "0.175"),
        ("Total", "SUM(Inputs!$D$1:$D$9)"),
        ("Gone", "#REF!"),
    ];

    for (target, refers_to) in cases {
        let out = under_json(verb::get(&package, &targets(&[target]), &Trace::Off));

        assert_eq!(out.exit, 4, "{target}");
        let body = envelope(&out);
        assert_eq!(body["ok"], json!(false), "{target}");
        assert_eq!(body["error"]["code"], json!("refused"), "{target}");
        let message = error_message(&out);
        assert!(
            message.contains(refers_to),
            "{target}: {message} must carry its refersTo"
        );
    }
}

/// A cell outside every row the sheet holds, and a cell on a sheet holding no
/// rows at all, are both absent rather than missing. `not_found` is for what
/// is genuinely not there to be found — a sheet the package does not have, a
/// name it does not declare — and a cell a template has no element for is the
/// ordinary case rather than one of those: a template carries an element only
/// for the cells something is already in, which is why a write to such a cell
/// puts one there.
#[test]
fn a_cell_outside_every_row_the_sheet_holds_is_empty_rather_than_missing() {
    let package = feature("no-row");

    for target in ["Inputs!A99", "Parameters!A1"] {
        let out = under_json(verb::get(&package, &targets(&[target]), &Trace::Off));

        assert_eq!(out.exit, 0, "{target}: {}", out.stderr);
        let cell = &envelope(&out)["cells"][0];
        assert_eq!(cell["type"], json!("empty"), "{target}");
        assert_eq!(cell["value"], json!(null), "{target}");
        assert_eq!(cell["style"], json!(null), "{target}");
        assert_eq!(cell["address"], json!(target), "{target}");
    }
}

#[test]
fn a_name_whose_sheet_is_not_in_the_package_is_not_found() {
    let workspace = Workspace::new("name-elsewhere");
    let package = workspace.package(
        "elsewhere.xlsx",
        &workbook_xml(
            r#"<sheets><sheet name="Only" sheetId="1" r:id="rId1"/></sheets>
               <definedNames><definedName name="Away">Gone!$A$1</definedName></definedNames>"#,
        ),
    );

    let out = under_json(verb::get(&package, &targets(&["Away"]), &Trace::Off));

    assert_eq!(out.exit, 3);
    let message = error_message(&out);
    assert!(message.contains("Gone!$A$1"), "{message}");
}

#[test]
fn one_failing_target_fails_the_whole_read_and_leaves_stdout_empty() {
    let package = feature("all-or-nothing");

    let out = in_text(verb::get(
        &package,
        &targets(&["Inputs!A1", "Missing!A1"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 3);
    assert_eq!(
        out.stdout, "",
        "a failure leaves the data channel empty, part-read or not"
    );
    assert!(out.stderr.starts_with("error: "), "{}", out.stderr);
}

#[test]
fn a_sheet_whose_name_needs_quoting_is_addressed_and_reported_quoted() {
    let workspace = Workspace::new("quoted-sheet");
    let package = workspace.zip(
        "quoted.xlsx",
        &[
            (support::CONTENT_TYPES_PART, support::CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (
                support::WORKBOOK_PART,
                &workbook_xml(
                    r#"<sheets><sheet name="My Sheet" sheetId="1" r:id="rId1"/></sheets>"#,
                ),
            ),
            (support::WORKBOOK_RELS_PART, support::WORKBOOK_RELS),
            (support::SHEET1_PART, support::NOTES_SHEET),
        ],
    );

    let out = under_json(verb::get(
        &package,
        &targets(&["'My Sheet'!A5"]),
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stderr);
    let cell = envelope(&out)["cells"][0].clone();
    assert_eq!(cell["sheet"], json!("My Sheet"));
    assert_eq!(cell["address"], json!("'My Sheet'!A5"));
    assert_eq!(cell["value"], json!("note"));
}

#[test]
fn get_needs_a_package_and_at_least_one_target() {
    let package = feature("usage");

    assert_eq!(exit_code(&run(&["get"])), 2);
    assert_eq!(
        exit_code(&run(&["get", package.to_str().expect("a UTF-8 path")])),
        2
    );
}

/// `get` end to end through the process: `--` demotes what follows it to an
/// operand, and what comes back on stdout is what the library answered.
#[test]
fn the_double_dash_still_hands_the_operands_over() {
    let package = feature("double-dash");

    let out = run(&[
        "get",
        "--json",
        "--",
        package.to_str().expect("a UTF-8 path"),
        "Inputs!A1",
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(support::json(&out)["cells"][0]["value"], json!(1));
}

#[test]
fn a_path_that_is_not_a_package_is_unreadable() {
    let workspace = Workspace::new("get-unreadable");
    let path = workspace.file("notes.txt", b"this is not a package");

    let out = under_json(verb::get(&path, &targets(&["Inputs!A1"]), &Trace::Off));

    assert_eq!(out.exit, 5);
    assert_eq!(envelope(&out)["error"]["code"], json!("unreadable"));
}

#[test]
fn quiet_and_verbose_move_stderr_only() {
    let package = feature("get-streams");
    let package = package.to_str().expect("a UTF-8 path");

    let plain = run(&["get", package, "Inputs!A1"]);
    let loud = run(&["get", package, "Inputs!A1", "--verbose"]);
    let hushed = run(&["get", package, "Inputs!A1", "--quiet"]);

    assert_eq!(support::stdout(&loud), support::stdout(&plain));
    assert_eq!(support::stdout(&hushed), support::stdout(&plain));
    assert!(!stderr(&loud).is_empty(), "--verbose must trace on stderr");
    assert_eq!(stderr(&hushed), "");
}

/// Excel names a worksheet part for the order its sheet was created in, not
/// for where its tab sits, so the two disagree the moment a sheet is moved.
/// This package is built so that they do: the first tab's cells are in
/// `sheet2.xml`. With the relationship gone, anything that guessed
/// `sheet1.xml` for the first tab would hand back the other sheet's cells,
/// so the only safe answer is no answer.
#[test]
fn a_sheet_whose_relationship_is_gone_is_not_found_rather_than_guessed_at() {
    let workspace = Workspace::new("reordered");
    let workbook = workbook_xml(
        r#"<sheets>
             <sheet name="Second" sheetId="2" r:id="rId2"/>
             <sheet name="First" sheetId="1" r:id="rId1"/>
           </sheets>"#,
    );
    let package = workspace.zip(
        "reordered.xlsx",
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (support::WORKBOOK_PART, &workbook),
            // The relationships that say which part is which sheet are gone.
            (support::SHEET1_PART, support::INPUTS_SHEET),
            (support::SHEET2_PART, support::NOTES_SHEET),
        ],
    );

    let out = under_json(verb::get(&package, &targets(&["Second!A1"]), &Trace::Off));

    assert_eq!(
        out.exit, 3,
        "a wrong answer is worse than no answer: {}",
        out.stdout
    );
    assert_eq!(envelope(&out)["error"]["code"], json!("not_found"));
    assert!(
        error_message(&out).contains("Second"),
        "{}",
        error_message(&out)
    );
}

/// A package holds at most one shared string table, so where a relationship
/// does not name one, the conventional path cannot be confused with anything
/// and is worth trying. This is the opposite call from a worksheet part, and
/// for the opposite reason.
#[test]
fn a_shared_string_table_the_relationships_do_not_name_is_still_found() {
    let workspace = Workspace::new("unnamed-table");
    let workbook = support::feature_workbook();
    let sheets_only = support::WORKBOOK_RELS
        .lines()
        .filter(|line| !line.contains("sharedStrings"))
        .collect::<Vec<_>>()
        .join("\n");
    let package = workspace.zip(
        "unnamed.xlsx",
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (support::WORKBOOK_PART, &workbook),
            (support::WORKBOOK_RELS_PART, &sheets_only),
            (support::SHEET1_PART, support::INPUTS_SHEET),
            (support::SHARED_STRINGS_PART, support::SHARED_STRINGS),
        ],
    );

    let out = under_json(verb::get(&package, &targets(&["Inputs!B1"]), &Trace::Off));

    assert_eq!(out.exit, 0, "{}", out.stderr);
    assert_eq!(envelope(&out)["cells"][0]["value"], json!("hello"));
}

#[test]
fn a_sheet_the_package_holds_no_worksheet_part_for_is_not_found() {
    let workspace = Workspace::new("no-sheet-part");
    let package = workspace.package(
        "partless.xlsx",
        &workbook_xml(r#"<sheets><sheet name="Only" sheetId="1" r:id="rId1"/></sheets>"#),
    );

    let out = under_json(verb::get(&package, &targets(&["Only!A1"]), &Trace::Off));

    assert_eq!(out.exit, 3);
    let message = error_message(&out);
    assert_eq!(envelope(&out)["error"]["code"], json!("not_found"));
    assert!(message.contains("Only"), "{message}");
}
