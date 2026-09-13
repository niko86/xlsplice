//! The per-part guarantee, held over whole packages rather than over one
//! splice at a time.
//!
//! Every other write suite pins what one operation does to one part: the part
//! afterwards is the part before it with exactly one substitution. That is the
//! assertion that says the splice landed where it was meant to. It says
//! nothing about the parts a template carries that the fixtures do not —
//! charts, drawings, media, printer settings, ActiveX controls, a VBA project
//! — because the fixtures carry none of them.
//!
//! So this suite runs the fixed operation set of [`support::corpus`] over
//! whole packages and asserts the guarantee itself: nothing outside the parts
//! the operation may touch moved a byte, nothing was added or removed that
//! was not accounted for, the parts stayed in the order the package held them
//! in, and the tool's own account of what it changed is exactly what the
//! container shows. The judge is the comparator in `support`, which reads both
//! containers itself.
//!
//! It runs over two sets of packages. The four fixtures run everywhere,
//! including CI, which is what keeps the machinery here honest on a machine
//! with no corpus. The corpus — real vendor templates, kept outside this
//! repository in the directory `XLSPLICE_CORPUS` names — runs where there is
//! one and skips where there is not.
//!
//! Nothing here writes to a corpus package or anywhere near the repository: a
//! case copies the package into a workspace under the temporary directory and
//! writes to the copy, comparing it against the original it never opened for
//! writing.

mod support;

use std::path::{Path, PathBuf};

use support::corpus::{self, Case};
use support::{Workspace, assert_nothing_outside, compare};
use xlsplice::batch::Report;

/// The fixtures run everywhere, corpus or no corpus, which is what says the
/// machinery here still works on a machine that has never seen a template.
#[test]
fn the_fixed_operation_set_holds_the_guarantee_over_every_fixture() {
    for name in [
        "plain.xlsx",
        "macros.xlsm",
        "feature.xlsx",
        "dated-row.xlsx",
    ] {
        holds_the_guarantee(&support::fixture(name));
    }
}

/// The same, over the real templates. Absent the variable there is no corpus
/// and this is a skip, which is said out loud rather than passed over in
/// silence: a suite nobody notices has stopped running is one that has.
#[test]
fn the_fixed_operation_set_holds_the_guarantee_over_every_corpus_package() {
    let Some(packages) = corpus::packages() else {
        return;
    };

    for package in &packages {
        holds_the_guarantee(package);
    }
    eprintln!(
        "the corpus: {} package(s) held the guarantee",
        packages.len()
    );
}

/// The corpus is vendor material and the repository is not where it, or
/// anything derived from it, may land. What a case writes to is a workspace,
/// so it is the workspace that is held to it.
#[test]
fn nothing_a_case_writes_lands_inside_the_repository() {
    let workspace = Workspace::new("outside");
    let package = workspace.copy_from(&support::fixture("plain.xlsx"));

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !workspace.dir().starts_with(repository),
        "a workspace is outside the repository: {}",
        workspace.dir().display()
    );
    assert!(
        !package.starts_with(repository),
        "and so is the copy a case writes to: {}",
        package.display()
    );
}

/// The variable is the whole of what says there is a corpus, and unset is the
/// ordinary machine. Empty counts as unset, because a variable exported as
/// nothing is how a shell spells one it does not have.
#[test]
fn without_the_variable_there_is_no_corpus_to_run_over() {
    assert_eq!(corpus::named(None), None, "unset is no corpus");
    assert_eq!(corpus::named(Some("")), None, "and so is empty");
    assert_eq!(
        corpus::named(Some("/templates")),
        Some(PathBuf::from("/templates")),
        "a directory named is the corpus"
    );
}

/// What the fixed set is, spelled out, so that a case quietly dropped from it
/// is a failure rather than a suite that got shorter.
#[test]
fn the_fixed_set_is_the_eight_operations_the_spec_names() {
    let planned: Vec<String> = corpus::cases(&support::fixture("feature.xlsx"))
        .into_iter()
        .map(|case| case.label)
        .collect();

    assert_eq!(
        planned,
        [
            "a number write to Inputs!A1",
            "a text write to Inputs!A1",
            "a bool write to Inputs!A1",
            "a date write to Inputs!A1",
            "a write to Inputs!B1, a cell the sheet does not hold",
            "a write to Inputs!A6, in a row the sheet does not hold",
            "a property set",
            "the calculate-on-load flag",
        ]
    );
}

/// The cells the cases are pointed at are found rather than given, so what
/// the finding answers is asserted: a cell the sheet holds and no formula
/// sits in, one the sheet does not hold, and one in a row it does not hold.
#[test]
fn the_cells_a_package_is_written_at_are_a_present_one_and_two_absent_ones() {
    let writable = corpus::writable(&support::fixture("feature.xlsx"))
        .expect("the feature fixture holds a cell that can be written to");

    assert_eq!(writable.sheet, "Inputs");
    assert_eq!(writable.part, "xl/worksheets/sheet1.xml");
    assert_eq!(
        writable.present.as_deref(),
        Some("Inputs!A1"),
        "the first cell of the first row, which holds a number"
    );
    assert_eq!(
        writable.absent.as_deref(),
        Some("Inputs!B1"),
        "the first column that row does not hold"
    );
    assert_eq!(
        writable.in_an_absent_row, "Inputs!A6",
        "the first row the sheet does not hold, which is the gap at six"
    );
}

/// A formula cell is passed over, because a write to one is refused without
/// the replace-formula flag and the fixed set carries none. A sheet whose
/// cells are all formulas offers no cell to write over, and the four writes
/// that need one are not planned. What it still takes is a cell put into its
/// row, beside the formulas rather than over one, and a row of its own.
#[test]
fn a_sheet_of_nothing_but_formulas_offers_no_cell_to_write_over() {
    let workspace = Workspace::new("all-formulas");
    let package = workspace.sheet_package(
        "formulas.xlsx",
        "",
        r#"<c r="A1"><f>1+1</f><v>2</v></c><c r="B1"><f>2+2</f><v>4</v></c>"#,
    );

    let writable = corpus::writable(&package).expect("the sheet still takes a cell and a row");

    assert_eq!(
        writable.present, None,
        "every cell it holds holds a formula"
    );
    assert_eq!(
        writable.absent.as_deref(),
        Some("Inputs!C1"),
        "the first column its row does not hold, which no formula is in"
    );
    assert_eq!(writable.in_an_absent_row, "Inputs!A2");
}

/// A template that starts empty holds no cell to write over and no row to put
/// one in, and still takes a row. One of the the vendor system templates is exactly
/// that: every sheet a `<sheetData/>`, which is the shape a hydration fills
/// from nothing.
#[test]
fn a_sheet_holding_nothing_at_all_still_takes_a_row() {
    let workspace = Workspace::new("empty-sheet");
    let package = two_sheets(&workspace, "empty.xlsx", &empty_sheet(), &empty_sheet());

    let planned: Vec<String> = corpus::cases(&package)
        .into_iter()
        .map(|case| case.label)
        .collect();

    assert_eq!(
        planned,
        [
            "a write to Inputs!A1, in a row the sheet does not hold",
            "a property set",
            "the calculate-on-load flag",
        ]
    );
}

/// A sheet holding cells is preferred to one holding none, because it has six
/// cases to answer rather than one.
#[test]
fn a_sheet_holding_cells_is_preferred_to_one_holding_none() {
    let workspace = Workspace::new("prefer-cells");
    let package = two_sheets(
        &workspace,
        "mixed.xlsx",
        &empty_sheet(),
        support::NOTES_SHEET,
    );

    let writable = corpus::writable(&package).expect("the second sheet holds a cell");

    assert_eq!(writable.sheet, "Notes");
    assert_eq!(writable.present.as_deref(), Some("Notes!A5"));
}

/// A sheet carrying a table is passed over while another sheet will do. A
/// table part declares its own range and the headers in it, so a write into
/// one of those cells leaves the two disagreeing — which is a question about
/// where the caller aimed rather than about the splice, and not one to have a
/// corpus case turn on.
#[test]
fn a_sheet_carrying_a_table_is_passed_over_while_another_sheet_will_do() {
    let workspace = Workspace::new("tables");
    let package = two_sheets(&workspace, "tabled.xlsx", &tabled(), support::NOTES_SHEET);

    let writable = corpus::writable(&package).expect("the second sheet holds a cell");

    assert_eq!(writable.sheet, "Notes", "the sheet with no table in it");
    assert_eq!(writable.present.as_deref(), Some("Notes!A5"));
}

/// Where every sheet carries one, the first is used anyway: no cell at all is
/// worse than an awkward one.
#[test]
fn where_every_sheet_carries_a_table_the_first_is_used_regardless() {
    let workspace = Workspace::new("all-tables");
    let package = two_sheets(&workspace, "tabled.xlsx", &tabled(), &tabled());

    let writable = corpus::writable(&package).expect("a sheet with a table still holds a cell");

    assert_eq!(writable.sheet, "Inputs");
    assert_eq!(writable.present.as_deref(), Some("Inputs!A1"));
}

/// A package of the feature workbook with only its first two sheets in it,
/// each written out by the caller. The third sheet the workbook declares is
/// not there, which is a sheet the finding cannot read and passes over.
fn two_sheets(workspace: &Workspace, name: &str, first: &str, second: &str) -> PathBuf {
    workspace.zip(
        name,
        &[
            (support::CONTENT_TYPES_PART, support::FEATURE_CONTENT_TYPES),
            (support::ROOT_RELS_PART, support::ROOT_RELS),
            (support::WORKBOOK_PART, &support::feature_workbook()),
            (support::WORKBOOK_RELS_PART, support::WORKBOOK_RELS),
            (support::SHEET1_PART, first),
            (support::SHEET2_PART, second),
        ],
    )
}

/// A worksheet holding no cells at all, which is what a template that starts
/// empty carries.
fn empty_sheet() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#
        .to_owned()
}

/// A worksheet holding one cell that could be written into, and a table that
/// says the cell is its header.
fn tabled() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData><row r="1" spans="1:1"><c r="A1" t="inlineStr"><is><t>Header</t></is></c></row></sheetData>
  <tableParts count="1"><tablePart r:id="rId9"/></tableParts>
</worksheet>"#
        .to_owned()
}

/// Put one package through the fixed set, a case at a time, each from the
/// package as it was, and say what it was put through.
///
/// The count is printed because six of the eight cases need something of the
/// package: four a cell holding no formula, one a row to put a cell into.
/// A template that offers less is put through less, and a run that says so is
/// one where that is noticed rather than passed over.
fn holds_the_guarantee(package: &Path) {
    let cases = corpus::cases(package);
    assert!(
        cases.len() >= 3,
        "{}: every package takes a row, a property and the flag at the least",
        package.display()
    );
    eprintln!("{}: {} case(s)", package.display(), cases.len());

    for case in cases {
        let workspace = Workspace::new("corpus");

        let (copy, report) = corpus::applied(&workspace, package, &case);

        assert_the_guarantee(package, &copy, &report, &case);
    }
}

/// The guarantee itself, for one case: what the container shows against what
/// the operation was allowed to do and what the tool said it did.
fn assert_the_guarantee(before: &Path, after: &Path, report: &Report, case: &Case) {
    let what = format!("{}: {}", before.display(), case.label);
    assert_nothing_outside(before, after, &case.may_touch, &what);

    // The tool says which parts it changed, added and removed. The container
    // says the same thing, read by something that is no part of the tool, and
    // the two are held against each other: a report that is wrong about what
    // it did is as much a failure as a splice that went astray.
    let comparison = compare(before, after);
    assert_eq!(
        sorted(&comparison.differs),
        sorted(&report.parts.changed),
        "{what}: the report and the container disagree about what changed"
    );
    assert_eq!(
        sorted(&comparison.added),
        sorted(&report.parts.added),
        "{what}: the report and the container disagree about what was added"
    );
    assert_eq!(
        sorted(&comparison.removed),
        sorted(&report.parts.removed),
        "{what}: the report and the container disagree about what was removed"
    );

    // And whether it changed anything at all. A write of the value already
    // there changes nothing and still landed, so what is held to is that the
    // tool's word and the container agree; two of the cases change something
    // whatever the package held, and a case that quietly did nothing is what
    // that catches.
    let moved = !comparison.differs.is_empty() || !comparison.added.is_empty();
    assert_eq!(
        report.operations[0].changed, moved,
        "{what}: the operation says it changed {}, and the container says {moved}",
        report.operations[0].changed
    );
    if case.must_change {
        assert!(moved, "{what}: this case changes something in any package");
    }
}

/// The same paths in one order, so that a report in path order and a container
/// in its own order are compared on what they hold rather than on how they
/// hold it.
fn sorted(paths: &[String]) -> Vec<String> {
    let mut sorted = paths.to_vec();
    sorted.sort();
    sorted
}
