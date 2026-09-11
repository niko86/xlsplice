//! The batch and the report: what a caller asks a package to become, and
//! what came of asking.
//!
//! A batch is a list of operations over one package. It is validated whole
//! before a byte is spliced and applied whole afterwards, so a failure on any
//! operation leaves the package as it was. Nothing here reaches the disk until
//! every operation has been located and every splice computed.
//!
//! One operation exists so far, `set`, and it writes a value into a cell that
//! is already there. A cell the sheet does not hold is
//! [`not_found`](crate::error::ErrorCode::NotFound) until the insertion
//! ticket, and a cell holding a formula is
//! [`refused`](crate::error::ErrorCode::Refused) until the flag that licenses
//! replacing one exists: a mis-addressed write must not quietly destroy a
//! template's formula.
//!
//! A batch that would change nothing writes nothing. The container is rebuilt
//! rather than copied byte for byte (ADR-0001), so rebuilding a package whose
//! parts all still say what they said would change the file without changing
//! what it holds. Leaving it alone is what makes re-running a hydration safe.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};

use crate::atomic;
use crate::cells::{Resolution, parts_of, resolve};
use crate::error::{Error, Result};
use crate::package::Package;
use crate::reference::Address;
use crate::relationships::Relationships;
use crate::splice::{self, Splice};
use crate::workbook::Workbook;
use crate::worksheet::{FormulaRole, Located, Worksheet, Written, formula_of, value_splices};

/// One thing to do to a package.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    /// Write a value into the cell a target names.
    Set {
        /// The cell, by address or by defined name.
        target: String,
        /// What to put in it.
        value: Written,
    },
}

impl Operation {
    /// The target the operation names, for resolving and for reporting.
    pub fn target(&self) -> &str {
        match self {
            Operation::Set { target, .. } => target,
        }
    }
}

/// Everything one invocation asks of one package.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Batch {
    /// The operations, in the order they were given.
    pub operations: Vec<Operation>,
}

impl Batch {
    /// A batch of one operation: what the command line builds.
    pub fn of(operation: Operation) -> Self {
        Batch {
            operations: vec![operation],
        }
    }
}

/// Where the result goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Over the package that was read, through a temporary file and a rename.
    InPlace,
    /// To another path, leaving the input untouched.
    Out(PathBuf),
}

impl From<Option<PathBuf>> for Destination {
    /// What `--out` asks for: a path, or nothing and therefore in place.
    fn from(out: Option<PathBuf>) -> Self {
        match out {
            None => Destination::InPlace,
            Some(path) => Destination::Out(path),
        }
    }
}

impl Destination {
    /// The path written to, given the path read from.
    pub fn path<'a>(&'a self, input: &'a Path) -> &'a Path {
        match self {
            Destination::InPlace => input,
            Destination::Out(path) => path,
        }
    }
}

/// What one operation did.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationReport {
    /// The target exactly as it was given.
    pub target: String,
    /// The defined name it went through, or `None` for an address.
    pub name: Option<String>,
    /// The cell it resolved to, in the package's own spelling.
    pub address: Address,
    /// Whether it changed a byte. A write of the value already there did not.
    pub changed: bool,
}

/// Which parts of the package a batch left different from the ones it read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parts {
    /// The parts whose bytes differ, in path order.
    pub changed: Vec<String>,
    /// The parts the batch created. `set` creates none; the properties and
    /// calculation operations will.
    pub added: Vec<String>,
    /// The parts the batch removed. `set` removes none; replacing the last
    /// chained formula will.
    pub removed: Vec<String>,
}

/// What a whole batch did.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// One result per operation, in the order the operations were given.
    pub operations: Vec<OperationReport>,
    /// What became of the package's parts.
    pub parts: Parts,
    /// Where the result was written, or would have been under a dry run.
    pub output: PathBuf,
    /// Whether this was a dry run, which does everything but put the result
    /// anywhere.
    pub dry_run: bool,
}

/// Apply `batch` to the package at `path`, putting the result at
/// `destination`.
///
/// Everything is validated and spliced before anything is written, and the
/// write itself is one atomic landing of the whole container, so a failure
/// anywhere leaves both the input and the destination as they were.
pub fn run(path: &Path, batch: &Batch, destination: &Destination, dry_run: bool) -> Result<Report> {
    let mut package = Package::open(path)?;
    let workbook = Workbook::read(&mut package)?;
    let rels = Relationships::read(&mut package, workbook.part())?;

    // Every target is resolved before any part is read, so a batch naming a
    // sheet the package does not have fails before a splice is computed.
    let resolved: Vec<ResolvedOperation> = batch
        .operations
        .iter()
        .map(|operation| {
            Ok(ResolvedOperation {
                operation,
                at: resolve(&package, &rels, &workbook, operation.target())?,
            })
        })
        .collect::<Result<_>>()?;

    let mut changed = vec![false; resolved.len()];
    let mut replaced: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for part in parts_of(resolved.iter().map(|resolved| &resolved.at)) {
        let xml = package.read_part_text(&part)?;
        let spliced = splice_part(&xml, &part, &resolved, &mut changed)?;
        if spliced != xml {
            replaced.insert(part, spliced.into_bytes());
        }
    }

    let output = destination.path(path).to_owned();
    if !dry_run {
        land(&mut package, path, destination, &replaced)?;
    }

    Ok(Report {
        operations: resolved
            .into_iter()
            .zip(changed)
            .map(|(resolved, changed)| OperationReport {
                target: resolved.at.target,
                name: resolved.at.name,
                address: resolved.at.address,
                changed,
            })
            .collect(),
        parts: Parts {
            changed: replaced.keys().cloned().collect(),
            ..Parts::default()
        },
        output,
        dry_run,
    })
}

/// One operation with the cell and part it turned out to name. The two travel
/// together from the moment the target is resolved, so nothing has to keep two
/// lists in step.
struct ResolvedOperation<'a> {
    operation: &'a Operation,
    at: Resolution,
}

/// One worksheet part with every operation that lands on it applied.
///
/// The part is parsed once however many operations name cells in it, and
/// every splice is computed against that one tree before any of them is
/// applied, which is what lets the splices be applied from the end backwards.
///
/// `changed` is filled in for the operations that landed on this part, by the
/// index each has in `resolved`.
fn splice_part(
    xml: &str,
    part: &str,
    resolved: &[ResolvedOperation],
    changed: &mut [bool],
) -> Result<String> {
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("not valid XML: {err}")).within(part))?;
    let sheet = Worksheet::of(&document).map_err(|err| err.within(part))?;

    let mut splices: Vec<Splice> = Vec::new();
    for (index, resolved) in resolved.iter().enumerate() {
        let at = &resolved.at;
        if at.part != part {
            continue;
        }
        let node = match sheet
            .locate(at.address.cell)
            .map_err(|err| err.within(part))?
        {
            Located::Cell(node) => node,
            // Inserting the cell, and the row it would sit in, is the next
            // ticket; until then a write has nothing to write into.
            Located::Absent | Located::NoRow => {
                return Err(Error::not_found(format!(
                    "no cell {} to write to: sheet '{}' does not hold it. \
                     Writing a cell that is not there is not yet supported.",
                    at.address, at.address.sheet
                )));
            }
        };
        refuse_a_formula(node, at)?;

        let Operation::Set { value, .. } = &resolved.operation;
        let here = value_splices(node, xml, value)?;
        changed[index] = here.iter().any(|splice| splice.changes(xml));
        splices.extend(here);
    }
    splice::apply(xml, &splices)
}

/// Refuse to write over a formula.
///
/// The spec's guarantee is that a mis-addressed write cannot silently destroy
/// a template's formula, and nothing yet says a caller meant to. A shared
/// master carries the formula its children take theirs from, and overwriting
/// it orphans them, so it names its range: it is refused even once replacing
/// a formula is possible.
fn refuse_a_formula(node: Node, at: &Resolution) -> Result<()> {
    let Some(formula) = formula_of(node, at.address.cell)? else {
        return Ok(());
    };
    if formula.role == FormulaRole::SharedMaster {
        return Err(Error::refused(format!(
            "cell {} carries the formula shared across {}; overwriting it would \
             orphan the rest of the range. Write to a cell outside it.",
            at.address,
            formula.range.as_deref().unwrap_or("its group")
        )));
    }
    // A shared child stores no formula text of its own, so what names it is
    // the group it takes one from.
    let what = match (&formula.role, formula.text.as_str()) {
        (FormulaRole::SharedChild, _) => format!(
            "takes its formula from shared group {}",
            formula
                .group
                .map_or_else(|| "it belongs to".to_owned(), |si| si.to_string())
        ),
        (_, "") => "holds a formula".to_owned(),
        (_, text) => format!("holds the formula '{text}'"),
    };
    Err(Error::refused(format!(
        "cell {} {what}; replacing a formula is not yet supported. Write to a \
         cell that holds a value.",
        at.address
    )))
}

/// Put the result where it was asked for.
///
/// A batch that changed nothing has nothing to rebuild: in place, the package
/// already says what was asked for and is left alone; to another path, its own
/// bytes are copied across, so the result is the input byte for byte rather
/// than a container rebuilt around the same parts.
fn land(
    package: &mut Package,
    path: &Path,
    destination: &Destination,
    replaced: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let bytes = match (replaced.is_empty(), destination) {
        (true, Destination::InPlace) => return Ok(()),
        (true, Destination::Out(_)) => std::fs::read(path).map_err(|err| {
            Error::internal(format!(
                "cannot read {} back to copy it: {err}. Nothing was written.",
                path.display()
            ))
        })?,
        (false, _) => package.rebuild(replaced)?,
    };
    atomic::replace(destination.path(path), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_destination_is_the_input_in_place_and_the_given_path_otherwise() {
        let input = Path::new("book.xlsx");

        assert_eq!(Destination::from(None).path(input), input);
        assert_eq!(
            Destination::from(Some(PathBuf::from("out.xlsx"))).path(input),
            Path::new("out.xlsx")
        );
    }

    #[test]
    fn a_batch_of_one_holds_that_one_operation() {
        let operation = Operation::Set {
            target: "Inputs!A1".to_owned(),
            value: Written::Number(1.0),
        };

        assert_eq!(Batch::of(operation.clone()).operations, [operation]);
    }
}
