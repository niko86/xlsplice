//! What real Excel makes of what xlsplice wrote.
//!
//! Every other suite holds a written package against bytes the test spells
//! out, which says the splice landed where it was meant to and nothing else
//! moved. It cannot say whether Excel will open the result without complaint,
//! because the only thing that knows that is Excel. So these open the package
//! in Excel and ask.
//!
//! They are ignored, all of them. Driving a GUI application needs a
//! logged-in session, takes the screen from whoever is at the machine, and
//! takes the better part of a minute for a cold Excel. Nothing here runs
//! unless it is asked for:
//!
//! ```text
//! cargo test --test oracle -- --ignored --test-threads=1
//! ```
//!
//! One at a time, because there is one Excel and they take turns at it.
//!
//! On a machine with no Excel, or an operating system with no backend, they
//! skip and say so. `XLSPLICE_ORACLE=require` turns a skip into a failure,
//! for the machine where the oracle is meant to run.
//!
//! ## What is put in front of Excel
//!
//! The three fixtures Excel saved, which must open clean, and a package whose
//! worksheet envelope has been put out of order, which must not — the pair
//! that says a clean verdict means something.
//!
//! Then every shape the spec names: each write type over a cell that was
//! there; a cell and a row that were not; a date serial in an inserted cell
//! taking a date style from its column and, separately, from its row; a
//! shared-string cell overwritten with inline text; a replaced formula with
//! its chain entry removed; the chain part removed when it became empty; a
//! created custom properties part; a created calculation element; and a
//! macro-enabled package written into.
//!
//! Then the fixed operation set of [`support::corpus`] over every fixture,
//! and over every corpus package where there is a corpus. That set is what
//! the byte-preservation suite in `corpus.rs` puts a whole package through;
//! the per-operation suites go further over the fixtures, and what they write
//! reaches Excel through the named cases above rather than through this.
//!
//! Three of the shapes no fixture has, because Excel saved none like them: a
//! row whose custom format is a date format, a workbook with no calculation
//! element, and a chain short enough to empty. Each is built or derived by
//! the case that needs it, and each such case asks Excel about the package it
//! starts from as well as the one xlsplice wrote, so that a repair is never
//! ambiguous about which of the two Excel objected to.

mod support;

use std::path::{Path, PathBuf};

use support::corpus;
use support::oracle::{Verdict, asked, decided, opened_by, requires};
use support::{Workspace, fixture, part_text, verb};
use xlsplice::batch::WriteType;

/// The three fixtures, which Excel saved and so must open clean.
const FIXTURES: [&str; 3] = ["plain.xlsx", "macros.xlsm", "feature.xlsx"];

const SHEET1: &str = "xl/worksheets/sheet1.xml";

/// How many cases put a package in front of Excel. Counted rather than
/// bounded, so that a case which stopped reaching Excel is as much a failure
/// as one added without the `--ignored` gate.
const CASES: usize = 18;

/// A writable copy of a fixture, and the workspace holding it alive.
///
/// Even a test that writes nothing takes a copy: Excel is being pointed at
/// the file, and an Excel that decided to save would be writing over the
/// baseline every other suite compares against.
fn copy(label: &str, name: &str) -> (Workspace, PathBuf) {
    let workspace = Workspace::new(label);
    let path = workspace.copy_of(name);
    (workspace, path)
}

/// Ask the oracle, and assert the answer, unless there is no oracle to ask.
fn assert_verdict(package: &Path, wanted: Verdict, what: &str) {
    let Some(said) = asked(package) else { return };
    assert_eq!(said, wanted, "{what}");
}

#[test]
#[ignore = "drives Excel"]
fn the_fixtures_excel_saved_open_clean() {
    for name in FIXTURES {
        let (_workspace, path) = copy("oracle-fixture", name);

        assert_verdict(&path, Verdict::Clean, name);
    }
}

/// The other half of the pair: a package that is wrong in a way Excel minds,
/// so that a clean verdict is known to mean something. The worksheet's
/// envelope elements have a fixed order, and `dimension` moved to after
/// `sheetData` breaks it while leaving the part valid XML and the container
/// intact — the smallest thing that is wrong at the schema and nowhere else.
#[test]
#[ignore = "drives Excel"]
fn a_worksheet_whose_envelope_is_out_of_order_demands_repair() {
    let (workspace, path) = copy("oracle-broken", "plain.xlsx");
    let sheet = part_text(&path, SHEET1);
    let dimension = element(&sheet, "<dimension");
    let reordered = sheet.replacen(&dimension, "", 1).replacen(
        "</sheetData>",
        &format!("</sheetData>{dimension}"),
        1,
    );
    assert_ne!(reordered, sheet, "the test must have moved something");
    let broken = workspace.dir().join("broken.xlsx");
    rewritten(&path, &broken, SHEET1, &reordered);

    assert_verdict(&broken, Verdict::Repair, "a reordered worksheet envelope");
}

/// The writes the byte-preservation suites make, put in front of Excel. Each
/// is the write those suites already assert the bytes of; what is new here is
/// that Excel opens the result without a word.
#[test]
#[ignore = "drives Excel"]
fn a_number_write_opens_clean() {
    let (_workspace, path) = copy("oracle-number", "plain.xlsx");
    verb::set(&path, "Sheet1!A1", WriteType::Number, "42", None, false)
        .expect("a number write must land");

    assert_verdict(&path, Verdict::Clean, "a number written over a number");
}

#[test]
#[ignore = "drives Excel"]
fn a_text_write_opens_clean() {
    let (_workspace, path) = copy("oracle-text", "plain.xlsx");
    verb::set(&path, "Sheet1!A2", WriteType::Text, "written", None, false)
        .expect("a text write must land");

    assert_verdict(&path, Verdict::Clean, "text written over a number");
}

#[test]
#[ignore = "drives Excel"]
fn a_boolean_write_opens_clean() {
    let (_workspace, path) = copy("oracle-boolean", "plain.xlsx");
    verb::set(&path, "Sheet1!D1", WriteType::Bool, "false", None, false)
        .expect("a boolean write must land");

    assert_verdict(&path, Verdict::Clean, "a boolean written over a boolean");
}

/// The one that ADR-0001 rests on. Text is written as an inline string, so a
/// cell that pointed into the shared string table stops pointing at it and
/// the entry it used is left orphaned with a `count` that no longer adds up.
/// The claim there is that Excel does not mind; this is what says so.
#[test]
#[ignore = "drives Excel"]
fn a_shared_string_cell_overwritten_inline_opens_clean() {
    let (_workspace, path) = copy("oracle-shared", "plain.xlsx");
    verb::set(&path, "Sheet1!B1", WriteType::Text, "goodbye", None, false)
        .expect("a write over a shared string must land");
    assert!(
        part_text(&path, SHEET1).contains(r#"<c r="B1" t="inlineStr">"#),
        "the write must have gone in as an inline string for this to be the case it is"
    );

    assert_verdict(&path, Verdict::Clean, "an orphaned shared string");
}

/// A date is a number under a format, so what lands in the cell is a serial
/// and what says it is a date is the style the cell already carried. C1 of
/// the plain fixture is Excel's own date cell, which is the one to write into
/// for the question to be about the serial rather than about the format.
#[test]
#[ignore = "drives Excel"]
fn a_date_write_opens_clean() {
    let (_workspace, path) = copy("oracle-date", "plain.xlsx");
    verb::set(
        &path,
        "Sheet1!C1",
        WriteType::Date,
        "2026-12-25",
        None,
        false,
    )
    .expect("a date write must land");

    assert_verdict(&path, Verdict::Clean, "a serial written over a serial");
}

/// A template carries an element only for the cells something is already in,
/// so a hydration writes into cells the part does not hold. Row 1 of the
/// feature fixture holds A1, D1 and E1, so B1 goes in between two of them.
#[test]
#[ignore = "drives Excel"]
fn an_inserted_cell_opens_clean() {
    let (_workspace, path) = copy("oracle-cell", "feature.xlsx");
    verb::set(&path, "Inputs!B1", WriteType::Number, "9", None, false)
        .expect("a write to an absent cell must land");
    assert!(
        part_text(&path, SHEET1).contains(r#"<c r="B1"><v>9</v></c>"#),
        "the cell must have gone in for this to be the case it is"
    );

    assert_verdict(&path, Verdict::Clean, "a cell put into a row");
}

/// The same, one level up: the sheet holds rows 1 to 5 and row 7, and the
/// row put in after the last of them takes the cell with it. The dimension
/// element still says `A1:G7`, which is what this case is really asking Excel
/// about: a sheet holding a cell outside the range it declares.
#[test]
#[ignore = "drives Excel"]
fn an_inserted_row_opens_clean() {
    let (_workspace, path) = copy("oracle-row", "feature.xlsx");
    verb::set(&path, "Inputs!A9", WriteType::Number, "9", None, false)
        .expect("a write to an absent row must land");
    let written = part_text(&path, SHEET1);
    assert!(
        written.contains(r#"<row r="9"><c r="A9"><v>9</v></c></row>"#),
        "the row must have gone in for this to be the case it is"
    );
    assert!(
        written.contains(r#"<dimension ref="A1:G7"/>"#),
        "and the dimension must still be the one the fixture declared"
    );

    assert_verdict(&path, Verdict::Clean, "a row put into the sheet data");
}

/// A cell that was not there has no style of its own, so it takes the one
/// Excel would show it under. Column G of the feature fixture is styled with
/// a date format, so a date written into G1 renders as a date only if the
/// inherited style came with it.
#[test]
#[ignore = "drives Excel"]
fn a_date_inheriting_a_date_style_from_its_column_opens_clean() {
    let (_workspace, path) = copy("oracle-date-column", "feature.xlsx");
    verb::set(
        &path,
        "Inputs!G1",
        WriteType::Date,
        "2026-09-11",
        None,
        false,
    )
    .expect("a date written into an absent cell must land");
    assert!(
        part_text(&path, SHEET1).contains(r#"<c r="G1" s="2"><v>46276</v></c>"#),
        "the cell must have taken the column's date style for this to be the case it is"
    );

    assert_verdict(&path, Verdict::Clean, "a date under a column's style");
}

/// The other half of the inheritance, which no fixture can be asked for as it
/// stands: row 7 declares a custom format, but the format it declares is a
/// bold one rather than a date one. So the case derives the package it needs,
/// moving that row to the date style the row's own cell already carries —
/// which is to say, to a style Excel itself wrote into that row. The package
/// it derived is put to Excel too, so a repair says which of the two Excel
/// minded.
///
/// A fixture saved with such a row would take the derivation out of it, and
/// #28 asks for one.
#[test]
#[ignore = "drives Excel"]
fn a_date_inheriting_a_date_style_from_its_row_opens_clean() {
    let (workspace, path) = copy("oracle-date-row", "feature.xlsx");
    let sheet = part_text(&path, SHEET1);
    let dated = sheet.replacen(r#"s="1" customFormat="1""#, r#"s="3" customFormat="1""#, 1);
    assert_ne!(
        dated, sheet,
        "the test must have moved the row to a date style"
    );
    let derived = workspace.dir().join("dated-row.xlsx");
    rewritten(&path, &derived, SHEET1, &dated);
    assert_verdict(&derived, Verdict::Clean, "the package this case derives");

    verb::set(
        &derived,
        "Inputs!B7",
        WriteType::Date,
        "2026-09-11",
        None,
        false,
    )
    .expect("a date written into an absent cell must land");

    assert!(
        part_text(&derived, SHEET1).contains(r#"<c r="B7" s="3"><v>46276</v></c>"#),
        "the cell must have taken the row's date style for this to be the case it is"
    );
    assert_verdict(&derived, Verdict::Clean, "a date under a row's style");
}

/// A formula replaced by a value leaves its entry in the calc chain naming a
/// cell that no longer holds a formula, which is the inconsistency the chain
/// maintenance exists to prevent. D1 of the feature fixture is a plain
/// formula with an entry of its own, and both go in the one operation.
#[test]
#[ignore = "drives Excel"]
fn a_replaced_formula_and_the_chain_entry_it_took_with_it_open_clean() {
    let (_workspace, path) = copy("oracle-formula", "feature.xlsx");
    verb::batch(
        &path,
        vec![verb::replacing(verb::writing(
            "Inputs!D1",
            WriteType::Number,
            "15",
        ))],
        None,
        false,
    )
    .expect("a licensed write over a formula must land");
    assert!(
        !part_text(&path, "xl/calcChain.xml").contains(r#"r="D1""#),
        "the entry must have gone for this to be the case it is"
    );

    assert_verdict(&path, Verdict::Clean, "a formula replaced by its value");
}

/// The last formula of a package takes the whole chain with it: the part
/// goes, and so do the relationship that reached it and the content-type
/// override that named it, because a package naming a part it does not hold
/// is exactly what Excel offers to repair. No fixture carries a chain short
/// enough to empty, so the package is built for the case, and Excel is asked
/// about it before it is written into as well as after.
#[test]
#[ignore = "drives Excel"]
fn a_package_whose_calc_chain_became_empty_opens_clean() {
    let workspace = Workspace::new("oracle-chain");
    let path = workspace.chained_package(
        "chained.xlsx",
        r#"<c r="A1"><f>1+1</f><v>2</v></c>"#,
        r#"<c r="A1" i="1"/>"#,
    );
    assert_verdict(&path, Verdict::Clean, "the package this case starts from");

    verb::batch(
        &path,
        vec![verb::replacing(verb::writing(
            "Inputs!A1",
            WriteType::Number,
            "2",
        ))],
        None,
        false,
    )
    .expect("a licensed write over the one formula must land");

    assert!(
        !support::parts(&path)
            .iter()
            .any(|part| part.path == "xl/calcChain.xml"),
        "the chain must have gone for this to be the case it is"
    );
    assert_verdict(&path, Verdict::Clean, "a package whose chain was emptied");
}

/// A template with no custom properties gets the part, its relationship and
/// its content-type override, all three at once. The plain fixture carries
/// none, so it is the one to stamp.
#[test]
#[ignore = "drives Excel"]
fn a_created_custom_properties_part_opens_clean() {
    let (_workspace, path) = copy("oracle-props", "plain.xlsx");
    assert!(
        !support::parts(&path)
            .iter()
            .any(|part| part.path == "docProps/custom.xml"),
        "the plain fixture carries no custom properties, which is why it is this case"
    );

    verb::batch(
        &path,
        vec![verb::stamping(
            "Stamp.Text",
            WriteType::Text,
            "xlsplice was here",
        )],
        None,
        false,
    )
    .expect("a property set must land");

    assert_verdict(&path, Verdict::Clean, "a created custom properties part");
}

/// A workbook with no calculation element gets one, after its sheets. Every
/// fixture carries the element Excel writes, so the case derives a package
/// without it — the element is optional, and a package Excel saved without
/// one is a package Excel opens — and asks Excel about that as well.
#[test]
#[ignore = "drives Excel"]
fn a_created_calculation_element_opens_clean() {
    let (workspace, path) = copy("oracle-calc", "plain.xlsx");
    let workbook = part_text(&path, "xl/workbook.xml");
    let without = workbook.replacen(r#"<calcPr calcId="181029"/>"#, "", 1);
    assert_ne!(
        without, workbook,
        "the test must have taken the element out"
    );
    let derived = workspace.dir().join("no-calc.xlsx");
    rewritten(&path, &derived, "xl/workbook.xml", &without);
    assert_verdict(&derived, Verdict::Clean, "the package this case derives");

    verb::batch(&derived, vec![verb::calculating(true)], None, false).expect("the flag must land");

    assert!(
        part_text(&derived, "xl/workbook.xml").contains(r#"<calcPr fullCalcOnLoad="1"/>"#),
        "the element must have been created for this to be the case it is"
    );
    assert_verdict(&derived, Verdict::Clean, "a created calculation element");
}

/// A macro-enabled package passes through with its VBA project untouched, and
/// Excel is the one that says whether a project it did not write the
/// container around still loads.
#[test]
#[ignore = "drives Excel"]
fn a_write_into_a_macro_enabled_package_opens_clean() {
    let (_workspace, path) = copy("oracle-macros", "macros.xlsm");
    verb::set(&path, "Sheet1!A1", WriteType::Number, "42", None, false)
        .expect("a write into a macro-enabled package must land");
    assert_eq!(
        support::part(&path, "xl/vbaProject.bin"),
        support::part(&fixture("macros.xlsm"), "xl/vbaProject.bin"),
        "the project must have been copied raw for this to be the case it is"
    );

    assert_verdict(
        &path,
        Verdict::Clean,
        "a macro-enabled package written into",
    );
}

/// Every output of the byte-preservation cases, put in front of Excel: the
/// fixed operation set of `support::corpus`, over the three fixtures. What
/// those suites assert is that nothing outside the target moved; what this
/// asserts is that Excel opens the result of each.
#[test]
#[ignore = "drives Excel"]
fn every_output_of_the_fixed_operation_set_over_the_fixtures_opens_clean() {
    for name in FIXTURES {
        opens_clean_after_the_fixed_set(&fixture(name));
    }
}

/// The same, over the real templates, which is the pair of cases the spec
/// asks for: every output of the byte-preservation suites, over the corpus
/// and over the fixtures. Absent the corpus variable there is no corpus, and
/// this skips whether or not there is an Excel.
#[test]
#[ignore = "drives Excel"]
fn every_output_of_the_fixed_operation_set_over_the_corpus_opens_clean() {
    let Some(packages) = corpus::packages() else {
        return;
    };

    for package in &packages {
        opens_clean_after_the_fixed_set(package);
    }
}

/// Put one package through the fixed set, a case at a time, and ask Excel
/// about each result. What the case did to the package is asserted by the
/// byte-preservation suite over the same set; what is asked here is only
/// whether Excel opens it.
fn opens_clean_after_the_fixed_set(package: &Path) {
    for case in corpus::cases(package) {
        let workspace = Workspace::new("oracle-set");

        let (copy, _) = corpus::applied(&workspace, package, &case);

        assert_verdict(
            &copy,
            Verdict::Clean,
            &format!("{}: {}", package.display(), case.label),
        );
    }
}

/// The skip is a behaviour, not an accident, so it is asserted rather than
/// left to be noticed. A machine with no Excel is simulated by pointing the
/// oracle somewhere Excel is not, which is the whole of what absence means to
/// it — nothing is launched, so this one runs in an ordinary build.
#[test]
fn a_machine_without_excel_answers_unavailable_rather_than_guessing() {
    let (_workspace, path) = copy("oracle-absent", "plain.xlsx");

    let said = opened_by(Path::new("/Applications/No Such Excel.app"), &path);

    let Verdict::Unavailable(why) = said else {
        panic!("there is no Excel there, so there is no verdict to be had")
    };
    assert!(
        why.contains("not installed") || why.contains("no oracle backend"),
        "the reason must say which of the two it is: {why}"
    );
}

#[test]
fn no_oracle_is_a_skip_that_says_why_rather_than_a_failure() {
    let nothing = Verdict::Unavailable("Excel is not installed".to_owned());

    assert_eq!(decided(nothing, false), None, "absence is not a failure");
    assert_eq!(
        decided(Verdict::Clean, false),
        Some(Verdict::Clean),
        "an answer is passed through whatever the policy is"
    );
}

/// The other half: on the machine the oracle is meant to run on, a skip means
/// the harness has broken rather than the machine being the wrong one, and
/// the environment says so.
#[test]
#[should_panic(expected = "the oracle could not answer")]
fn requiring_an_oracle_turns_absence_into_a_failure() {
    decided(
        Verdict::Unavailable("Excel is not installed".to_owned()),
        true,
    );
}

#[test]
fn only_require_requires_an_oracle() {
    assert!(requires(Some("require")));
    assert!(!requires(Some("skip")));
    assert!(!requires(Some("")));
    assert!(
        !requires(None),
        "unset is the ordinary machine, which skips"
    );
}

/// The `--ignored` gate is what keeps Excel out of an ordinary build, so it is
/// held to rather than trusted: every case here that puts a package in front
/// of Excel carries the attribute. The suite reads itself to say so, because
/// the thing that goes wrong is a case added without it. A case reaches Excel
/// when it calls `assert_verdict` on a package, or the one thing that does
/// that for it, and both are what is looked for.
#[test]
fn every_case_that_reaches_excel_is_ignored_by_default() {
    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file!()))
        .expect("the suite must be able to read itself");
    // The two calls that put a package in front of Excel, each spelled in
    // halves so that this test does not go off on its own source.
    let reaches = [
        format!("{}{}", "assert_verdict", "("),
        format!("{}{}", "opens_clean_after_the_fixed_set", "("),
    ];
    let mut reaching = 0;

    for case in source.split("\n#[test]").skip(1) {
        let name = case
            .split_once("\nfn ")
            .and_then(|(_, rest)| rest.split_once('('))
            .map(|(name, _)| name)
            .expect("a test has a name");
        if !reaches.iter().any(|call| case.contains(call)) {
            continue;
        }
        reaching += 1;
        assert!(
            case.starts_with("\n#[ignore"),
            "{name} puts a package in front of Excel and is not #[ignore]d"
        );
    }

    assert_eq!(
        reaching, CASES,
        "the oracle cases must be the ones counted, and there are {CASES} of them"
    );
}

/// One element of `xml`, from its opening angle bracket to the `/>` that
/// closes it. Enough for the empty elements this suite moves about.
fn element(xml: &str, opening: &str) -> String {
    let start = xml
        .find(opening)
        .unwrap_or_else(|| panic!("the part must hold {opening}: {xml}"));
    let end = xml[start..]
        .find("/>")
        .expect("the element must be an empty one")
        + start
        + 2;
    xml[start..end].to_owned()
}

/// A copy of `from` at `to` with one part's text replaced. The container is
/// rebuilt rather than spliced, because this is a test building a package
/// that is deliberately wrong, not the tool writing one.
fn rewritten(from: &Path, to: &Path, part: &str, text: &str) {
    let reader = std::fs::File::open(from).expect("a test must be able to read its own copy");
    let mut zip = zip::ZipArchive::new(reader).expect("the fixture must be a container");
    let writer = std::fs::File::create(to).expect("a test must be able to write its own package");
    let mut out = zip::ZipWriter::new(writer);
    for index in 0..zip.len() {
        use std::io::Read;
        let mut entry = zip.by_index(index).expect("an entry the archive listed");
        let name = entry.name().to_owned();
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        out.start_file(&name, options)
            .expect("a test must be able to write an entry");
        use std::io::Write;
        if name == part {
            out.write_all(text.as_bytes())
        } else {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .expect("an entry must be readable");
            out.write_all(&bytes)
        }
        .expect("a test must be able to write an entry");
    }
    out.finish().expect("the container must close");
}
