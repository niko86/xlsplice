//! The batch and the report: what a caller asks a package to become, and
//! what came of asking.
//!
//! A batch is a list of operations over one package. It is validated whole
//! before a byte is spliced and applied whole afterwards, so a failure on any
//! operation leaves the package as it was. Nothing here reaches the disk until
//! every operation has been located and every splice computed.
//!
//! An operation carries what a caller gave and nothing derived from it: a
//! target, and a value as the text it was written as under the write type it
//! names. Reading that text into the value it becomes is done here, with the
//! workbook open and before any part is read, so a batch is a document a
//! caller can write as readily as the command line can build one.
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
use serde::{Deserialize, Serialize};

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
///
/// An operation is named by what it does, and carries what the caller gave and
/// nothing derived from it: a target, and a value as the text it was spelled
/// with under the [`WriteType`] it says that text is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Write a value into the cell a target names.
    Set {
        /// The cell, by address or by defined name.
        target: String,
        /// What the value is to become in the cell.
        #[serde(rename = "type")]
        write_type: WriteType,
        /// The value, exactly as the caller spelled it.
        value: String,
    },
}

impl Operation {
    /// The target the operation names, for resolving and for reporting.
    pub fn target(&self) -> &str {
        match self {
            Operation::Set { target, .. } => target,
        }
    }

    /// The value this operation writes, read the way its write type says to.
    ///
    /// A failure names the operation by its index in the batch, as well as by
    /// what it asked of which target: a batch may ask twice of one cell, so
    /// the target alone does not say which operation was wrong. The index is
    /// the one the report counts by, so a caller reading a failure and a
    /// caller reading a report are counting the same way.
    fn written(&self, index: usize) -> Result<Written> {
        let Operation::Set {
            target,
            write_type,
            value,
        } = self;
        write_type
            .read(value)
            .map_err(|err| err.within(format!("operation at index {index} (set {target})")))
    }
}

/// What a write says a value is to become in the cell.
///
/// Not a stored type, which is how the cell then spells it: a write type of
/// `text` is stored as an inline string, and one of `number` declares no type
/// at all. `date` joins these once there is a workbook date system to read one
/// against, which is the reason this lives here and not on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteType {
    /// A finite decimal number, stored without a type attribute.
    Number,
    /// Text, stored in the cell as an inline string.
    Text,
    /// `true` or `false`, or `1` or `0`, stored as a boolean cell.
    Bool,
}

impl WriteType {
    /// Every write type, in the order a caller is offered them.
    pub const ALL: [WriteType; 3] = [WriteType::Number, WriteType::Text, WriteType::Bool];

    /// The type as a caller spells it, in a batch and after `--type`.
    pub fn as_str(self) -> &'static str {
        match self {
            WriteType::Number => "number",
            WriteType::Text => "text",
            WriteType::Bool => "bool",
        }
    }

    /// The write type `name` spells, or `None` where it spells none of them.
    pub fn named(name: &str) -> Option<Self> {
        WriteType::ALL
            .into_iter()
            .find(|write_type| write_type.as_str() == name)
    }

    /// One line saying what this type asks for, for a caller choosing between
    /// them.
    pub fn description(self) -> &'static str {
        match self {
            WriteType::Number => "A finite decimal number, stored without a type attribute",
            WriteType::Text => "Text, stored in the cell as an inline string",
            WriteType::Bool => "`true` or `false`, or `1` or `0`, stored as a boolean cell",
        }
    }

    /// Read `text` the way this type says to, or say why it is not of this
    /// type.
    pub fn read(self, text: &str) -> Result<Written> {
        match self {
            WriteType::Number => read_number(text),
            WriteType::Text => Ok(Written::text(text)),
            WriteType::Bool => read_boolean(text),
        }
    }
}

/// Read a number.
///
/// Only a finite number is a number: Excel has no cell that holds an infinity
/// or a not-a-number, so asking for one is a usage error rather than something
/// to store.
fn read_number(text: &str) -> Result<Written> {
    text.parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .map(Written::Number)
        .ok_or_else(|| {
            Error::usage(format!(
                "'{text}' is not a number; write type number takes a finite \
                 decimal number such as 42, -2.5 or 1e6"
            ))
        })
}

/// Read a boolean, in either the spelling the schema uses or the one Excel
/// shows, and in any case.
fn read_boolean(text: &str) -> Result<Written> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(Written::Bool(true)),
        "false" | "0" => Ok(Written::Bool(false)),
        _ => Err(Error::usage(format!(
            "'{text}' is not a boolean; write type bool takes true or false, \
             or 1 or 0"
        ))),
    }
}

/// Everything one invocation asks of one package.
///
/// A batch is one document: a JSON array of operations, which is what the
/// command line builds one of and what `apply` will read. The array is the
/// whole of it, so the batch is transparent to its operations rather than an
/// object wrapping them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
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

    // Every value is read once the workbook model is in hand, because a date
    // is a number only once the workbook's date system has had its say, and
    // before any target is looked up, because a batch whose own text is not of
    // the type it names has nothing worth looking up.
    let written: Vec<Written> = batch
        .operations
        .iter()
        .enumerate()
        .map(|(index, operation)| operation.written(index))
        .collect::<Result<_>>()?;

    // Every target is resolved before any part is read, so a batch naming a
    // sheet the package does not have fails before a splice is computed.
    let resolved: Vec<ResolvedOperation> = batch
        .operations
        .iter()
        .zip(written)
        .map(|(operation, written)| {
            Ok(ResolvedOperation {
                written,
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

/// One operation's value, read, with the cell and part its target turned out
/// to name. The two travel together from the moment the target is resolved, so
/// nothing has to keep two lists in step.
struct ResolvedOperation {
    /// The value, read the way the operation's write type said to.
    written: Written,
    /// The cell and part the operation's target named.
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

        let here = value_splices(node, xml, &resolved.written)?;
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

    use crate::error::ErrorCode;

    #[test]
    fn a_destination_is_the_input_in_place_and_the_given_path_otherwise() {
        let input = Path::new("book.xlsx");

        assert_eq!(Destination::from(None).path(input), input);
        assert_eq!(
            Destination::from(Some(PathBuf::from("out.xlsx"))).path(input),
            Path::new("out.xlsx")
        );
    }

    /// The names, transcribed rather than derived from the code, so a renaming
    /// fails the test. A caller spells a write type one way whatever is
    /// reading it: `--type`, a JSON batch, and whatever a batch is written
    /// back out as.
    const NAMES: [(WriteType, &str); 3] = [
        (WriteType::Number, "number"),
        (WriteType::Text, "text"),
        (WriteType::Bool, "bool"),
    ];

    #[test]
    fn a_write_type_is_spelled_the_same_way_wherever_it_is_read() {
        assert_eq!(WriteType::ALL.len(), NAMES.len(), "a type with no name");
        for (write_type, name) in NAMES {
            assert_eq!(write_type.as_str(), name);
            assert_eq!(WriteType::named(name), Some(write_type));
            assert_eq!(
                serde_json::to_value(write_type).expect("a type serialises"),
                name
            );
        }
        assert_eq!(WriteType::named("date"), None, "date is #9's to add");
    }

    #[test]
    fn a_value_that_is_not_of_the_type_it_names_is_a_usage_error() {
        for text in ["", "hello", "1,5", "inf", "NaN", "2 "] {
            let err = WriteType::Number.read(text).expect_err(text);
            assert_eq!(err.code(), ErrorCode::Usage, "{text}");
        }
        for text in ["TRUE", "False", "1", "0"] {
            WriteType::Bool.read(text).expect(text);
        }
        for text in ["", "yes", "2"] {
            let err = WriteType::Bool.read(text).expect_err(text);
            assert_eq!(err.code(), ErrorCode::Usage, "{text}");
        }
        for text in ["", "hello", "42", "true"] {
            assert_eq!(
                WriteType::Text.read(text).expect(text),
                Written::text(text),
                "any text is text"
            );
        }
    }

    /// A batch may ask twice of one cell, so a failure says which operation it
    /// was, by the index the report counts by, as well as what it asked.
    #[test]
    fn a_value_that_is_not_of_its_type_names_the_operation_it_came_from() {
        let operation = Operation::Set {
            target: "Inputs!A1".to_owned(),
            write_type: WriteType::Number,
            value: "hello".to_owned(),
        };

        let err = operation.written(1).expect_err("hello is not a number");

        assert_eq!(err.code(), ErrorCode::Usage);
        assert!(
            err.message()
                .starts_with("operation at index 1 (set Inputs!A1): 'hello' is not a number;"),
            "{err}"
        );
    }

    /// The document `apply` parses a batch from, spelled out here rather than
    /// derived from the code: a change to any of this is a change to what a
    /// caller writes. An array of operations is the shape #8 asks for; a
    /// value is text under the type that says how to read it, which is what
    /// #20 asks for.
    #[test]
    fn a_batch_round_trips_through_json() {
        let batch = Batch {
            operations: vec![
                Operation::Set {
                    target: "Inputs!A1".to_owned(),
                    write_type: WriteType::Number,
                    value: "42.5".to_owned(),
                },
                Operation::Set {
                    target: "Total".to_owned(),
                    write_type: WriteType::Text,
                    value: " kept ".to_owned(),
                },
            ],
        };

        let json = serde_json::to_value(&batch).expect("a batch serialises");

        assert_eq!(
            json,
            serde_json::json!([
                {"op": "set", "target": "Inputs!A1", "type": "number", "value": "42.5"},
                {"op": "set", "target": "Total", "type": "text", "value": " kept "}
            ])
        );
        assert_eq!(
            serde_json::from_value::<Batch>(json).expect("a batch deserialises"),
            batch
        );
    }

    /// A field an operation does not know is ignored, so a batch written for a
    /// later ticket's flag still reads here.
    #[test]
    fn a_field_an_operation_does_not_know_is_ignored() {
        let json = serde_json::json!([
            {"op": "set", "target": "Inputs!A1", "type": "bool", "value": "1",
             "replace_formula": false}
        ]);

        let batch = serde_json::from_value::<Batch>(json).expect("a batch deserialises");

        assert_eq!(
            batch.operations,
            [Operation::Set {
                target: "Inputs!A1".to_owned(),
                write_type: WriteType::Bool,
                value: "1".to_owned(),
            }]
        );
    }

    #[test]
    fn a_batch_of_one_holds_that_one_operation() {
        let operation = Operation::Set {
            target: "Inputs!A1".to_owned(),
            write_type: WriteType::Number,
            value: "1".to_owned(),
        };

        assert_eq!(Batch::of(operation.clone()).operations, [operation]);
    }
}
