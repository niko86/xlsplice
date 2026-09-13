//! The batch and the report: what a caller asks a package to become, and
//! what came of asking.
//!
//! A batch is a list of operations over one package. It is validated whole
//! before a byte is spliced and applied whole afterwards, so a failure on any
//! operation leaves the package as it was. Nothing here reaches the disk until
//! every operation has answered with the edits it wants. Each is asked in
//! turn, so the failure a caller is told about is the first operation's that
//! has one, whatever kind of failure the ones after it would have had.
//!
//! An operation is handed the package and answers with [`PartEdit`]s, named
//! by part: a splice of a part's bytes, the creation of a part, or its
//! removal. So the batch runs over parts rather than over cells, and an
//! operation that touches a part holding no cell at all, or none the package
//! holds yet, is a kind of operation rather than an impossibility. An
//! operation owns its own reading, and the package memoises the text of a
//! part it has read, so two operations landing on one part read it once
//! between them (ADR-0005).
//!
//! An operation carries what a caller gave and nothing derived from it: a
//! target, and a value as the text it was written as under the write type it
//! names. Reading that text into the value it becomes is done here, with the
//! workbook open, so a batch is a document a caller can write as readily as
//! the command line can build one.
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
use crate::error::{Error, Result};
use crate::package::{Content, Package};
use crate::reference::Address;
use crate::relationships::Relationships;
use crate::splice::{self, Splice};
use crate::target::{Resolution, resolve};
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

    /// The edits this operation wants made, named by the part each lands in,
    /// and the cell it resolved to for the report.
    ///
    /// The operation reads whatever parts it needs out of `opened`, which
    /// memoises them, and answers without changing anything: a batch is
    /// applied only once every operation has answered. `index` is the place
    /// the operation has in the batch, which is what a failure names it by.
    pub fn edits(&self, index: usize, opened: &mut Opened) -> Result<Asked> {
        match self {
            Operation::Set { target, .. } => {
                let written = self.written(index)?;
                let at = opened.resolve(target)?;
                let xml = opened.text(&at.part)?;
                let splices = splices_of(xml, &at, &written)?;
                Ok(Asked {
                    parts: vec![(at.part.clone(), PartEdit::Splice(splices))],
                    at: Some(at),
                })
            }
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

/// One thing to be done to one part.
///
/// A splice is one species of part edit rather than the whole of it: a part
/// may also be created or removed, and an operation that does either names a
/// part the way an operation that splices one does. Which parts a package
/// holds is the container's business, so a created part carries bytes alone
/// and nothing about where its entry sits or what stamp it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartEdit {
    /// Replace the bytes of named ranges of the part, and leave every other
    /// byte of it alone.
    Splice(Vec<Splice>),
    /// Put a part the package does not hold into it, holding these bytes.
    Create(Vec<u8>),
    /// Leave the part out of the package.
    Remove,
}

impl PartEdit {
    /// The two edits as the one edit they come to, or `None` where they
    /// disagree about what is being done to the part.
    ///
    /// Splices merge, because they are ranges of one text and
    /// [`splice::apply`] is what says whether two of them collide; removals
    /// merge, because two operations may both be done with a part; two
    /// creations merge only if they carry the same bytes. A part is spliced,
    /// created or removed, not two of the three.
    fn and(self, other: &PartEdit) -> Option<PartEdit> {
        match (self, other) {
            (PartEdit::Splice(mut mine), PartEdit::Splice(theirs)) => {
                mine.extend(theirs.iter().cloned());
                Some(PartEdit::Splice(mine))
            }
            (PartEdit::Create(mine), PartEdit::Create(theirs)) if &mine == theirs => {
                Some(PartEdit::Create(mine))
            }
            (PartEdit::Remove, PartEdit::Remove) => Some(PartEdit::Remove),
            _ => None,
        }
    }

    /// The splices this edit is, or none at all where it is another kind of
    /// edit. A part created or removed is not spliced, so it has none.
    fn splices(&self) -> &[Splice] {
        match self {
            PartEdit::Splice(splices) => splices,
            PartEdit::Create(_) | PartEdit::Remove => &[],
        }
    }
}

/// What one operation asked of the package: its part edits, and where it
/// landed.
///
/// The edits are named by part, and there may be none: an operation that
/// finds the package already saying what it was asked for asks for nothing.
/// A cell operation also answers with the cell it resolved to, for the
/// report; an operation naming no cell answers with none, and the batch
/// resolves nothing on its behalf.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    /// The cell the operation named, or `None` where it named none.
    pub at: Option<Resolution>,
    /// What each part is to have done to it, named by part path.
    pub parts: Vec<(String, PartEdit)>,
}

/// A package open for writing, with the models an operation reads it through.
///
/// This is what an operation is handed, and it is the whole of what an
/// operation may do to a package: resolve a target, ask whether a part is
/// there, and read one. Rebuilding the container is no part of it, which is
/// why the batch alone reaches that, through [`Opened::land`].
///
/// The workbook and its relationships are read once, when the package is
/// opened, because every target goes through them; every other part is read
/// by the operation that wants it, and memoised by the package, so the second
/// operation to want a part pays nothing for it (ADR-0005).
pub struct Opened {
    package: Package,
    workbook: Workbook,
    rels: Relationships,
}

impl Opened {
    /// Open the package at `path` and read the two parts every target is
    /// resolved through.
    pub fn open(path: &Path) -> Result<Self> {
        let mut package = Package::open(path)?;
        let workbook = Workbook::read(&mut package)?;
        let rels = Relationships::read(&mut package, workbook.part())?;
        Ok(Opened {
            package,
            workbook,
            rels,
        })
    }

    /// The cell and part a target names.
    pub fn resolve(&self, target: &str) -> Result<Resolution> {
        resolve(&self.package, &self.rels, &self.workbook, target)
    }

    /// The text of one part, read once however many operations ask for it.
    pub fn text(&mut self, part: &str) -> Result<&str> {
        self.package.read_part_text(part)
    }

    /// Whether the package holds a part at `part`.
    pub fn has_part(&self, part: &str) -> bool {
        self.package.has_part(part)
    }

    /// How many parts have been taken out of the container so far.
    pub fn reads(&self) -> usize {
        self.package.reads()
    }

    /// Put the result where it was asked for.
    ///
    /// A batch that changed nothing has nothing to rebuild: in place, the
    /// package already says what was asked for and is left alone; to another
    /// path, its own bytes are copied across, so the result is the input byte
    /// for byte rather than a container rebuilt around the same parts
    /// (ADR-0003).
    fn land(
        &mut self,
        path: &Path,
        destination: &Destination,
        content: &BTreeMap<String, Content>,
    ) -> Result<()> {
        let bytes = match (content.is_empty(), destination) {
            (true, Destination::InPlace) => return Ok(()),
            (true, Destination::Out(_)) => std::fs::read(path).map_err(|err| {
                Error::internal(format!(
                    "cannot read {} back to copy it: {err}. Nothing was written.",
                    path.display()
                ))
            })?,
            (false, _) => self.package.rebuild(content)?,
        };
        atomic::replace(destination.path(path), &bytes)
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
    /// The cell it resolved to, in the package's own spelling, or `None` for
    /// an operation that names no cell.
    pub address: Option<Address>,
    /// Whether it changed a byte. A write of the value already there did not.
    pub changed: bool,
}

/// Which parts of the package a batch left different from the ones it read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parts {
    /// The parts whose bytes differ, in path order.
    pub changed: Vec<String>,
    /// The parts the batch created, in path order. `set` creates none; the
    /// custom properties operation will.
    pub added: Vec<String>,
    /// The parts the batch removed, in path order. `set` removes none;
    /// replacing the last chained formula will.
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
/// Every operation answers with its edits before any of them is applied, and
/// the write itself is one atomic landing of the whole container, so a failure
/// anywhere leaves both the input and the destination as they were.
pub fn run(path: &Path, batch: &Batch, destination: &Destination, dry_run: bool) -> Result<Report> {
    let mut opened = Opened::open(path)?;

    // Every operation is asked what it wants before anything is done to a
    // part, so a batch that fails on its last operation has changed nothing
    // for its first.
    let asked: Vec<Asked> = batch
        .operations
        .iter()
        .enumerate()
        .map(|(index, operation)| operation.edits(index, &mut opened))
        .collect::<Result<_>>()?;

    let applied = apply(&mut opened, &asked)?;

    let output = destination.path(path).to_owned();
    if !dry_run {
        opened.land(path, destination, &applied.content)?;
    }

    Ok(Report {
        operations: reports(&batch.operations, &asked, &applied.changed),
        parts: applied.parts,
        output,
        dry_run,
    })
}

/// One result per operation, in the order the operations were given.
///
/// What an operation resolved to is what it answered with rather than
/// something looked up here: an operation that named no cell reports none,
/// and nothing resolves one on its behalf.
fn reports(operations: &[Operation], asked: &[Asked], changed: &[bool]) -> Vec<OperationReport> {
    operations
        .iter()
        .zip(asked)
        .zip(changed)
        .map(|((operation, edits), changed)| OperationReport {
            target: operation.target().to_owned(),
            name: edits.at.as_ref().and_then(|at| at.name.clone()),
            address: edits.at.as_ref().map(|at| at.address.clone()),
            changed: *changed,
        })
        .collect()
}

/// What a batch's edits came to, before anything is written.
struct Applied {
    /// What each part the batch touched is to become.
    content: BTreeMap<String, Content>,
    /// Whether each operation changed a byte, by its place in the batch.
    changed: Vec<bool>,
    /// What to report about the parts.
    parts: Parts,
}

/// Work every operation's edits out into what each part is to become.
///
/// A part is dealt with once, however many operations landed on it: it is
/// read once, spliced once with all of their splices, and written once. The
/// parts are taken in path order, which is the order the report lists them
/// in.
fn apply(opened: &mut Opened, asked: &[Asked]) -> Result<Applied> {
    let mut applied = Applied {
        content: BTreeMap::new(),
        changed: vec![false; asked.len()],
        parts: Parts::default(),
    };
    for (part, edits) in by_part(asked) {
        match merged(&part, &edits)? {
            PartEdit::Splice(all) => {
                let xml = opened.text(&part)?;
                spliced(&mut applied.changed, &edits, xml);
                let spliced = splice::apply(xml, &all)?;
                if spliced != xml {
                    applied.parts.changed.push(part.clone());
                    applied
                        .content
                        .insert(part, Content::Bytes(spliced.into_bytes()));
                }
            }
            PartEdit::Create(bytes) => {
                if opened.has_part(&part) {
                    return Err(Error::internal(format!(
                        "part '{part}' is already in the package, so it cannot be created"
                    )));
                }
                mark(&mut applied.changed, &edits);
                applied.parts.added.push(part.clone());
                applied.content.insert(part, Content::Bytes(bytes));
            }
            PartEdit::Remove => {
                if !opened.has_part(&part) {
                    return Err(Error::internal(format!(
                        "part '{part}' is not in the package, so it cannot be removed"
                    )));
                }
                mark(&mut applied.changed, &edits);
                applied.parts.removed.push(part.clone());
                applied.content.insert(part, Content::Gone);
            }
        }
    }
    Ok(applied)
}

/// Every operation's edits, gathered by the part they land in, in path order.
fn by_part(asked: &[Asked]) -> BTreeMap<String, Vec<(usize, &PartEdit)>> {
    let mut by_part: BTreeMap<String, Vec<(usize, &PartEdit)>> = BTreeMap::new();
    for (index, edits) in asked.iter().enumerate() {
        for (part, edit) in &edits.parts {
            by_part.entry(part.clone()).or_default().push((index, edit));
        }
    }
    by_part
}

/// What the edits on one part come to together.
///
/// The edits are folded into one, and two that disagree about what is being
/// done to the part are two operations asking different things of it: a fault
/// in whatever built the batch rather than something the package can be wrong
/// about, so the failure names both of them.
fn merged(part: &str, edits: &[(usize, &PartEdit)]) -> Result<PartEdit> {
    let Some(((first, edit), rest)) = edits.split_first() else {
        return Err(Error::internal(format!(
            "part '{part}' is named by no edit"
        )));
    };
    let mut merged = (*edit).clone();
    for (other, edit) in rest {
        merged = merged.and(edit).ok_or_else(|| {
            Error::internal(format!(
                "the operations at index {first} and {other} ask different things of part \
                 '{part}': a part is spliced, created or removed, not two of the three"
            ))
        })?;
    }
    Ok(merged)
}

/// Say which operations this part's splices changed a byte for.
///
/// An operation may edit several parts, and the parts are taken one at a
/// time, so what this part says about an operation is added to what its other
/// parts said rather than put in place of it: an operation changed something
/// if any one of its edits did.
fn spliced(changed: &mut [bool], edits: &[(usize, &PartEdit)], xml: &str) {
    for (index, edit) in edits {
        changed[*index] |= edit.splices().iter().any(|splice| splice.changes(xml));
    }
}

/// Say that every operation that asked for this part changed something. A
/// part created or removed is a change by every operation that asked for it,
/// because the part was not there, or was, before any of them asked.
fn mark(changed: &mut [bool], edits: &[(usize, &PartEdit)]) {
    for (index, _) in edits {
        changed[*index] = true;
    }
}

/// The splices that put `written` into the cell `at` names, in the part whose
/// text is `xml`.
///
/// The part is parsed here, by the operation that named it, and the splices
/// are byte ranges of the text handed in: the same text every other operation
/// on this part is handed, so their ranges all mean the same thing.
fn splices_of(xml: &str, at: &Resolution, written: &Written) -> Result<Vec<Splice>> {
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("not valid XML: {err}")).within(&at.part))?;
    let sheet = Worksheet::of(&document).map_err(|err| err.within(&at.part))?;
    let node = match sheet
        .locate(at.address.cell)
        .map_err(|err| err.within(&at.part))?
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
    value_splices(node, xml, written)
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

    fn splice() -> Splice {
        Splice::new(0..1, "x")
    }

    /// Two operations landing on one part are one splice list against one
    /// text; whether two of those ranges collide is `splice::apply`'s to say,
    /// not this.
    #[test]
    fn the_splices_of_every_operation_on_one_part_are_merged_into_one_list() {
        let one = PartEdit::Splice(vec![splice()]);
        let two = PartEdit::Splice(vec![Splice::new(4..5, "y"), Splice::new(9..9, "z")]);

        let merged = merged("sheet1.xml", &[(0, &one), (1, &two)]).expect("splices merge");

        assert_eq!(
            merged,
            PartEdit::Splice(vec![
                splice(),
                Splice::new(4..5, "y"),
                Splice::new(9..9, "z")
            ])
        );
    }

    /// Two operations may both be done with one part, and a part is removed
    /// once however many of them said so.
    #[test]
    fn two_operations_removing_one_part_remove_it_once() {
        let merged = merged(
            "xl/calcChain.xml",
            &[(0, &PartEdit::Remove), (1, &PartEdit::Remove)],
        )
        .expect("two removals are one removal");

        assert_eq!(merged, PartEdit::Remove);
    }

    /// Two operations creating one part agree only if they agree about what
    /// is in it.
    #[test]
    fn two_operations_creating_one_part_must_carry_the_same_bytes() {
        let one = PartEdit::Create(b"<properties/>".to_vec());
        let same = PartEdit::Create(b"<properties/>".to_vec());
        let other = PartEdit::Create(b"<properties count=\"1\"/>".to_vec());

        assert_eq!(
            merged("docProps/custom.xml", &[(0, &one), (1, &same)]).expect("the same bytes"),
            one
        );
        let err = merged("docProps/custom.xml", &[(0, &one), (1, &other)])
            .expect_err("different bytes are two answers to one question");
        assert_eq!(err.code(), ErrorCode::Internal);
    }

    /// A part is spliced, created or removed, not two of the three. Whatever
    /// built such a batch is at fault, so the failure names both operations.
    #[test]
    fn edits_of_different_kinds_on_one_part_are_refused_naming_both_operations() {
        let splicing = PartEdit::Splice(vec![splice()]);

        let err = merged("sheet1.xml", &[(2, &splicing), (5, &PartEdit::Remove)])
            .expect_err("a part is not spliced and removed at once");

        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(err.message().contains("index 2 and 5"), "{}", err.message());
        assert!(err.message().contains("sheet1.xml"), "{}", err.message());
    }

    /// An operation may edit several parts, and the parts are taken one at a
    /// time. What one of them says about an operation is added to what the
    /// others said, so a part an operation changed nothing in cannot take
    /// back a part it did change. No operation edits two parts yet; #10 and
    /// #12 are the first that will.
    #[test]
    fn an_operation_that_changed_any_of_its_parts_changed_something() {
        let changes = PartEdit::Splice(vec![Splice::new(0..1, "y")]);
        let does_not = PartEdit::Splice(vec![Splice::new(0..1, "x")]);

        let mut changed = [false];
        spliced(&mut changed, &[(0, &changes)], "xxx");
        assert!(changed[0], "the splice put a byte there that was not");
        spliced(&mut changed, &[(0, &does_not)], "xxx");
        assert!(
            changed[0],
            "a later part it changed nothing in does not take that back"
        );

        let mut removed = [false];
        mark(&mut removed, &[(0, &PartEdit::Remove)]);
        spliced(&mut removed, &[(0, &does_not)], "xxx");
        assert!(removed[0], "and neither does one after a part removed");
    }

    /// An operation that names no cell reports none, and the batch resolves
    /// nothing on its behalf. Nothing builds such an operation yet: #10, #11
    /// and #12 do, and this is the shape their reports take.
    #[test]
    fn an_operation_that_names_no_cell_is_reported_with_no_address() {
        let operations = [
            Operation::Set {
                target: "Inputs!A1".to_owned(),
                write_type: WriteType::Number,
                value: "1".to_owned(),
            },
            Operation::Set {
                target: "the flag".to_owned(),
                write_type: WriteType::Bool,
                value: "true".to_owned(),
            },
        ];
        let asked = [
            Asked {
                at: Some(Resolution {
                    target: "Inputs!A1".to_owned(),
                    name: None,
                    address: Address {
                        sheet: "Inputs".to_owned(),
                        cell: crate::reference::Cell::parse("A1").expect("a cell"),
                    },
                    part: "xl/worksheets/sheet1.xml".to_owned(),
                }),
                parts: vec![(
                    "xl/worksheets/sheet1.xml".to_owned(),
                    PartEdit::Splice(vec![splice()]),
                )],
            },
            Asked {
                at: None,
                parts: vec![("xl/workbook.xml".to_owned(), PartEdit::Splice(Vec::new()))],
            },
        ];

        let reports = reports(&operations, &asked, &[true, false]);

        assert_eq!(
            reports[0].address.as_ref().map(ToString::to_string),
            Some("Inputs!A1".to_owned())
        );
        assert_eq!(reports[1].address, None, "no cell, no address");
        assert_eq!(
            reports[1].target, "the flag",
            "the target is still reported"
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
