//! The corpus, and the fixed operation set every package is put through.
//!
//! A fixture is a small package saved into this repository; a corpus package
//! is a real one, vendor material, kept outside it in the directory
//! [`DIRECTORY`] names. Nothing here ever writes to a corpus package: a case
//! takes a copy into a workspace under the temporary directory and writes to
//! that, so the corpus is read and never touched, and nothing derived from it
//! lands anywhere near the repository.
//!
//! Absent the variable there is no corpus, and a suite over it skips. What
//! does not skip is the same operation set over the four fixtures, which is
//! how the machinery here is held to on a machine that has no corpus at all.
//!
//! ## What one package is put through
//!
//! [`cases`] answers with them: a number, text, boolean and date write to
//! a cell the sheet holds, a write to a cell it does not, a write to a row it
//! does not, a property set, and the calculate-on-load flag. Each is one
//! operation applied on its own, because several of them name one cell and a
//! batch naming a cell twice is refused (ADR-0004).
//!
//! Where those land is not something a corpus package can be asked for in
//! advance, so [`writable`] finds it: the first cell of the first sheet that
//! holds one and no formula, the first column that sheet's row does not hold,
//! and the first row the sheet does not hold. A formula cell is passed over
//! because writing one is refused without the replace-formula flag, and the
//! fixed set carries no flag: what it exercises is the ordinary write.
//!
//! Every case also carries the parts it may touch, which is what says a
//! number write has no business in the styles part. The guarantee itself is
//! asserted by the suite, against the comparator in [`super`], never against
//! anything the tool says about itself.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use xlsplice::batch::{self, Batch, Destination, Opened, Operation, Report, WriteType};
use xlsplice::declared::CONTENT_TYPES;
use xlsplice::package::Package;
use xlsplice::reference::{Address, Cell, MAX_COLUMN, MAX_ROW, quote_sheet_name};
use xlsplice::relationships::{ROOT_RELS, Relationships};
use xlsplice::target::resolve;
use xlsplice::workbook::Workbook;

/// The variable naming the directory the corpus lives in. Unset is a machine
/// with no corpus, which is every machine but the maintainer's.
pub const DIRECTORY: &str = "XLSPLICE_CORPUS";

/// The extensions a corpus package is one of. A `.xlsb` is a package
/// xlsplice has nothing to say about, so it is no part of the corpus either.
const EXTENSIONS: [&str; 2] = ["xlsx", "xlsm"];

/// One case of the fixed set: one operation, and the parts it may touch.
#[derive(Debug, Clone)]
pub struct Case {
    /// What the case does, for a failure to name.
    pub label: String,
    /// The operation, applied as a batch of one.
    pub operation: Operation,
    /// Every part this operation may have changed or added. A part outside
    /// them must be byte for byte what it was; one inside them need not have
    /// moved at all, because a write of the value already there changes
    /// nothing.
    pub may_touch: Vec<String>,
    /// Whether this case must have changed a byte.
    ///
    /// Most of them need not: a write of the value a cell already holds is a
    /// write that landed, and it is the tool saying so that is asserted. Two
    /// of them must, whatever the package holds — a cell the sheet did not
    /// hold becomes one it does, and a property named the way no template
    /// names one is a property that was not there — and they are what says a
    /// case which quietly did nothing at all is a failure.
    pub must_change: bool,
}

/// Where in one package the cell cases write.
///
/// Two of the three are what the sheet happens to hold. A template that
/// starts empty — and one of the the vendor system templates is exactly that, every
/// sheet a `<sheetData/>` — holds no cell to write over and no row to put a
/// cell into, but it still takes a row, which is the write a hydration into
/// it would make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writable {
    /// The sheet, in the package's own spelling.
    pub sheet: String,
    /// The worksheet part it sits in, which is the only part a cell case may
    /// touch.
    pub part: String,
    /// A cell the sheet holds with no formula in it, where it holds one.
    pub present: Option<String>,
    /// A cell the sheet does not hold, in the first row it does — where it
    /// holds a row at all.
    pub absent: Option<String>,
    /// A cell in a row the sheet does not hold. Every sheet with a sheet data
    /// element has one of these, empty or not.
    pub in_an_absent_row: String,
}

/// The corpus, or `None` where there is none and a suite over it is to skip.
///
/// The reason is printed rather than swallowed, here rather than at each
/// caller, because a suite that quietly skipped is one nobody notices has
/// stopped running.
///
/// A variable naming a directory that is not there is a broken setting rather
/// than an absent corpus, so it fails rather than skipping: the machine that
/// sets it is the machine the corpus suite is meant to run on.
pub fn packages() -> Option<Vec<PathBuf>> {
    let Some(directory) = named(std::env::var(DIRECTORY).ok().as_deref()) else {
        eprintln!("skipping: {DIRECTORY} is not set, so there is no corpus");
        return None;
    };
    assert!(
        directory.is_dir(),
        "{DIRECTORY} names {}, which is not a directory",
        directory.display()
    );
    let mut found = Vec::new();
    collect(&directory, &mut found);
    found.sort();
    assert!(
        !found.is_empty(),
        "{DIRECTORY} names {}, which holds no .xlsx or .xlsm at all",
        directory.display()
    );
    Some(found)
}

/// The reading of [`DIRECTORY`], apart from the environment so that a test can
/// say what unset means without unsetting anything.
///
/// Unset and empty are both no corpus: a variable exported as nothing is how
/// a shell spells one it does not have.
pub fn named(set_to: Option<&str>) -> Option<PathBuf> {
    match set_to.map(str::trim) {
        None | Some("") => None,
        Some(directory) => Some(PathBuf::from(directory)),
    }
}

/// Every package under `directory`, and under anything under it.
///
/// Excel's own lock files are `~$` in front of a package's name and are not
/// packages; anything starting with a dot is the operating system's rather
/// than the corpus's.
fn collect(directory: &Path, into: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", directory.display()));
    for entry in entries {
        let path = entry.expect("an entry of the corpus directory").path();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') || name.starts_with("~$") {
            continue;
        }
        if path.is_dir() {
            collect(&path, into);
        } else if is_a_package(&path) {
            into.push(path);
        }
    }
}

/// Whether the file at `path` is a package by its extension.
fn is_a_package(path: &Path) -> bool {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .is_some_and(|extension| EXTENSIONS.contains(&extension.as_str()))
}

/// The fixed operation set for one package.
///
/// The property and the flag are asked of every package. The cell cases are
/// asked of what the package can take: four writes need a cell the sheet
/// holds and no formula sits in, the fifth needs a row to put a cell into,
/// and the sixth needs only a sheet. A template that offers less is put
/// through less rather than through nothing, and the suite says how many
/// cases each package was put through so that a short plan is noticed.
pub fn cases(package: &Path) -> Vec<Case> {
    let mut opened = Opened::open(package)
        .unwrap_or_else(|err| panic!("{} must open: {err}", package.display()));
    let (properties, held) = opened.properties().unwrap_or_else(|err| {
        panic!(
            "{} must say where its properties go: {err}",
            package.display()
        )
    });
    let declarations = match held {
        true => vec![properties.clone()],
        false => vec![
            properties.clone(),
            CONTENT_TYPES.to_owned(),
            ROOT_RELS.to_owned(),
        ],
    };
    let workbook = vec![opened.workbook_part().to_owned()];

    let mut cases = Vec::new();
    if let Some(writable) = writable(package) {
        let part = vec![writable.part.clone()];
        if let Some(present) = &writable.present {
            for (write_type, value) in [
                (WriteType::Number, "1234.5"),
                (WriteType::Text, "xlsplice wrote this"),
                (WriteType::Bool, "true"),
                (WriteType::Date, "2026-09-11"),
            ] {
                cases.push(Case {
                    label: format!("a {} write to {present}", write_type.as_str()),
                    operation: writing(present, write_type, value),
                    may_touch: part.clone(),
                    must_change: false,
                });
            }
        }
        if let Some(absent) = &writable.absent {
            cases.push(Case {
                label: format!("a write to {absent}, a cell the sheet does not hold"),
                operation: writing(absent, WriteType::Number, "1234.5"),
                may_touch: part.clone(),
                must_change: true,
            });
        }
        cases.push(Case {
            label: format!(
                "a write to {}, in a row the sheet does not hold",
                writable.in_an_absent_row
            ),
            operation: writing(&writable.in_an_absent_row, WriteType::Number, "1234.5"),
            may_touch: part,
            must_change: true,
        });
    }
    cases.push(Case {
        label: "a property set".to_owned(),
        operation: Operation::PropsSet {
            name: "xlsplice.Corpus".to_owned(),
            write_type: WriteType::Text,
            value: "the corpus suite was here".to_owned(),
        },
        may_touch: declarations,
        must_change: true,
    });
    cases.push(Case {
        label: "the calculate-on-load flag".to_owned(),
        operation: Operation::Calc {
            full_calc_on_load: true,
        },
        may_touch: workbook,
        must_change: false,
    });
    cases
}

/// Apply one case to a copy of `package` in `workspace`, and give back the
/// copy and what the tool said it did.
///
/// The copy is what is written to, so the package itself — a fixture whose
/// bytes are a baseline, or a corpus template that is vendor material — is
/// read and never touched. One case is one batch: several of them name one
/// cell, and a batch naming a cell twice is refused (ADR-0004), so what is
/// put to the package is what one operation does to it rather than what eight
/// do to each other.
pub fn applied(workspace: &super::Workspace, package: &Path, case: &Case) -> (PathBuf, Report) {
    let copy = workspace.copy_from(package);
    let report = batch::run(
        &copy,
        &Batch::of(case.operation.clone()),
        &Destination::InPlace,
        false,
    )
    .unwrap_or_else(|err| {
        panic!(
            "{}: {} must be applied: {err}",
            package.display(),
            case.label
        )
    });
    (copy, report)
}

/// One `set` of the fixed set, which never licenses a formula: what the set
/// exercises is the ordinary write.
fn writing(target: &str, write_type: WriteType, value: &str) -> Operation {
    Operation::Set {
        target: target.to_owned(),
        write_type,
        value: value.to_owned(),
        replace_formula: false,
    }
}

/// Where the cell cases write in `package`, or `None` where no sheet of it
/// holds a cell that can be written to.
///
/// The sheets are gone through in the order the workbook declares them, and
/// the first that answers is the one used, so a package always plans the same
/// cases. What the worksheet part holds is read here with the container crate
/// and parsed with the locator, neither of which is the write path under
/// test: a target chosen wrongly makes a write fail, never a guarantee pass.
///
/// A sheet carrying a table is passed over while any other sheet will do. A
/// table part declares its own range and the headers in it, and a write into
/// one of those cells leaves the two disagreeing — which is Excel's business
/// with the caller who aimed there, not the splice's, and not something to
/// have a corpus case turn on. Where every sheet carries one, the first is
/// used anyway: no sheet at all is worse than an awkward one.
///
/// A sheet holding cells is preferred to one holding none, for the same
/// reason: a package whose first sheet is empty and whose second is full has
/// six cases to answer rather than one.
pub fn writable(package: &Path) -> Option<Writable> {
    let mut open = Package::open(package)
        .unwrap_or_else(|err| panic!("{} must open: {err}", package.display()));
    let workbook = Workbook::read(&mut open)
        .unwrap_or_else(|err| panic!("{} must hold a workbook: {err}", package.display()));
    let rels = Relationships::read(&mut open, workbook.part())
        .unwrap_or_else(|err| panic!("{} must hold its relationships: {err}", package.display()));

    let mut emptiest = None;
    for avoiding_tables in [true, false] {
        for sheet in workbook.sheets() {
            let named = format!("{}!A1", quote_sheet_name(&sheet.name));
            let Ok(resolved) = resolve(&open, &rels, &workbook, &named) else {
                continue;
            };
            let text = super::part_text(package, &resolved.part);
            if avoiding_tables && text.contains("<tableParts") {
                continue;
            }
            let Some(writable) = writable_in(&text, &resolved.address.sheet, &resolved.part) else {
                continue;
            };
            if writable.present.is_some() {
                return Some(writable);
            }
            emptiest = emptiest.or(Some(writable));
        }
    }
    emptiest
}

/// The same, for one sheet: the worksheet part's `text`, the sheet it is, and
/// the part it is.
fn writable_in(text: &str, sheet: &str, part: &str) -> Option<Writable> {
    let rows = rows_of(text)?;
    let at = |column, row| {
        Cell::new(column, row).map(|cell| {
            Address {
                sheet: sheet.to_owned(),
                cell,
            }
            .to_string()
        })
    };

    // The cell written over is the first with no formula in it. The cell put
    // in goes in the sheet's first row, whatever that row holds: a row of
    // nothing but formulas still has a column it does not hold, and putting a
    // cell there treads on none of them.
    let present = rows
        .iter()
        .find_map(|(row, cells)| first_without_a_formula(cells).map(|column| (*row, column)))
        .and_then(|(row, column)| at(column, row));
    let absent = rows.first().and_then(|(row, cells)| {
        let held: Vec<u32> = cells.iter().map(|(column, _)| *column).collect();
        first_missing(&held, MAX_COLUMN).and_then(|column| at(column, *row))
    });
    let numbers: Vec<u32> = rows.iter().map(|(number, _)| *number).collect();

    Some(Writable {
        part: part.to_owned(),
        present,
        absent,
        in_an_absent_row: at(1, first_missing(&numbers, MAX_ROW)?)?,
        sheet: sheet.to_owned(),
    })
}

/// The cells one row holds, as the column each sits in and whether a formula
/// sits in it.
type Cells = Vec<(u32, bool)>;

/// One row of a worksheet part: its number, and the cells it holds.
type Row = (u32, Cells);

/// Every row of a worksheet part. `None` where the part holds no sheet data
/// element at all, which is a sheet nothing can be written into.
fn rows_of(text: &str) -> Option<Vec<Row>> {
    let document = roxmltree::Document::parse(text).expect("a worksheet part must be XML");
    let data = document
        .descendants()
        .find(|node| node.has_tag_name("sheetData"))?;
    // A row may leave its number out and take the one its position gives it.
    // Nothing in a corpus template does, and a sheet where one did would have
    // rows this cannot count, so such a sheet is passed over rather than
    // guessed at.
    if data
        .children()
        .filter(|node| node.has_tag_name("row"))
        .any(|row| row.attribute("r").is_none())
    {
        return None;
    }
    Some(
        data.children()
            .filter(|node| node.has_tag_name("row"))
            .filter_map(|row| {
                let number = row.attribute("r")?.parse().ok()?;
                let cells = row
                    .children()
                    .filter(|node| node.has_tag_name("c"))
                    .filter_map(|cell| {
                        let at = Cell::parse(cell.attribute("r")?)?;
                        let formula = cell.children().any(|node| node.has_tag_name("f"));
                        Some((at.column(), formula))
                    })
                    .collect();
                Some((number, cells))
            })
            .collect(),
    )
}

/// The first column of a row that can be written into: one holding no
/// formula, because the fixed set carries no licence to replace one.
fn first_without_a_formula(cells: &Cells) -> Option<u32> {
    cells
        .iter()
        .find(|(_, formula)| !formula)
        .map(|(column, _)| *column)
}

/// The first number from one up to `last` that `held` does not hold.
fn first_missing(held: &[u32], last: u32) -> Option<u32> {
    let mut sorted = held.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut wanted = 1;
    for number in sorted {
        if number > wanted {
            break;
        }
        wanted = number + 1;
    }
    (wanted <= last).then_some(wanted)
}
