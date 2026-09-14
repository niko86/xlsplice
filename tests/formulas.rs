//! The formula guard, and what replacing a formula does to the calc chain.
//!
//! A mis-addressed write must not silently destroy a template's formula, so a
//! cell holding one is refused unless the operation says it meant it. When one
//! is replaced, its entry in the calc chain goes with it, because an entry
//! naming a cell that no longer holds a formula is the sort of inconsistency
//! Excel offers to repair.
//!
//! The feature fixture is the one Excel saved with formulas and a calc chain:
//! `Inputs!D1` and `Inputs!E1` are plain, `Inputs!E2` is a shared master over
//! `E2:E5`, and `E3`, `E4` and `E5` are its children. Its chain holds all six.

mod support;

use std::path::{Path, PathBuf};

use support::binary::{exit_code, run, stderr};
use support::container::{
    CALC_CHAIN, CONTENT_TYPES, SHEET1, WORKBOOK_RELS, assert_only_these_differ, assert_same_bytes,
    assert_spliced, part_text,
};
use support::library::{envelope, op, targets, under_json};
use support::workspace::{Workspace, copy_of, fixture};
use xlsplice::batch::Batch;
use xlsplice::batch::Destination;
use xlsplice::batch::WriteType;
use xlsplice::verb::{self, Trace};

/// A package holding one formula, with a calc chain of one entry, the
/// relationship that reaches it and its content-type override. Built in
/// `support`, because the oracle suite puts one in front of Excel as well.
fn one_chained_formula(workspace: &Workspace) -> PathBuf {
    workspace.chained_package(
        "chained.xlsx",
        r#"<c r="A1"><f>1+1</f><v>2</v></c>"#,
        r#"<c r="A1" i="1"/>"#,
    )
}

/// The same, with two formulas and two entries, so that emptying the chain
/// takes two operations.
fn two_chained_formulas(workspace: &Workspace) -> PathBuf {
    workspace.chained_package(
        "chained.xlsx",
        r#"<c r="A1"><f>1+1</f><v>2</v></c><c r="B1"><f>2+2</f><v>4</v></c>"#,
        r#"<c r="A1" i="1"/><c r="B1" i="1"/>"#,
    )
}

/// Write a number into `target`, licensed or not, and give back the envelope.
fn write(package: &Path, target: &str, licensed: bool) -> xlsplice::render::Rendered {
    let operation = op::writing(target, WriteType::Number, "99");
    let operation = match licensed {
        true => op::replacing(operation),
        false => operation,
    };
    under_json(verb::run(
        package,
        &Batch {
            operations: vec![operation],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ))
}

#[test]
fn a_plain_formula_cell_is_refused_without_the_flag_and_the_file_is_untouched() {
    let package = copy_of("plain-refused", "feature.xlsx");

    for target in ["Inputs!D1", "Inputs!E1"] {
        let out = write(&package, target, false);

        assert_eq!(out.exit, 4, "{target}");
        let body = envelope(&out);
        assert_eq!(
            body["error"]["code"],
            serde_json::json!("refused"),
            "{target}"
        );
        let message = body["error"]["message"]
            .as_str()
            .expect("a failed envelope carries a message");
        assert!(
            message.contains("--replace-formula") && message.contains("replace_formula: true"),
            "{target}: the message must name the flag both ways: {message}"
        );
    }
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn clearing_a_plain_formula_cell_is_refused_without_the_flag_too() {
    let package = copy_of("clear-refused", "feature.xlsx");

    let out = under_json(verb::clear(
        &package,
        "Inputs!D1",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 4);
    assert!(
        envelope(&out)["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("--replace-formula")
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// With the flag the formula goes, the value lands, and `get` shows no
/// formula at all where one was.
#[test]
fn with_the_flag_the_formula_goes_and_the_value_lands() {
    let package = copy_of("plain-replaced", "feature.xlsx");

    let out = write(&package, "Inputs!D1", true);

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="D1"><f>SUM(A1:A5)</f><v>15</v></c>"#,
        r#"<c r="D1"><v>99</v></c>"#,
    );
    let cell = envelope(&under_json(verb::get(
        &package,
        &targets(&["Inputs!D1"]),
        &Trace::Off,
    )));
    assert_eq!(cell["cells"][0]["formula"], serde_json::json!(null));
    assert_eq!(cell["cells"][0]["value"], serde_json::json!(99));
}

#[test]
fn clearing_a_formula_cell_with_the_flag_empties_it() {
    let package = copy_of("clear-replaced", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::replacing(op::clearing("Inputs!D1"))],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), SHEET1),
        &part_text(&package, SHEET1),
        r#"<c r="D1"><f>SUM(A1:A5)</f><v>15</v></c>"#,
        r#"<c r="D1"/>"#,
    );
}

/// A shared master carries the formula its children take theirs from, so
/// overwriting it orphans them. No flag licenses that, and the message names
/// the range so the caller can see what it would have taken with it.
#[test]
fn the_shared_master_is_refused_even_with_the_flag() {
    let package = copy_of("master", "feature.xlsx");

    for licensed in [false, true] {
        let out = write(&package, "Inputs!E2", licensed);

        assert_eq!(out.exit, 4, "licensed: {licensed}");
        let message = envelope(&out)["error"]["message"]
            .as_str()
            .expect("a message")
            .to_owned();
        assert!(message.contains("E2:E5"), "{message}");
        assert!(message.contains("orphan"), "{message}");
    }
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn a_shared_child_is_replaced_with_the_flag() {
    let package = copy_of("child", "feature.xlsx");

    let out = write(&package, "Inputs!E3", true);

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert!(
        part_text(&package, SHEET1).contains(r#"<c r="E3"><v>99</v></c>"#),
        "{}",
        part_text(&package, SHEET1)
    );
    assert!(
        part_text(&package, SHEET1).contains(r#"<f t="shared" ref="E2:E5" si="0">A2*2</f>"#),
        "the master keeps its formula and its range"
    );
}

/// The chain comes out as the input minus exactly that cell's entry, and
/// every other part of the package is untouched.
#[test]
fn the_calc_chain_loses_exactly_that_cell_and_nothing_else_moves() {
    let package = copy_of("chain-entry", "feature.xlsx");

    let out = write(&package, "Inputs!D1", true);

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["parts"],
        serde_json::json!({"changed": [CALC_CHAIN, SHEET1], "added": [], "removed": []})
    );
    // The comparator lists parts in container order, where the worksheet comes
    // first; the report lists them in path order, where the chain does.
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[SHEET1, CALC_CHAIN]);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), CALC_CHAIN),
        &part_text(&package, CALC_CHAIN),
        r#"<c r="D1" i="1"/>"#,
        "",
    );
}

/// Replacing the last chained formula takes the part with it, and the
/// relationship that reaches it and its content type go too, so the package
/// never says it holds a part it does not.
///
/// No fixture has a chain that can be emptied: the feature package's holds the
/// shared master's entry, and a master is refused whatever else a batch does.
/// So the package is built here, one formula and one entry, and every part it
/// declares is written out so that what goes is visible in the test.
#[test]
fn emptying_the_chain_removes_the_part_its_relationship_and_its_override() {
    let workspace = Workspace::new("chain-emptied");
    let package = one_chained_formula(&workspace);
    let before = support::container::parts(&package).len();

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::replacing(op::writing(
                "Inputs!A1",
                WriteType::Number,
                "7",
            ))],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["parts"]["removed"],
        serde_json::json!([CALC_CHAIN]),
        "the report says the part went"
    );
    let now: Vec<String> = support::container::parts(&package)
        .into_iter()
        .map(|part| part.path)
        .collect();
    assert_eq!(now.len(), before - 1, "one part fewer: {now:?}");
    assert!(!now.contains(&CALC_CHAIN.to_owned()), "{now:?}");
    assert!(
        !part_text(&package, CONTENT_TYPES).contains("calcChain"),
        "the content type went with it: {}",
        part_text(&package, CONTENT_TYPES)
    );
    assert!(
        !part_text(&package, WORKBOOK_RELS).contains("calcChain"),
        "the relationship went with it: {}",
        part_text(&package, WORKBOOK_RELS)
    );
    assert!(part_text(&package, SHEET1).contains(r#"<c r="A1"><v>7</v></c>"#));
}

/// Two operations may each take out an entry, and neither can see the other's,
/// so whether the last one has gone is asked of the chain as the whole batch
/// leaves it.
#[test]
fn a_batch_that_empties_the_chain_between_its_operations_still_removes_it() {
    let workspace = Workspace::new("emptied-between");
    let package = two_chained_formulas(&workspace);

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::replacing(op::writing("Inputs!A1", WriteType::Number, "1")),
                op::replacing(op::writing("Inputs!B1", WriteType::Number, "2")),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["parts"]["removed"],
        serde_json::json!([CALC_CHAIN]),
        "neither operation emptied it alone, and together they did"
    );
    assert!(
        !support::container::parts(&package)
            .iter()
            .any(|part| part.path == CALC_CHAIN)
    );
}

/// A package with no calc chain has nothing to maintain, and replacing a
/// formula in it touches only the worksheet.
#[test]
fn a_package_with_no_calc_chain_needs_no_maintenance() {
    let workspace = Workspace::new("no-chain");
    let package =
        workspace.sheet_package("no-chain.xlsx", "", r#"<c r="A1"><f>1+1</f><v>2</v></c>"#);

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::replacing(op::writing(
                "Inputs!A1",
                WriteType::Number,
                "7",
            ))],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["parts"],
        serde_json::json!({"changed": [SHEET1], "added": [], "removed": []})
    );
    assert!(part_text(&package, SHEET1).contains(r#"<c r="A1"><v>7</v></c>"#));
}

/// The flag is one operation's, not the batch's.
#[test]
fn the_flag_on_one_operation_does_not_license_another() {
    let package = copy_of("one-licence", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::replacing(op::writing("Inputs!D1", WriteType::Number, "1")),
                op::writing("Inputs!E1", WriteType::Number, "2"),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 4);
    let message = envelope(&out)["error"]["message"]
        .as_str()
        .expect("a message")
        .to_owned();
    assert!(
        message.starts_with("operation at index 1 (set Inputs!E1): "),
        "{message}"
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// Two operations replacing two formulas take both entries out of one chain,
/// which is one part spliced once with both edits in it.
#[test]
fn two_replacements_take_two_entries_out_of_one_chain() {
    let package = copy_of("two-entries", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::replacing(op::writing("Inputs!D1", WriteType::Number, "1")),
                op::replacing(op::writing("Inputs!E1", WriteType::Number, "2")),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let chain = part_text(&package, CALC_CHAIN);
    assert!(!chain.contains(r#"r="D1""#), "{chain}");
    assert!(!chain.contains(r#"r="E1""#), "{chain}");
    assert_eq!(chain.matches("<c ").count(), 4, "{chain}");
}

/// The flag reaches the command line under the name the message gives.
#[test]
fn the_flag_works_from_the_command_line() {
    let package = copy_of("argv", "feature.xlsx");
    let file = package.display().to_string();

    let refused = run(&["set", &file, "Inputs!D1", "5", "--type", "number"]);
    assert_eq!(exit_code(&refused), 4, "{}", stderr(&refused));

    let out = run(&[
        "set",
        &file,
        "Inputs!D1",
        "5",
        "--type",
        "number",
        "--replace-formula",
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert!(part_text(&package, SHEET1).contains(r#"<c r="D1"><v>5</v></c>"#));
    assert!(!part_text(&package, CALC_CHAIN).contains(r#"r="D1""#));
}

#[test]
fn both_writing_verbs_list_the_flag_in_their_help() {
    for verb in ["set", "clear"] {
        let out = run(&[verb, "--help"]);

        assert_eq!(exit_code(&out), 0, "{verb}: {}", stderr(&out));
        assert!(
            support::binary::stdout(&out).contains("--replace-formula"),
            "{verb} --help must list the flag"
        );
    }
}

/// `apply` says it per operation, so it takes no flag of its own.
#[test]
fn apply_takes_no_flag_of_its_own() {
    assert!(
        !support::binary::stdout(&run(&["apply", "--help"])).contains("--replace-formula"),
        "a batch says it per operation"
    );
}

#[test]
fn the_fixture_is_the_baseline_and_stays_put() {
    assert!(Path::new(&fixture("feature.xlsx")).exists());
}

/// Neither declaration part is touched while the chain still holds entries.
#[test]
fn a_chain_that_survives_leaves_the_declarations_alone() {
    let package = copy_of("declarations-kept", "feature.xlsx");

    let out = write(&package, "Inputs!D1", true);

    assert_eq!(out.exit, 0, "{}", out.stdout);
    for part in [CONTENT_TYPES, WORKBOOK_RELS] {
        assert_eq!(
            part_text(&package, part),
            part_text(&fixture("feature.xlsx"), part),
            "{part} must not move while the chain is still there"
        );
    }
}
