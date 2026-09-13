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

mod support;

use std::path::{Path, PathBuf};

use support::oracle::{Verdict, asked, decided, opened_by, requires};
use support::{Workspace, part_text, verb};
use xlsplice::batch::WriteType;

/// The three fixtures, which Excel saved and so must open clean.
const FIXTURES: [&str; 3] = ["plain.xlsx", "macros.xlsm", "feature.xlsx"];

const SHEET1: &str = "xl/worksheets/sheet1.xml";

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
/// when it calls `assert_verdict` on a package, which is what is looked for.
#[test]
fn every_case_that_reaches_excel_is_ignored_by_default() {
    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file!()))
        .expect("the suite must be able to read itself");
    // The call that puts a package in front of Excel, spelled in two halves
    // so that this test does not go off on its own source.
    let reaches = format!("{}{}", "assert_verdict", "(&");
    let mut reaching = 0;

    for case in source.split("\n#[test]").skip(1) {
        let name = case
            .split_once("\nfn ")
            .and_then(|(_, rest)| rest.split_once('('))
            .map(|(name, _)| name)
            .expect("a test has a name");
        if !case.contains(&reaches) {
            continue;
        }
        reaching += 1;
        assert!(
            case.starts_with("\n#[ignore"),
            "{name} puts a package in front of Excel and is not #[ignore]d"
        );
    }

    assert!(
        reaching >= 6,
        "the oracle cases must be the ones counted: {reaching}"
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
