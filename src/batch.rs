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
//! Two operations write a cell, `set` and `clear`. A cell the sheet does not
//! hold is put there by a `set`, and the row it sits in with it, because a
//! template carries an element only for the cells something is already in; a
//! `clear` puts nothing in, because an absent cell already holds nothing. A
//! cell holding a formula is [`refused`](crate::error::ErrorCode::Refused)
//! unless the caller says it may be replaced, so a mis-addressed write cannot
//! quietly destroy a template's formula. The rest write no cell at all:
//! `calc` sets a flag on the workbook, and `props.set` and `props.unset`
//! write the properties a package carries about itself.
//!
//! A few things a batch does have no per-operation answer, because one
//! snapshot is what every operation sees: whether an emptied calc chain still
//! belongs in the package, and how many properties are being added and which
//! identifiers they take. Those are settled once, for the part, after the
//! operations' edits are merged and before anything is written, and so is how
//! many cells a row being put into a sheet holds. Each is a settlement over
//! [`Wanted`](crate::wanted::Wanted), which is what the batch wants written
//! and who asked for it; the three there are are listed in [`settlements`],
//! and what the whole of it comes to is worked out there too.
//!
//! A batch that would change nothing writes nothing. The container is rebuilt
//! rather than copied byte for byte (ADR-0001), so rebuilding a package whose
//! parts all still say what they said would change the file without changing
//! what it holds. Leaving it alone is what makes re-running a hydration safe.

use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::calc_chain;
use crate::calculation;
use crate::date;
use crate::declared;
use crate::error::{Error, Result};
use crate::package::{Content, FromBytes, FromFile, Package};
use crate::properties;
use crate::reference::Address;
use crate::relationships::{PACKAGE_ROOT, Relationships, part_or_conventional};
use crate::splice::{self, Splice};
use crate::target::{Resolution, resolve};
use crate::wanted::{Applied, Settlement, Wanted};
use crate::workbook::{DateSystem, Workbook};
use crate::worksheet::{
    self, FormulaRole, Located, NewCell, Worksheet, Written, formula_of, value_splices,
};

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
        /// Whether the caller says a formula in the cell may be replaced.
        /// Without it a cell holding one is refused, so that a mis-addressed
        /// write cannot silently destroy it; a shared formula's master is
        /// refused even with it, because overwriting one orphans its
        /// children.
        #[serde(default)]
        replace_formula: bool,
    },
    /// Empty the cell a target names, keeping the element and its style.
    Clear {
        /// The cell, by address or by defined name.
        target: String,
        /// Whether the caller says a formula in the cell may be replaced.
        #[serde(default)]
        replace_formula: bool,
    },
    /// Say whether the workbook recalculates fully when it is opened. The
    /// first operation that names no cell.
    Calc {
        /// What the flag is to say. Setting it to what it already says
        /// changes nothing and writes nothing.
        full_calc_on_load: bool,
    },
    /// Give a custom document property a value, adding it where the package
    /// has none of that name.
    #[serde(rename = "props.set")]
    PropsSet {
        /// The property, by the name the package spells it with. Matched
        /// exactly: two names differing in case are two properties.
        name: String,
        /// What the value is to become.
        #[serde(rename = "type")]
        write_type: WriteType,
        /// The value, exactly as the caller spelled it.
        value: String,
    },
    /// Take a custom document property out.
    #[serde(rename = "props.unset")]
    PropsUnset {
        /// The property, by the name the package spells it with.
        name: String,
    },
}

impl Operation {
    /// Every operation kind, spelled as a batch spells it, in the order a
    /// caller is offered them.
    ///
    /// A batch naming a kind that is not here is refused with this list, so
    /// a caller writing one against a later version and running it against
    /// this one is told what this one knows.
    pub const KINDS: [&'static str; 5] = ["set", "clear", "calc", "props.set", "props.unset"];

    /// The kind this operation is, as a batch spells it.
    pub fn kind(&self) -> &'static str {
        match self {
            Operation::Set { .. } => "set",
            Operation::Clear { .. } => "clear",
            Operation::Calc { .. } => "calc",
            Operation::PropsSet { .. } => "props.set",
            Operation::PropsUnset { .. } => "props.unset",
        }
    }

    /// Whether the operation says a formula in the cell it names may be
    /// replaced.
    fn replaces_a_formula(&self) -> bool {
        match self {
            Operation::Set {
                replace_formula, ..
            }
            | Operation::Clear {
                replace_formula, ..
            } => *replace_formula,
            Operation::Calc { .. } | Operation::PropsSet { .. } | Operation::PropsUnset { .. } => {
                false
            }
        }
    }

    /// What the operation is pointed at, or `None` where it is pointed at
    /// nothing in particular.
    ///
    /// A cell for one that writes a cell, and a property for one that writes
    /// a property; the calculation flag is the workbook's, so it is pointed
    /// at nothing. This is what the report puts in its target column and what
    /// a failure names the operation by, so it is what the caller wrote
    /// rather than anything resolved from it.
    pub fn target(&self) -> Option<&str> {
        match self {
            Operation::Set { target, .. } | Operation::Clear { target, .. } => Some(target),
            Operation::PropsSet { name, .. } | Operation::PropsUnset { name } => Some(name),
            Operation::Calc { .. } => None,
        }
    }

    /// How a failure of this operation names it: its place in the batch, and
    /// what it asked of which target.
    ///
    /// A batch may ask of one cell in more than one way, and two operations
    /// may fail for the same reason, so neither the target nor the reason
    /// alone says which operation was wrong. The index is the one the report
    /// counts by, so a caller reading a failure and a caller reading a report
    /// are counting the same way.
    fn within(&self, index: usize) -> String {
        match self.target() {
            Some(target) => format!("operation at index {index} ({} {target})", self.kind()),
            None => format!("operation at index {index} ({})", self.kind()),
        }
    }

    /// The cell this operation names, resolved, or `None` where it names no
    /// cell.
    ///
    /// Answering reads nothing: a target is resolved against the workbook
    /// model and the relationships, which are in hand the moment the package
    /// is opened. So the batch can hold its operations up against each other
    /// before a single part comes out of the container, which is what lets
    /// two operations on one cell be refused rather than spliced.
    pub fn at<R: Read + Seek>(&self, opened: &Opened<R>) -> Result<Option<Resolution>> {
        match self {
            Operation::Set { target, .. } | Operation::Clear { target, .. } => {
                opened.resolve(target).map(Some)
            }
            Operation::Calc { .. } | Operation::PropsSet { .. } | Operation::PropsUnset { .. } => {
                Ok(None)
            }
        }
    }

    /// The edits this operation wants made, named by the part each lands in.
    ///
    /// `at` is what the operation answered to [`Operation::at`], handed back
    /// so that the cell the batch checked is the cell the operation acts on.
    /// The operation reads whatever parts it needs out of `opened`, which
    /// memoises them, and answers without changing anything: a batch is
    /// applied only once every operation has answered.
    pub fn edits<R: Read + Seek>(
        &self,
        at: Option<Resolution>,
        opened: &mut Opened<R>,
    ) -> Result<Asked> {
        match self {
            Operation::Calc { full_calc_on_load } => calculated(*full_calc_on_load, opened),
            Operation::Set { .. } | Operation::Clear { .. } => self.written_into(at, opened),
            Operation::PropsSet { name, .. } => {
                // The value is read here, so that a value that is not one
                // fails against the operation that gave it even though it is
                // the batch that adds a property the part does not hold.
                let value = self.property_value()?;
                property_written(name, &value, opened)
            }
            Operation::PropsUnset { name } => property_withdrawn(name, opened),
        }
    }

    /// What this operation puts on the property it names, read the way it
    /// says to.
    ///
    /// A property holds a moment rather than a serial, so a date here is not
    /// the number a cell would hold and the workbook's date system has no say
    /// in it.
    fn property_value(&self) -> Result<properties::Value> {
        let Operation::PropsSet {
            write_type, value, ..
        } = self
        else {
            return Err(Error::internal(
                "only props.set has a property value to read".to_owned(),
            ));
        };
        match write_type {
            WriteType::Text => Ok(properties::Value::Text(value.clone())),
            WriteType::Number => match read_number(value)? {
                Written::Number(number) => Ok(properties::Value::number(number)),
                other => Err(Error::internal(format!("a number was read as {other:?}"))),
            },
            WriteType::Bool => match read_boolean(value)? {
                Written::Bool(yes) => Ok(properties::Value::Bool(yes)),
                other => Err(Error::internal(format!("a boolean was read as {other:?}"))),
            },
            WriteType::Date => date::utc(value).map(properties::Value::Moment),
        }
    }

    /// The edits that put this operation's value into the cell `at`, and take
    /// a formula it replaced out of the calc chain.
    fn written_into<R: Read + Seek>(
        &self,
        at: Option<Resolution>,
        opened: &mut Opened<R>,
    ) -> Result<Asked> {
        match at {
            Some(at) => {
                let written = self.written(opened.dates())?;
                // The borrow of the part's text ends here, because taking a
                // formula's entry out of the calc chain reads another part.
                let wrote = {
                    let xml = opened.text(&at.part)?;
                    splices_of(xml, &at, &written, self.replaces_a_formula())?
                };
                let mut parts = vec![(at.part.clone(), PartEdit::Splice(wrote.splices))];
                if wrote.replaced {
                    parts.extend(uncalculated(opened, &at)?);
                }
                Ok(Asked {
                    parts,
                    at: Some(at),
                    into_a_new_row: wrote.into_a_new_row,
                })
            }
            None => Err(Error::internal(format!(
                "a {} names a cell, and this one was handed none",
                self.kind()
            ))),
        }
    }

    /// What this operation puts in the cell, read the way it says to.
    ///
    /// `dates` is the workbook's, because a date is a number only once the
    /// workbook's date system has had its say, and clearing a cell puts
    /// nothing in it.
    fn written(&self, dates: DateSystem) -> Result<Written> {
        match self {
            Operation::Set {
                write_type, value, ..
            } => write_type.read(value, dates),
            Operation::Clear { .. } => Ok(Written::Nothing),
            // Only a cell-writing operation asks what it writes into a cell.
            other => Err(Error::internal(format!(
                "a {} was asked what it writes into a cell, and it writes into none",
                other.kind()
            ))),
        }
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
    /// An ISO 8601 date or datetime, or a serial, stored as the serial the
    /// workbook's date system gives it.
    Date,
}

impl WriteType {
    /// Every write type, in the order a caller is offered them.
    pub const ALL: [WriteType; 4] = [
        WriteType::Number,
        WriteType::Text,
        WriteType::Bool,
        WriteType::Date,
    ];

    /// The type as a caller spells it, in a batch and after `--type`.
    pub fn as_str(self) -> &'static str {
        match self {
            WriteType::Number => "number",
            WriteType::Text => "text",
            WriteType::Bool => "bool",
            WriteType::Date => "date",
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
            WriteType::Date => {
                "An ISO 8601 date or datetime, or a serial, stored as a serial in the \
                 workbook's date system"
            }
        }
    }

    /// Read `text` the way this type says to, or say why it is not of this
    /// type.
    ///
    /// `dates` is the workbook's date system, which only a date needs: which
    /// number a date is depends on the workbook, and nothing else here does.
    pub fn read(self, text: &str, dates: DateSystem) -> Result<Written> {
        match self {
            WriteType::Number => read_number(text),
            WriteType::Text => Ok(Written::text(text)),
            WriteType::Bool => read_boolean(text),
            WriteType::Date => date::serial(text, dates).map(Written::Number),
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
    pub(crate) fn and(&self, other: &PartEdit) -> Option<PartEdit> {
        match (self, other) {
            (PartEdit::Splice(mine), PartEdit::Splice(theirs)) => {
                let mut both = mine.clone();
                both.extend(theirs.iter().cloned());
                Some(PartEdit::Splice(both))
            }
            (PartEdit::Create(mine), PartEdit::Create(theirs)) if mine == theirs => {
                Some(PartEdit::Create(mine.clone()))
            }
            (PartEdit::Remove, PartEdit::Remove) => Some(PartEdit::Remove),
            _ => None,
        }
    }

    /// The splices this edit is, or none at all where it is another kind of
    /// edit. A part created or removed is not spliced, so it has none.
    pub(crate) fn splices(&self) -> &[Splice] {
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
    /// The cell this operation puts into a row the sheet does not hold, where
    /// it does that; the part is the one [`Asked::at`] names.
    ///
    /// The operation answers with the cell rather than with a splice, because
    /// how many cells the new row holds is a question about the part: two
    /// operations writing cells of one absent row cannot see each other over
    /// one snapshot, so the batch puts the row in once, holding both.
    pub into_a_new_row: Option<NewCell>,
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
pub struct Opened<R> {
    package: Package<R>,
    workbook: Workbook,
    rels: Relationships,
}

impl Opened<FromFile> {
    /// Open the package at `path` and read the two parts every target is
    /// resolved through.
    pub fn open(path: &Path) -> Result<Self> {
        Opened::over(Package::open(path)?)
    }
}

impl Opened<FromBytes> {
    /// Open a package whose bytes are already in hand.
    ///
    /// The operations see the same package they would have seen had those
    /// bytes been a file: what an open package is over is no part of what an
    /// operation may do to it.
    pub fn of(bytes: Vec<u8>) -> Result<Self> {
        Opened::over(Package::of(bytes)?)
    }
}

impl<R: Read + Seek> Opened<R> {
    /// Read the two parts every target is resolved through, whatever the
    /// package is over.
    fn over(mut package: Package<R>) -> Result<Self> {
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

    /// Which day the workbook counts its date serials from.
    pub fn dates(&self) -> DateSystem {
        self.workbook.dates()
    }

    /// The calc chain part, where the package holds one. The relationship
    /// names it, falling back to where Excel puts it, as every other part
    /// xlsplice follows by type does.
    pub fn calc_chain(&self) -> Option<String> {
        part_or_conventional(
            &self.package,
            self.rels.part_of_kind(calc_chain::CALC_CHAIN),
            calc_chain::CONVENTIONAL_PART,
        )
    }

    /// The workbook part, which owns the relationships every part under
    /// `xl/` is reached by.
    pub fn workbook_part(&self) -> &str {
        self.workbook.part()
    }

    /// Where the package's custom document properties are, and whether it
    /// holds them at all.
    ///
    /// A package with none still answers with a path: it is where the part
    /// would go, which is what a batch adding a property needs to know.
    pub fn properties(&mut self) -> Result<(String, bool)> {
        properties::part_of(&mut self.package)
    }

    /// The package itself, for the declarations a part removed has to be
    /// taken out of. Nothing else reaches past this into the container.
    fn package(&mut self) -> &mut Package<R> {
        &mut self.package
    }

    /// The number the workbook gives the sheet called `sheet`, which is what
    /// the calc chain calls it.
    pub fn sheet_number(&self, sheet: &str) -> Option<u32> {
        self.workbook
            .sheet_named(sheet)
            .and_then(|found| found.sheet_id)
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

    /// The batch `json` spells, or why it is not one.
    ///
    /// A document that is not a batch is the caller's mistake rather than the
    /// package's, so it is a usage error. Where the document is JSON but not a
    /// batch, the message carries the operation kinds this build knows: the
    /// likeliest such mistake is a batch written for a kind a later ticket
    /// adds.
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|err| {
            let known = match err.classify() {
                serde_json::error::Category::Data => {
                    format!(" The operation kinds are: {}.", Operation::KINDS.join(", "))
                }
                _ => String::new(),
            };
            Error::usage(format!("cannot read the batch: {err}.{known}"))
        })
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
    /// The target exactly as it was given, or `None` for an operation that
    /// names none.
    pub target: Option<String>,
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

    // Where every operation lands is settled before any part is read, both
    // because the operations are held up against each other there and because
    // a batch naming a sheet the package does not have is wrong whatever is
    // in its parts.
    let land_at: Vec<Option<Resolution>> = batch
        .operations
        .iter()
        .enumerate()
        .map(|(index, operation)| {
            operation
                .at(&opened)
                .map_err(|err| err.within(operation.within(index)))
        })
        .collect::<Result<_>>()?;
    refuse_a_repeated_cell(&land_at)?;
    refuse_a_repeated_property(&batch.operations)?;

    // Every operation is then asked what it wants before anything is done to
    // a part, so a batch that fails on its last operation has changed nothing
    // for its first.
    let asked: Vec<Asked> = batch
        .operations
        .iter()
        .zip(land_at)
        .enumerate()
        .map(|(index, (operation, at))| {
            operation
                .edits(at, &mut opened)
                .map_err(|err| err.within(operation.within(index)))
        })
        .collect::<Result<_>>()?;

    let applied = apply(&mut opened, &batch.operations, &asked)?;

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

/// Refuse a batch that names one cell twice.
///
/// A batch is a set of edits over the package as it was read, not a sequence
/// over a document that changes under it: every operation computes its edits
/// against the same text, and ADR-0002's one-parse rule is what makes that
/// so. Two operations on one cell are therefore not a first write and then a
/// second, but two answers to one question, and xlsplice cannot tell a
/// deliberate overwrite from a mis-addressed one because a `set` asserts
/// nothing about what it expects to find. So the batch is refused whole,
/// before a part is read, and the message names both operations and the cell
/// they agree on (ADR-0004).
///
/// The two are compared on what they resolved to rather than on what they
/// said, so a batch naming one cell by its address and again by a defined
/// name that anchors there is the same contradiction and is refused the same
/// way.
fn refuse_a_repeated_cell(land_at: &[Option<Resolution>]) -> Result<()> {
    for (later, one) in land_at.iter().enumerate() {
        let Some(one) = one else { continue };
        for (earlier, other) in land_at[..later].iter().enumerate() {
            let Some(other) = other else { continue };
            if other.part == one.part && other.address == one.address {
                return Err(Error::usage(format!(
                    "the operations at index {earlier} and {later} both name cell {}; \
                     a batch is applied to the package as it was read, so one cell \
                     cannot be asked two things at once. Give one operation per cell.",
                    one.address
                )));
            }
        }
    }
    Ok(())
}

/// Refuse a batch that names one custom document property twice.
///
/// The rule is the cell rule and the reason is the same one: a batch is a set
/// of edits over the package as it was read, so two operations on one
/// property are not a first write and then a second but two answers to one
/// question (ADR-0004). Setting a property and then unsetting it in one batch
/// is the same contradiction as setting it twice, so both are refused
/// together, on the name rather than on what either meant to do with it.
fn refuse_a_repeated_property(operations: &[Operation]) -> Result<()> {
    let named: Vec<Option<&str>> = operations
        .iter()
        .map(|operation| match operation {
            Operation::PropsSet { name, .. } | Operation::PropsUnset { name } => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect();
    for (later, one) in named.iter().enumerate() {
        let Some(one) = one else { continue };
        for (earlier, other) in named[..later].iter().enumerate() {
            if other == &Some(*one) {
                return Err(Error::usage(format!(
                    "the operations at index {earlier} and {later} both name the custom \
                     document property '{one}'; a batch is applied to the package as it \
                     was read, so one property cannot be asked two things at once. Give \
                     one operation per property."
                )));
            }
        }
    }
    Ok(())
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
            target: operation.target().map(str::to_owned),
            name: edits.at.as_ref().and_then(|at| at.name.clone()),
            address: edits.at.as_ref().map(|at| at.address.clone()),
            changed: *changed,
        })
        .collect()
}

/// The three answers ADR-0004 settles once for the part, in the order they
/// are run.
///
/// The order is not load-bearing and a test holds them to that: each reads a
/// different corner of the package, only the first reads what the batch
/// already wants, and no two of them write to a part the other reads. Two
/// that both merge into a declaration part merge into one splice list, which
/// `splice::apply` orders by where the splices land rather than by the order
/// they arrived in. A fourth settlement joins the list; nothing else moves.
fn settlements<R: Read + Seek>() -> [Settlement<R>; 3] {
    [
        withdraw_an_emptied_calc_chain,
        add_the_new_properties,
        put_the_new_rows_in,
    ]
}

/// Work every operation's edits out into what each part is to become.
///
/// What the operations asked for is merged part by part, the batch-level
/// questions are settled over that, and what the whole of it comes to is
/// worked out once. Nothing here knows what any of the three settlements is
/// about: they are answers to questions about parts, and a part is what they
/// are handed.
fn apply<R: Read + Seek>(
    opened: &mut Opened<R>,
    operations: &[Operation],
    asked: &[Asked],
) -> Result<Applied> {
    let mut wanted = Wanted::of(operations, asked)?;
    for settle in settlements() {
        settle(opened, &mut wanted)?;
    }
    wanted.into_applied(opened)
}

/// A calc chain with no entries left is not a calc chain, so it goes.
///
/// Whether the last entry has gone is a question about the part rather than
/// about any one operation: two operations may each take an entry out, and
/// neither can see the other's, so it is asked once, of the chain as the whole
/// batch leaves it. The part then goes with the relationship that reaches it
/// and its content type, because a package naming a part it does not hold is
/// the sort of inconsistency Excel offers to repair.
fn withdraw_an_emptied_calc_chain<R: Read + Seek>(
    opened: &mut Opened<R>,
    wanted: &mut Wanted,
) -> Result<()> {
    let Some(part) = opened.calc_chain() else {
        return Ok(());
    };
    let Some(PartEdit::Splice(splices)) = wanted.of_part(&part) else {
        return Ok(());
    };
    let left = {
        let xml = opened.text(&part)?;
        let emptied = splice::apply(xml, splices)?;
        calc_chain::entries_in(&emptied).map_err(|err| err.within(&part))?
    };
    if left > 0 {
        return Ok(());
    }

    let owner = opened.workbook_part().to_owned();
    let declarations = declared::withdrawn(opened.package(), &owner, &part)?;
    wanted.merge_all(declarations, "the emptied calc chain")?;
    wanted.withdraw(part);
    Ok(())
}

/// Add the properties the package does not already hold.
///
/// How many properties are being added is a question about the part rather
/// than about any one operation: each is added after the last one there and
/// takes the next identifier free, and over one snapshot two operations
/// adding one would both add it in the same place and both call it the same
/// thing. So it is asked once, of the part as the whole batch leaves it, the
/// way an emptied calc chain is.
///
/// Where the package has no custom properties part at all, one is written
/// holding them, and its content type and the relationship reaching it go in
/// with it, because a package holding a part it does not declare is the sort
/// of inconsistency Excel offers to repair.
fn add_the_new_properties<R: Read + Seek>(
    opened: &mut Opened<R>,
    wanted: &mut Wanted,
) -> Result<()> {
    let (part, held) = opened.properties()?;
    let mut adding: Vec<(usize, &str, properties::Value)> = Vec::new();
    for (index, operation) in wanted.operations().iter().enumerate() {
        let Operation::PropsSet { name, .. } = operation else {
            continue;
        };
        let there = match held {
            false => false,
            true => {
                let xml = opened.text(&part)?;
                properties::holds(xml, name).map_err(|err| err.within(&part))?
            }
        };
        if !there {
            let value = operation
                .property_value()
                .map_err(|err| err.within(operation.within(index)))?;
            adding.push((index, name, value));
        }
    }
    if adding.is_empty() {
        return Ok(());
    }

    let new: Vec<(&str, &properties::Value)> = adding
        .iter()
        .map(|(_, name, value)| (*name, value))
        .collect();
    let edit = match held {
        true => {
            let xml = opened.text(&part)?;
            PartEdit::Splice(properties::added(xml, &new).map_err(|err| err.within(&part))?)
        }
        false => {
            let declarations = declared::declared(
                opened.package(),
                PACKAGE_ROOT,
                &part,
                properties::CONTENT_TYPE,
                properties::CUSTOM_PROPERTIES,
            )?;
            wanted.merge_all(declarations, &format!("the declaration of {part}"))?;
            PartEdit::Create(properties::part_holding(&new))
        }
    };
    wanted.merge(part, edit, "the custom document properties")?;
    for (index, _, _) in adding {
        wanted.credit(index);
    }
    Ok(())
}

/// Put in the rows the sheets do not hold, one row however many cells go into
/// it.
///
/// How many cells a row being put in holds is a question about the part rather
/// than about any one operation: they all go in the same place and the row's
/// own tags go in once, and over one snapshot two operations writing cells of
/// one absent row cannot see each other. So it is asked once, of the batch,
/// the way an emptied calc chain and a package's new properties are.
///
/// The cells of one row go in in column order whatever order the batch named
/// them in, because that is the order a row has to read in.
fn put_the_new_rows_in<R: Read + Seek>(opened: &mut Opened<R>, wanted: &mut Wanted) -> Result<()> {
    let mut by_row: BTreeMap<(String, u32), Vec<(usize, NewCell)>> = BTreeMap::new();
    for (index, edits) in wanted.asked().iter().enumerate() {
        if let (Some(at), Some(cell)) = (edits.at.as_ref(), edits.into_a_new_row.as_ref()) {
            by_row
                .entry((at.part.clone(), cell.at.row()))
                .or_default()
                .push((index, cell.clone()));
        }
    }
    for ((part, row), members) in by_row {
        let cells: Vec<NewCell> = members.iter().map(|(_, cell)| cell.clone()).collect();
        let splice = {
            let xml = opened.text(&part)?;
            let document = Document::parse(xml)
                .map_err(|err| Error::unreadable(format!("not valid XML: {err}")).within(&part))?;
            let sheet = Worksheet::of(&document).map_err(|err| err.within(&part))?;
            let Located::InSheetData(data) =
                sheet.locate(cells[0].at).map_err(|err| err.within(&part))?
            else {
                return Err(Error::internal(format!(
                    "row {row} of part '{part}' was to be put in, and the part holds it"
                )));
            };
            worksheet::row_inserted(data, xml, row, &cells).map_err(|err| err.within(&part))?
        };
        wanted.merge(
            part,
            PartEdit::Splice(vec![splice]),
            "the rows being put in",
        )?;
        for (index, _) in members {
            wanted.credit(index);
        }
    }
    Ok(())
}

/// The splices that put `written` into the cell `at` names, in the part whose
/// text is `xml`, and whether a formula was replaced to do it.
///
/// The part is parsed here, by the operation that named it, and the splices
/// are byte ranges of the text handed in: the same text every other operation
/// on this part is handed, so their ranges all mean the same thing.
///
/// A cell the part does not hold is put there, and the row it would sit in
/// with it. There is nothing to replace in a cell that was not there, so a
/// formula never is.
///
/// `licensed` is whether the operation said a formula may be replaced. A
/// formula that is replaced goes with the value that replaces it, because
/// what goes between the cell's tags is written whole, and its calc chain
/// entry goes with it, which is what the second half of the answer is for.
fn splices_of(xml: &str, at: &Resolution, written: &Written, licensed: bool) -> Result<Wrote> {
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("not valid XML: {err}")).within(&at.part))?;
    let sheet = Worksheet::of(&document).map_err(|err| err.within(&at.part))?;
    let cell = at.address.cell;
    let located = sheet.locate(cell).map_err(|err| err.within(&at.part))?;
    let node = match located {
        Located::Cell(node) => node,
        // Clearing a cell that is not there would put an empty one where
        // there was nothing, which is a change with nothing behind it: an
        // absent cell already holds nothing and already shows the format its
        // row or its column gives it.
        _ if written == &Written::Nothing => return Ok(Wrote::default()),
        Located::InRow(row) => {
            let style = sheet
                .style_of_an_absent_cell(cell)
                .map_err(|err| err.within(&at.part))?;
            let splice = worksheet::cell_inserted(row, xml, cell, style, written)
                .map_err(|err| err.within(&at.part))?;
            return Ok(Wrote {
                splices: vec![splice],
                ..Wrote::default()
            });
        }
        // The row itself is put in by the batch, because how many cells it
        // holds is a question no one operation can answer: two writing cells
        // of one absent row cannot see each other over one snapshot, and both
        // would put the row in.
        Located::InSheetData(_) => {
            let style = sheet
                .style_of_an_absent_cell(cell)
                .map_err(|err| err.within(&at.part))?;
            return Ok(Wrote {
                into_a_new_row: Some(NewCell {
                    at: cell,
                    style,
                    written: written.clone(),
                }),
                ..Wrote::default()
            });
        }
        Located::Nowhere => {
            return Err(Error::not_found(format!(
                "no cell {} to write to: sheet '{}' has no sheet data element, so it holds \
                 no rows and there is nowhere to put one. A worksheet part without one is \
                 not a sheet Excel wrote.",
                at.address, at.address.sheet
            ))
            .within(&at.part));
        }
    };
    Ok(Wrote {
        replaced: formula_to_replace(node, at, licensed)?,
        splices: value_splices(node, xml, written)?,
        into_a_new_row: None,
    })
}

/// What a write to one cell came to.
#[derive(Debug, Default)]
struct Wrote {
    /// The splices that put the value where it goes.
    splices: Vec<Splice>,
    /// Whether a formula was replaced to make room for it, so that its calc
    /// chain entry goes too.
    replaced: bool,
    /// The cell to go into a row the sheet does not hold, where the row has
    /// to be put in. The batch puts it in; see [`put_the_new_rows_in`].
    into_a_new_row: Option<NewCell>,
}

/// The edit that makes the workbook say whether it recalculates fully on
/// load.
///
/// The flag is the workbook part's, so this is the one operation that names
/// its part rather than being handed it, and the one that reads no cell.
fn calculated<R: Read + Seek>(full_calc_on_load: bool, opened: &mut Opened<R>) -> Result<Asked> {
    let part = opened.workbook_part().to_owned();
    let xml = opened.text(&part)?;
    let splices = calculation::set_full_calc_on_load(xml, full_calc_on_load)
        .map_err(|err| err.within(&part))?;
    Ok(Asked {
        at: None,
        parts: vec![(part, PartEdit::Splice(splices))],
        into_a_new_row: None,
    })
}

/// The edit that writes `value` onto the property `name`.
///
/// A property the part already holds is written over where it stands, which
/// keeps its identifier and moves nothing else. One the part does not hold is
/// added by the batch instead — see [`add_the_new_properties`] — so this asks
/// for nothing and the batch marks the operation as having changed something.
fn property_written<R: Read + Seek>(
    name: &str,
    value: &properties::Value,
    opened: &mut Opened<R>,
) -> Result<Asked> {
    let (part, held) = opened.properties()?;
    if !held {
        return Ok(nothing_asked());
    }
    let written = {
        let xml = opened.text(&part)?;
        properties::written(xml, name, value).map_err(|err| err.within(&part))?
    };
    Ok(match written {
        None => nothing_asked(),
        Some(splices) => Asked {
            at: None,
            parts: vec![(part, PartEdit::Splice(splices))],
            into_a_new_row: None,
        },
    })
}

/// The edit that takes the property `name` out.
///
/// A property that is not there is the caller pointing at something the
/// package does not have, which is the one thing an unset can be wrong about.
fn property_withdrawn<R: Read + Seek>(name: &str, opened: &mut Opened<R>) -> Result<Asked> {
    let (part, held) = opened.properties()?;
    let splices = match held {
        false => None,
        true => {
            let xml = opened.text(&part)?;
            properties::unset(xml, name).map_err(|err| err.within(&part))?
        }
    };
    let splices = splices.ok_or_else(|| no_such_property(name, opened))?;
    Ok(Asked {
        at: None,
        parts: vec![(part, PartEdit::Splice(splices))],
        into_a_new_row: None,
    })
}

/// An operation that wants nothing done to any part.
fn nothing_asked() -> Asked {
    Asked {
        at: None,
        parts: Vec::new(),
        into_a_new_row: None,
    }
}

/// Why there is no property called `name` to take out, and what is there
/// instead.
fn no_such_property<R: Read + Seek>(name: &str, opened: &mut Opened<R>) -> Error {
    let held = (|| -> Result<Vec<String>> {
        let (part, held) = opened.properties()?;
        match held {
            false => Ok(Vec::new()),
            true => {
                let xml = opened.text(&part)?;
                Ok(properties::read(xml)?
                    .into_iter()
                    .map(|property| property.name)
                    .collect::<Vec<String>>())
            }
        }
    })()
    .unwrap_or_default();
    let names = match held.is_empty() {
        true => "the package has no custom document properties at all".to_owned(),
        false => format!("the package has {}", held.join(", ")),
    };
    Error::not_found(format!(
        "there is no custom document property called '{name}': {names}. A name is \
         matched exactly, so check its spelling and its case."
    ))
}

/// The edit that takes the cell `at` names out of the calc chain, where the
/// package holds one that mentions it.
///
/// The chain is a cache of the order the formulas were last calculated in, so
/// an entry for a cell that no longer holds a formula is an inconsistency
/// Excel may complain about. A package with no chain has nothing to maintain,
/// and a workbook that gives the sheet no number gives the chain no way to
/// name it: rather than guess at which entry is which, the chain is left as it
/// is, because a wrong entry removed is worse than a stale one kept.
fn uncalculated<R: Read + Seek>(
    opened: &mut Opened<R>,
    at: &Resolution,
) -> Result<Option<(String, PartEdit)>> {
    let (Some(part), Some(sheet)) = (opened.calc_chain(), opened.sheet_number(&at.address.sheet))
    else {
        return Ok(None);
    };
    let removal = {
        let xml = opened.text(&part)?;
        calc_chain::without(xml, sheet, at.address.cell).map_err(|err| err.within(&part))?
    };
    Ok(match removal.splices.is_empty() {
        true => None,
        false => Some((part, PartEdit::Splice(removal.splices))),
    })
}

/// Whether a formula is being replaced, refusing where the operation was not
/// licensed to replace one.
///
/// The spec's guarantee is that a mis-addressed write cannot silently destroy
/// a template's formula, so a cell holding one is refused unless the operation
/// says it meant it. A shared master carries the formula its children take
/// theirs from, and overwriting it orphans them, so it is refused whatever the
/// operation says, and its range is in the message so the caller can see what
/// it would have taken with it.
fn formula_to_replace(node: Node, at: &Resolution, licensed: bool) -> Result<bool> {
    let Some(formula) = formula_of(node, at.address.cell)? else {
        return Ok(false);
    };
    if formula.role == FormulaRole::SharedMaster {
        return Err(Error::refused(format!(
            "cell {} carries the formula shared across {}; overwriting it would \
             orphan the rest of the range, so no flag licenses it. Write to a \
             cell outside it.",
            at.address,
            formula.range.as_deref().unwrap_or("its group")
        )));
    }
    if licensed {
        return Ok(true);
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
        "cell {} {what}; pass --replace-formula, or replace_formula: true in a \
         batch, to replace it and take its calc chain entry with it.",
        at.address
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::error::ErrorCode;
    use crate::reference::Cell;

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
    const NAMES: [(WriteType, &str); 4] = [
        (WriteType::Number, "number"),
        (WriteType::Text, "text"),
        (WriteType::Bool, "bool"),
        (WriteType::Date, "date"),
    ];

    /// The 1900 system, which is what a workbook that says nothing is on: the
    /// system a test that is not about dates is reading against.
    const DATES: DateSystem = DateSystem::Date1900;

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
        assert_eq!(
            WriteType::named("datetime"),
            None,
            "a name no write type carries is no write type"
        );
    }

    #[test]
    fn a_value_that_is_not_of_the_type_it_names_is_a_usage_error() {
        for text in ["", "hello", "1,5", "inf", "NaN", "2 "] {
            let err = WriteType::Number.read(text, DATES).expect_err(text);
            assert_eq!(err.code(), ErrorCode::Usage, "{text}");
        }
        for text in ["TRUE", "False", "1", "0"] {
            WriteType::Bool.read(text, DATES).expect(text);
        }
        for text in ["", "yes", "2"] {
            let err = WriteType::Bool.read(text, DATES).expect_err(text);
            assert_eq!(err.code(), ErrorCode::Usage, "{text}");
        }
        for text in ["", "hello", "42", "true"] {
            assert_eq!(
                WriteType::Text.read(text, DATES).expect(text),
                Written::text(text),
                "any text is text"
            );
        }
    }

    /// A batch may ask of one cell in more than one way, and two operations
    /// may fail for the same reason, so every failure of an operation says
    /// which operation it was, by the index the report counts by, as well as
    /// what it asked of which target.
    #[test]
    fn a_failure_of_an_operation_names_the_operation_it_came_from() {
        let operation = Operation::Set {
            target: "Inputs!A1".to_owned(),
            write_type: WriteType::Number,
            value: "hello".to_owned(),
            replace_formula: false,
        };

        let err = operation
            .written(DATES)
            .expect_err("hello is not a number")
            .within(operation.within(1));

        assert_eq!(err.code(), ErrorCode::Usage);
        assert!(
            err.message()
                .starts_with("operation at index 1 (set Inputs!A1): 'hello' is not a number;"),
            "{err}"
        );
    }

    fn writing(target: &str, write_type: WriteType, value: &str) -> Operation {
        Operation::Set {
            target: target.to_owned(),
            write_type,
            value: value.to_owned(),
            replace_formula: false,
        }
    }

    /// The document `apply` parses a batch from, spelled out here rather than
    /// derived from the code: a change to any of this is a change to what a
    /// caller writes. An array of operations is the shape #8 asks for; a value
    /// is text under the type that says how to read it, which is what #20
    /// asks for; `replace_formula` is carried for #10 to honour.
    #[test]
    fn a_batch_round_trips_through_json() {
        let batch = Batch {
            operations: vec![
                writing("Inputs!A1", WriteType::Number, "42.5"),
                Operation::Set {
                    target: "Total".to_owned(),
                    write_type: WriteType::Text,
                    value: " kept ".to_owned(),
                    replace_formula: true,
                },
            ],
        };

        let json = serde_json::to_value(&batch).expect("a batch serialises");

        assert_eq!(
            json,
            serde_json::json!([
                {"op": "set", "target": "Inputs!A1", "type": "number", "value": "42.5",
                 "replace_formula": false},
                {"op": "set", "target": "Total", "type": "text", "value": " kept ",
                 "replace_formula": true}
            ])
        );
        assert_eq!(
            serde_json::from_value::<Batch>(json).expect("a batch deserialises"),
            batch
        );
    }

    /// `replace_formula` may be left out, because most operations have no
    /// opinion about a formula and every one of them would otherwise have to
    /// say so.
    #[test]
    fn an_operation_that_says_nothing_about_a_formula_does_not_replace_one() {
        let batch =
            Batch::parse(r#"[{"op": "set", "target": "Inputs!A1", "type": "bool", "value": "1"}]"#)
                .expect("a batch may leave the flag out");

        assert_eq!(
            batch.operations,
            [writing("Inputs!A1", WriteType::Bool, "1")]
        );
    }

    /// A field an operation does not know is ignored, so a batch written
    /// against a later version still reads here.
    #[test]
    fn a_field_an_operation_does_not_know_is_ignored() {
        let batch = Batch::parse(
            r#"[{"op": "set", "target": "Inputs!A1", "type": "bool", "value": "1",
                 "why": "a field no ticket has added"}]"#,
        )
        .expect("a batch deserialises");

        assert_eq!(
            batch.operations,
            [writing("Inputs!A1", WriteType::Bool, "1")]
        );
    }

    /// The kinds, transcribed rather than derived from the code, so that a
    /// kind added without being listed fails the test: the list is what a
    /// caller naming an unknown kind is told.
    #[test]
    fn every_operation_kind_is_listed_under_the_name_a_batch_spells_it_with() {
        for kind in Operation::KINDS {
            // Every field any kind takes, so that one document serves them
            // all: a field an operation does not know is ignored.
            let json = format!(
                r#"[{{"op": "{kind}", "target": "A1", "name": "P", "type": "text",
                      "value": "", "full_calc_on_load": true}}]"#
            );
            let batch = Batch::parse(&json).unwrap_or_else(|err| panic!("{kind}: {err}"));
            assert_eq!(batch.operations[0].kind(), kind);
        }
        assert_eq!(
            Operation::KINDS,
            ["set", "clear", "calc", "props.set", "props.unset"],
            "a kind with no list entry"
        );
    }

    /// A kind this build does not know is the caller's mistake, and the
    /// likeliest one is a batch written for a later ticket, so the failure
    /// says what this build does know.
    #[test]
    fn a_kind_this_build_does_not_know_is_a_usage_error_listing_the_ones_it_does() {
        let err = Batch::parse(r#"[{"op": "props.set", "name": "Ref", "value": "1"}]"#)
            .expect_err("props.set is #12's to add");

        assert_eq!(err.code(), ErrorCode::Usage);
        assert!(err.message().contains("props.set"), "{}", err.message());
        // Derived from the list rather than transcribed, because the list
        // itself is transcribed in the test above: what matters here is that
        // the failure carries it.
        assert!(
            err.message().contains(&format!(
                "The operation kinds are: {}.",
                Operation::KINDS.join(", ")
            )),
            "{}",
            err.message()
        );
    }

    /// A document that is not JSON at all fails on that alone: the operation
    /// kinds have nothing to do with it and would only be noise.
    #[test]
    fn a_document_that_is_not_json_says_so_and_nothing_about_kinds() {
        let err = Batch::parse("not json").expect_err("that is not JSON");

        assert_eq!(err.code(), ErrorCode::Usage);
        assert!(!err.message().contains("kinds are"), "{}", err.message());
    }

    fn splice() -> Splice {
        Splice::new(0..1, "x")
    }

    /// An operation that names no cell reports none, and the batch resolves
    /// nothing on its behalf. `calc` is the first such operation, and #12's
    /// properties are the next.
    #[test]
    fn an_operation_that_names_no_cell_is_reported_with_no_address() {
        let operations = [
            writing("Inputs!A1", WriteType::Number, "1"),
            Operation::Calc {
                full_calc_on_load: true,
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
                into_a_new_row: None,
            },
            Asked {
                at: None,
                parts: vec![("xl/workbook.xml".to_owned(), PartEdit::Splice(Vec::new()))],
                into_a_new_row: None,
            },
        ];

        let reports = reports(&operations, &asked, &[true, false]);

        assert_eq!(
            reports[0].address.as_ref().map(ToString::to_string),
            Some("Inputs!A1".to_owned())
        );
        assert_eq!(reports[1].address, None, "no cell, no address");
        assert_eq!(reports[1].target, None, "and no target either");
    }

    #[test]
    fn a_batch_of_one_holds_that_one_operation() {
        let operation = writing("Inputs!A1", WriteType::Number, "1");

        assert_eq!(Batch::of(operation.clone()).operations, [operation]);
    }

    /// The parts of a package these tests open: one sheet, reached the way
    /// Excel reaches it, and the declarations a part put in or taken out has
    /// to be declared in. Built here rather than copied from a fixture,
    /// because a settlement is about what it does to the parts rather than
    /// about any real workbook.
    const TYPES: &str = concat!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
        r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/>"#,
        "</Types>"
    );

    const ROOT_RELS: &str = concat!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>"#,
        "</Relationships>"
    );

    const WORKBOOK: &str = concat!(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" "#,
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
        r#"<sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );

    const WORKBOOK_RELS: &str = concat!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>"#,
        "</Relationships>"
    );

    const SHEET: &str = concat!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        r#"<sheetData><row r="1"><c r="A1"><f>1+1</f><v>2</v></c></row>"#,
        r#"<row r="4"><c r="A4"><v>4</v></c></row></sheetData></worksheet>"#
    );

    const CHAIN: &str = r#"<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="A1" i="1"/></calcChain>"#;

    const SHEET_PART: &str = "xl/worksheets/sheet1.xml";
    const CHAIN_PART: &str = "xl/calcChain.xml";
    const TYPES_PART: &str = "[Content_Types].xml";
    const ROOT_RELS_PART: &str = "_rels/.rels";

    /// The package as it is, with `parts` put in or over what it holds.
    ///
    /// The settlements are asked about a package rather than about a file, so
    /// the package is built where it is read and there is nothing to clean up
    /// after (ADR-0005).
    fn opened(parts: &[(&str, &str)]) -> Opened<FromBytes> {
        let mut all: Vec<(&str, &str)> = vec![
            (TYPES_PART, TYPES),
            (ROOT_RELS_PART, ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            (SHEET_PART, SHEET),
        ];
        for (part, text) in parts {
            match all.iter_mut().find(|(held, _)| held == part) {
                Some(entry) => entry.1 = text,
                None => all.push((part, text)),
            }
        }
        Opened::of(crate::package::container_of(&all)).expect("the parts make a package")
    }

    /// What the batch wants of `part` once the splices it holds are applied.
    fn spliced_text(wanted: &Wanted, part: &str, was: &str) -> String {
        let Some(PartEdit::Splice(splices)) = wanted.of_part(part) else {
            panic!("the batch wants nothing spliced in '{part}'");
        };
        splice::apply(was, splices).expect("the splices apply")
    }

    /// One operation, answering with one part edit: what a settlement is
    /// given to work over.
    fn asking(part: &str, edit: PartEdit) -> Asked {
        Asked {
            at: None,
            parts: vec![(part.to_owned(), edit)],
            into_a_new_row: None,
        }
    }

    /// The splice that takes the one entry out of the chain, which is what an
    /// operation replacing that cell's formula would have answered with.
    fn emptying_the_chain() -> Splice {
        let from = CHAIN.find("<c ").expect("the chain holds an entry");
        let to = CHAIN.find("</calcChain>").expect("the chain closes");
        Splice::new(from..to, "")
    }

    #[test]
    fn a_chain_with_no_entries_left_is_withdrawn_with_the_declarations_that_name_it() {
        let mut opened = opened(&[(CHAIN_PART, CHAIN)]);
        let asked = [asking(
            CHAIN_PART,
            PartEdit::Splice(vec![emptying_the_chain()]),
        )];
        let mut wanted = Wanted::of(&[], &asked).expect("one operation, one part");

        withdraw_an_emptied_calc_chain(&mut opened, &mut wanted).expect("the chain is readable");

        assert_eq!(
            wanted.of_part(CHAIN_PART),
            Some(&PartEdit::Remove),
            "an emptied chain is not a chain"
        );
        assert!(
            !spliced_text(&wanted, TYPES_PART, TYPES).contains("calcChain"),
            "the content type of a part that has gone goes with it"
        );
        assert!(
            !spliced_text(&wanted, "xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
                .contains("calcChain"),
            "and so does the relationship that reached it"
        );
    }

    #[test]
    fn a_chain_that_still_holds_an_entry_is_spliced_and_left_where_it_is() {
        let two = CHAIN.replace("</calcChain>", r#"<c r="A4" i="1"/></calcChain>"#);
        let mut opened = opened(&[(CHAIN_PART, &two)]);
        let from = two.find("<c ").expect("the chain holds an entry");
        let to = two.find(r#"<c r="A4""#).expect("it holds a second");
        let asked = [asking(
            CHAIN_PART,
            PartEdit::Splice(vec![Splice::new(from..to, "")]),
        )];
        let mut wanted = Wanted::of(&[], &asked).expect("one operation, one part");

        withdraw_an_emptied_calc_chain(&mut opened, &mut wanted).expect("the chain is readable");

        assert!(
            matches!(wanted.of_part(CHAIN_PART), Some(PartEdit::Splice(_))),
            "the chain is spliced and stays"
        );
        for declaring in [TYPES_PART, "xl/_rels/workbook.xml.rels"] {
            assert_eq!(
                wanted.of_part(declaring),
                None,
                "nothing is declared differently"
            );
        }
    }

    #[test]
    fn a_package_holding_no_chain_has_none_to_withdraw() {
        let mut opened = opened(&[]);
        let asked = [asking(
            SHEET_PART,
            PartEdit::Splice(vec![Splice::new(0..0, "")]),
        )];
        let mut wanted = Wanted::of(&[], &asked).expect("one operation, one part");

        withdraw_an_emptied_calc_chain(&mut opened, &mut wanted).expect("there is nothing to read");

        assert!(wanted.of_part(SHEET_PART).is_some());
        assert_eq!(
            wanted.of_part(CHAIN_PART),
            None,
            "the sheet's own edit, and nothing else"
        );
    }

    /// A property the package has no part for brings the part with it, and the
    /// declarations that reach a part it does not hold yet.
    #[test]
    fn a_property_added_to_a_package_with_no_properties_part_creates_one_and_declares_it() {
        let mut opened = opened(&[]);
        let operations = [Operation::PropsSet {
            name: "Reference".to_owned(),
            write_type: WriteType::Text,
            value: "R-1".to_owned(),
        }];
        let asked = [nothing_asked()];
        let mut wanted = Wanted::of(&operations, &asked).expect("one operation");

        add_the_new_properties(&mut opened, &mut wanted).expect("the property is added");

        let Some(PartEdit::Create(bytes)) = wanted.of_part(properties::CONVENTIONAL_PART) else {
            panic!("the properties part is created");
        };
        let part = String::from_utf8(bytes.clone()).expect("a part of XML text");
        assert!(part.contains("Reference"), "{part}");
        assert!(part.contains("R-1"), "{part}");
        assert!(
            spliced_text(&wanted, TYPES_PART, TYPES).contains(properties::CONTENT_TYPE),
            "a part the package holds is a part it declares"
        );
        assert!(
            spliced_text(&wanted, ROOT_RELS_PART, ROOT_RELS).contains("docProps/custom.xml"),
            "and a part it holds is one it can reach"
        );
        assert_eq!(
            wanted.credited(),
            [true],
            "the operation that added it changed it"
        );
    }

    /// Two properties over one snapshot are one part, because neither could
    /// have seen the other put in.
    #[test]
    fn two_properties_added_at_once_go_into_one_created_part() {
        let mut opened = opened(&[]);
        let operations = [
            Operation::PropsSet {
                name: "Reference".to_owned(),
                write_type: WriteType::Text,
                value: "R-1".to_owned(),
            },
            Operation::PropsSet {
                name: "Revision".to_owned(),
                write_type: WriteType::Number,
                value: "2".to_owned(),
            },
        ];
        let asked = [nothing_asked(), nothing_asked()];
        let mut wanted = Wanted::of(&operations, &asked).expect("two operations");

        add_the_new_properties(&mut opened, &mut wanted).expect("both properties are added");

        let Some(PartEdit::Create(bytes)) = wanted.of_part(properties::CONVENTIONAL_PART) else {
            panic!("the properties part is created");
        };
        let part = String::from_utf8(bytes.clone()).expect("a part of XML text");
        assert!(
            part.contains("Reference") && part.contains("Revision"),
            "{part}"
        );
        assert_eq!(wanted.credited(), [true, true]);
    }

    /// A property already there is written by the operation's own splice, so
    /// the settlement has nothing to add and says nothing about it.
    #[test]
    fn a_property_the_package_already_holds_is_not_added_again() {
        let held =
            properties::part_holding(&[("Reference", &properties::Value::Text("R-0".to_owned()))]);
        let held = String::from_utf8(held).expect("a part of XML text");
        let declared_rels = ROOT_RELS.replace(
            "</Relationships>",
            &format!(
                r#"<Relationship Id="rId2" Type="{}" Target="docProps/custom.xml"/></Relationships>"#,
                properties::CUSTOM_PROPERTIES
            ),
        );
        let mut opened = opened(&[
            (properties::CONVENTIONAL_PART, &held),
            (ROOT_RELS_PART, &declared_rels),
        ]);
        let operations = [Operation::PropsSet {
            name: "Reference".to_owned(),
            write_type: WriteType::Text,
            value: "R-1".to_owned(),
        }];
        let asked = [nothing_asked()];
        let mut wanted = Wanted::of(&operations, &asked).expect("one operation");

        add_the_new_properties(&mut opened, &mut wanted).expect("the property is there");

        assert_eq!(
            wanted.of_part(properties::CONVENTIONAL_PART),
            None,
            "nothing to add"
        );
        assert_eq!(
            wanted.credited(),
            [false],
            "whether it changed is the splice's to say"
        );
    }

    /// Two cells of one absent row are one row, holding both, in column order
    /// whatever order the batch named them in.
    #[test]
    fn two_cells_of_one_absent_row_are_put_in_as_one_row() {
        let mut opened = opened(&[]);
        let asked = [new_cell("Inputs!B2", 2, 2), new_cell("Inputs!A2", 1, 2)];
        let mut wanted = Wanted::of(&[], &asked).expect("two operations");

        put_the_new_rows_in(&mut opened, &mut wanted).expect("the row goes in");

        let sheet = spliced_text(&wanted, SHEET_PART, SHEET);
        assert!(
            sheet.contains(concat!(
                r#"<row r="2"><c r="A2" t="inlineStr"><is><t>A2</t></is></c>"#,
                r#"<c r="B2" t="inlineStr"><is><t>B2</t></is></c></row>"#
            )),
            "one row, its cells in column order: {sheet}"
        );
        assert_eq!(wanted.credited(), [true, true]);
    }

    /// Two absent rows are two rows, each put in where it belongs.
    #[test]
    fn cells_of_two_absent_rows_are_two_rows() {
        let mut opened = opened(&[]);
        let asked = [new_cell("Inputs!A3", 1, 3), new_cell("Inputs!A2", 1, 2)];
        let mut wanted = Wanted::of(&[], &asked).expect("two operations");

        put_the_new_rows_in(&mut opened, &mut wanted).expect("both rows go in");

        let sheet = spliced_text(&wanted, SHEET_PART, SHEET);
        let (row2, row3) = (
            sheet.find(r#"<row r="2""#).expect("row 2 went in"),
            sheet.find(r#"<row r="3""#).expect("row 3 went in"),
        );
        let row4 = sheet.find(r#"<row r="4""#).expect("row 4 was there");
        assert!(
            row2 < row3 && row3 < row4,
            "each row in its own place: {sheet}"
        );
    }

    /// Two operations landing on one worksheet read it once between them:
    /// the package memoises the text of a part it has read, which is what
    /// lets an operation own its own reading without every operation paying
    /// for it (ADR-0005).
    #[test]
    fn a_part_two_operations_both_read_is_read_once() {
        let mut opened = opened(&[]);
        let write = |target: &str| Operation::Set {
            target: target.to_owned(),
            write_type: WriteType::Number,
            value: "7".to_owned(),
            replace_formula: false,
        };
        let (one, two) = (write("Inputs!A4"), write("Inputs!B4"));
        let at = |operation: &Operation| {
            operation
                .at(&opened)
                .expect("both cells are in the package")
        };

        let opening = opened.reads();
        let (at_one, at_two) = (at(&one), at(&two));
        one.edits(at_one, &mut opened).expect("A4 must be writable");
        let after_one = opened.reads();
        two.edits(at_two, &mut opened).expect("B4 must be writable");

        assert_eq!(
            opening, 3,
            "opening a package reads the root relationships, the workbook part \
             they name, and that part's own relationships, and resolving a \
             target reads nothing more"
        );
        assert_eq!(
            after_one,
            opening + 1,
            "the first operation read the worksheet its cell sits in"
        );
        assert_eq!(
            opened.reads(),
            after_one,
            "the second operation read no part the first had not"
        );
    }

    /// The order the settlements run in is not load-bearing, which is what
    /// lets them be a list rather than three lines that have to stay in that
    /// order. This runs a batch that fires all three — an emptied calc chain,
    /// a properties part that has to be created, and a row that has to be put
    /// in — forwards and backwards, and the package comes out the same.
    ///
    /// Two of them merge into `[Content_Types].xml`, which is where an order
    /// would show if there were one: the splices arrive in a different order
    /// and `splice::apply` puts them in the order they land in either way.
    #[test]
    fn the_settlements_answer_the_same_whatever_order_they_run_in() {
        let operations = [
            Operation::PropsSet {
                name: "Reference".to_owned(),
                write_type: WriteType::Text,
                value: "R-1".to_owned(),
            },
            writing("Inputs!A1", WriteType::Number, "1"),
            writing("Inputs!A2", WriteType::Number, "2"),
        ];
        let asked = [
            nothing_asked(),
            asking(CHAIN_PART, PartEdit::Splice(vec![emptying_the_chain()])),
            new_cell("Inputs!A2", 1, 2),
        ];
        let settled = |backwards: bool| {
            let mut opened = opened(&[(CHAIN_PART, CHAIN)]);
            let mut wanted = Wanted::of(&operations, &asked).expect("three operations");
            let mut order = settlements().to_vec();
            if backwards {
                order.reverse();
            }
            for settle in order {
                settle(&mut opened, &mut wanted).expect("every settlement answers");
            }
            wanted
                .into_applied(&mut opened)
                .expect("what the batch wants is what the parts become")
        };

        let (forwards, backwards) = (settled(false), settled(true));

        assert_eq!(
            forwards.parts.added,
            [properties::CONVENTIONAL_PART],
            "the properties part was created, so that settlement fired"
        );
        assert_eq!(
            forwards.parts.removed,
            [CHAIN_PART],
            "the chain was withdrawn, so that one did too"
        );
        assert!(
            forwards.parts.changed.contains(&SHEET_PART.to_owned()),
            "and the row went into the sheet: {:?}",
            forwards.parts.changed
        );
        assert_eq!(forwards.content, backwards.content, "part for part");
        assert_eq!(forwards.parts, backwards.parts);
        assert_eq!(forwards.changed, backwards.changed);
    }

    /// One operation putting one cell into a row the sheet does not hold.
    fn new_cell(target: &str, column: u32, row: u32) -> Asked {
        let cell = Cell::new(column, row).expect("a cell on the grid");
        Asked {
            at: Some(Resolution {
                target: target.to_owned(),
                name: None,
                address: Address {
                    sheet: "Inputs".to_owned(),
                    cell,
                },
                part: SHEET_PART.to_owned(),
            }),
            parts: Vec::new(),
            into_a_new_row: Some(NewCell {
                at: cell,
                style: None,
                written: Written::text(&cell.a1()),
            }),
        }
    }
}
