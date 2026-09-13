//! Contract tests for `diff`: what two packages come to, part by part.
//!
//! The verb exists to answer one question about a hydration — which parts did
//! that write actually touch — so the tests are mostly about running a write
//! and asking. It is the one verb whose exit code says something when it
//! succeeds, so what `--exit-code` does, and what it does not do to the other
//! codes, is asserted here.

mod support;

use std::path::PathBuf;

use serde_json::json;
use support::{Workspace, exit_code, fixture, json, run, stderr, stdout, verb};
use xlsplice::batch::WriteType;

const SHEET1: &str = "xl/worksheets/sheet1.xml";
const CUSTOM: &str = "docProps/custom.xml";
const CONTENT_TYPES: &str = "[Content_Types].xml";
const ROOT_RELS: &str = "_rels/.rels";

/// A workspace holding a writable copy of `name` and the untouched fixture to
/// compare it against.
fn copies(label: &str, name: &str) -> (Workspace, PathBuf, PathBuf) {
    let workspace = Workspace::new(label);
    let before = workspace.copy_of(name);
    let after = workspace.dir().join(format!("after-{name}"));
    std::fs::copy(&before, &after).expect("a test must be able to copy its own package");
    (workspace, before, after)
}

/// What `diff A B --json` answers, having asserted it succeeded.
fn compared(a: &std::path::Path, b: &std::path::Path) -> serde_json::Value {
    let out = run(&[
        "diff",
        &a.display().to_string(),
        &b.display().to_string(),
        "--json",
    ]);
    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    json(&out)
}

/// The parts the comparison says are not identical, by path and status.
fn moved(body: &serde_json::Value) -> Vec<(String, String)> {
    body["parts"]
        .as_array()
        .expect("a list of parts")
        .iter()
        .filter(|part| part["status"] != json!("identical"))
        .map(|part| {
            (
                part["part"].as_str().expect("a path").to_owned(),
                part["status"].as_str().expect("a status").to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_package_against_itself_is_identical_part_for_part() {
    let (_workspace, before, after) = copies("same", "feature.xlsx");

    let body = compared(&before, &after);

    assert_eq!(body["identical"], json!(true));
    assert_eq!(moved(&body), Vec::new());
    assert_eq!(
        body["parts"].as_array().expect("a list of parts").len(),
        14,
        "every part the fixture holds is reported"
    );
}

/// A package compared with the one file it is: the same bytes on both sides.
#[test]
fn a_package_against_the_same_file_is_identical() {
    let package = fixture("feature.xlsx");

    let body = compared(&package, &package);

    assert_eq!(body["identical"], json!(true));
}

#[test]
fn a_write_shows_up_as_exactly_the_part_it_touched() {
    let (_workspace, before, after) = copies("written", "feature.xlsx");
    let out = support::under_json(verb::set(
        &after,
        "Inputs!A1",
        WriteType::Number,
        "99",
        None,
        false,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);

    let body = compared(&before, &after);

    assert_eq!(body["identical"], json!(false));
    assert_eq!(moved(&body), [(SHEET1.to_owned(), "differs".to_owned())]);
}

/// A part put into a package shows up as added, and the parts that had to
/// declare it as differing.
#[test]
fn a_part_a_write_added_is_reported_as_added() {
    let (_workspace, before, after) = copies("added", "plain.xlsx");
    let out = support::under_json(verb::batch(
        &after,
        vec![verb::stamping("Reference", WriteType::Text, "R-1")],
        None,
        false,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);

    let body = compared(&before, &after);

    assert_eq!(
        moved(&body),
        [
            (CONTENT_TYPES.to_owned(), "differs".to_owned()),
            (ROOT_RELS.to_owned(), "differs".to_owned()),
            (CUSTOM.to_owned(), "added".to_owned()),
        ],
        "the added part comes last, because the package it was added to had it last"
    );
}

/// Which package is which decides whether a part is added or removed, so the
/// same write compared the other way round is the same part gone.
#[test]
fn a_part_only_the_first_package_holds_is_reported_as_removed() {
    let (_workspace, before, after) = copies("removed", "plain.xlsx");
    let out = support::under_json(verb::batch(
        &after,
        vec![verb::stamping("Reference", WriteType::Text, "R-1")],
        None,
        false,
    ));
    assert_eq!(out.exit, 0, "{}", out.stdout);

    let body = compared(&after, &before);

    assert_eq!(
        moved(&body),
        [
            (CONTENT_TYPES.to_owned(), "differs".to_owned()),
            (ROOT_RELS.to_owned(), "differs".to_owned()),
            (CUSTOM.to_owned(), "removed".to_owned()),
        ]
    );
}

/// Without the flag the command exits 0 whether or not they differ, because
/// the answer is where the difference is read.
#[test]
fn the_exit_code_is_zero_either_way_without_the_flag() {
    let (_workspace, before, after) = copies("no-flag", "feature.xlsx");
    support::under_json(verb::set(
        &after,
        "Inputs!A1",
        WriteType::Number,
        "99",
        None,
        false,
    ));

    for (label, a, b) in [
        ("identical", &before, &before),
        ("different", &before, &after),
    ] {
        let out = run(&["diff", &a.display().to_string(), &b.display().to_string()]);

        assert_eq!(exit_code(&out), 0, "{label}: {}", stderr(&out));
    }
}

/// With the flag, a difference is 1 and no difference is 0, which is what
/// diff(1) does and what a caller reaching for the flag is reaching for.
#[test]
fn the_flag_exits_one_on_a_difference_and_zero_without_one() {
    let (_workspace, before, after) = copies("flag", "feature.xlsx");
    support::under_json(verb::set(
        &after,
        "Inputs!A1",
        WriteType::Number,
        "99",
        None,
        false,
    ));

    for (label, a, b, expected) in [
        ("identical", &before, &before, 0),
        ("different", &before, &after, 1),
    ] {
        let out = run(&[
            "diff",
            &a.display().to_string(),
            &b.display().to_string(),
            "--exit-code",
        ]);

        assert_eq!(exit_code(&out), expected, "{label}: {}", stderr(&out));
    }
}

/// An exit code that says the two differ is still an answer: the envelope
/// says the command succeeded, which is what tells it apart from the internal
/// failure that shares its number.
#[test]
fn a_difference_under_the_flag_is_still_a_successful_answer() {
    let (_workspace, before, after) = copies("flag-json", "feature.xlsx");
    support::under_json(verb::set(
        &after,
        "Inputs!A1",
        WriteType::Number,
        "99",
        None,
        false,
    ));

    let out = run(&[
        "diff",
        &before.display().to_string(),
        &after.display().to_string(),
        "--exit-code",
        "--json",
    ]);

    assert_eq!(exit_code(&out), 1);
    let body = json(&out);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["identical"], json!(false));
    assert_eq!(body.get("error"), None);
    assert_eq!(
        stderr(&out),
        "",
        "under --json the envelope is the whole of it"
    );
}

/// The flag says nothing about failures: a package that cannot be read is
/// unreadable, not a difference, whether the flag is there or not.
#[test]
fn a_package_that_is_not_one_is_unreadable_on_either_side() {
    let workspace = Workspace::new("unreadable");
    let package = workspace.copy_of("feature.xlsx");
    let not_a_package = workspace.file("notes.txt", b"this is not a package\n");
    let (real, fake) = (
        package.display().to_string(),
        not_a_package.display().to_string(),
    );

    for (label, args) in [
        ("second", vec!["diff", &real, &fake, "--json"]),
        ("first", vec!["diff", &fake, &real, "--json"]),
        (
            "with the flag",
            vec!["diff", &real, &fake, "--exit-code", "--json"],
        ),
    ] {
        let out = run(&args);

        assert_eq!(exit_code(&out), 5, "{label}");
        assert_eq!(json(&out)["error"]["code"], json!("unreadable"), "{label}");
        assert_eq!(json(&out)["ok"], json!(false), "{label}");
    }
}

#[test]
fn a_package_that_is_not_there_is_unreadable() {
    let workspace = Workspace::new("missing");
    let package = workspace.copy_of("feature.xlsx");
    let nowhere = workspace.dir().join("nowhere.xlsx");

    let out = run(&[
        "diff",
        &package.display().to_string(),
        nowhere.to_str().expect("a path"),
        "--json",
    ]);

    assert_eq!(exit_code(&out), 5);
    assert_eq!(json(&out)["error"]["code"], json!("unreadable"));
}

#[test]
fn the_rows_are_one_part_each_tab_separated_down_a_pipe() {
    let (_workspace, before, after) = copies("rows", "plain.xlsx");

    let out = run(&[
        "diff",
        &before.display().to_string(),
        &after.display().to_string(),
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[Content_Types].xml\tidentical\n\
         _rels/.rels\tidentical\n\
         xl/workbook.xml\tidentical\n\
         xl/_rels/workbook.xml.rels\tidentical\n\
         xl/worksheets/sheet1.xml\tidentical\n\
         xl/theme/theme1.xml\tidentical\n\
         xl/styles.xml\tidentical\n\
         xl/sharedStrings.xml\tidentical\n\
         docProps/core.xml\tidentical\n\
         docProps/app.xml\tidentical\n"
    );
}

/// Neither package is written to, whatever the comparison says.
#[test]
fn neither_package_is_touched() {
    let (_workspace, before, after) = copies("untouched", "feature.xlsx");
    support::under_json(verb::set(
        &after,
        "Inputs!A1",
        WriteType::Number,
        "99",
        None,
        false,
    ));
    let (was, then) = (
        std::fs::read(&before).expect("readable"),
        std::fs::read(&after).expect("readable"),
    );

    run(&[
        "diff",
        &before.display().to_string(),
        &after.display().to_string(),
        "--exit-code",
    ]);

    assert_eq!(std::fs::read(&before).expect("readable"), was);
    assert_eq!(std::fs::read(&after).expect("readable"), then);
    support::assert_same_bytes(&fixture("feature.xlsx"), &before);
}

#[test]
fn the_help_lists_both_operands_and_the_flag() {
    let out = run(&["diff", "--help"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    for wanted in ["A", "B", "--exit-code"] {
        assert!(
            stdout(&out).contains(wanted),
            "diff --help must list {wanted}: {}",
            stdout(&out)
        );
    }
}

#[test]
fn one_operand_is_a_usage_error() {
    let package = fixture("feature.xlsx").display().to_string();

    let out = run(&["diff", &package, "--json"]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(json(&out)["error"]["code"], json!("usage"));
}
