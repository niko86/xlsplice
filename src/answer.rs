//! What a verb answers with, in both shapes at once.
//!
//! An [`Answer`] carries what one verb found twice over: the payload the JSON
//! envelope flattens, and the same facts as rows a person reads. Both are
//! built here, side by side, so that what one says can be read against the
//! other, and the constructor is where the shape of the rows is checked,
//! once, for every verb.
//!
//! The JSON shape is the stable one and may only grow: a field is never
//! removed or retyped, and a field that does not apply is `null` rather than
//! absent, so every member of a list has the same keys.
//!
//! A verb that answers about a list of things builds one [`Entry`] per thing
//! and takes the row from it, so the two shapes are worked out in one place
//! and cannot drift apart. Where the two spell a value differently — a large
//! non-integral number is one — the entry carries both spellings side by
//! side, worked out from the one value.

use serde::Serialize;

use crate::batch::{OperationReport, Report};
use crate::cells::{CellReport, Value};
use crate::diff::{Difference, PartStatus};
use crate::error::{Error, Result};
use crate::help::Topic;
use crate::properties::{self, Property};
use crate::workbook::{DefinedName, Resolved, Scope, Sheet, Workbook};
use crate::worksheet::Formula;
use crate::xml::number_text;

/// What one verb found: the payload for the envelope, and the same facts in
/// the shape human output takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    payload: serde_json::Value,
    shape: Shape,
}

/// The human form of an answer. Most verbs answer with rows under headers;
/// a verb whose answer is a sentence answers with one line, which is the same
/// line in a terminal and in a pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Shape {
    /// One line, whatever the style: what a verb answering in a sentence
    /// says.
    Line(String),
    /// One row per thing answered, under the columns they are fields of.
    Rows {
        headers: &'static [&'static str],
        rows: Vec<Vec<String>>,
    },
}

impl Answer {
    /// An answer whose human form is a table: one row per thing answered,
    /// under `headers`.
    ///
    /// This is the one place the arity invariant lives. A row that is not as
    /// wide as the headers would put a field under the wrong column, and to a
    /// caller reading the tab-separated form a field is always the same field;
    /// so a verb that builds one is refused here, once, rather than by a
    /// hand-written test per verb.
    pub fn rows(
        payload: impl Serialize,
        headers: &'static [&'static str],
        rows: Vec<Vec<String>>,
    ) -> Result<Self> {
        for (index, row) in rows.iter().enumerate() {
            if row.len() != headers.len() {
                return Err(Error::internal(format!(
                    "row {index} has {} field(s) under {} column(s): {headers:?}",
                    row.len(),
                    headers.len()
                )));
            }
        }
        Ok(Answer {
            payload: object(payload)?,
            shape: Shape::Rows { headers, rows },
        })
    }

    /// An answer whose human form is one line.
    pub fn line(payload: impl Serialize, line: impl Into<String>) -> Result<Self> {
        Ok(Answer {
            payload: object(payload)?,
            shape: Shape::Line(line.into()),
        })
    }

    /// The payload, as the envelope flattens it.
    pub fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    /// The human form, for whatever is laying it out. Nothing outside the
    /// library needs it: a caller wanting text asks for text rendered.
    pub(crate) fn shape(&self) -> &Shape {
        &self.shape
    }
}

/// A payload as the JSON object the envelope flattens it into.
///
/// The envelope carries `ok` and `schema_version` beside the payload's own
/// members, so a payload that is not an object has nowhere to go. No verb
/// builds one; this is where that stops being something to remember.
fn object(payload: impl Serialize) -> Result<serde_json::Value> {
    let value = serde_json::to_value(payload)
        .map_err(|err| Error::internal(format!("a payload must serialise: {err}")))?;
    match value.is_object() {
        true => Ok(value),
        false => Err(Error::internal(format!(
            "a payload must be a JSON object, not {value}"
        ))),
    }
}

/// One thing a verb answers about, in both shapes at once.
///
/// The entry is where the facts are gathered, once, and the row is taken from
/// the entry rather than gathered again beside it. So an enum is matched once
/// per verb, and a field cannot say one thing in the envelope and another in
/// the table.
///
/// Every implementation destructures itself to build its row, which is what
/// makes a field added to an entry a compile error until the row has been
/// told about it, and `N` is what holds the row to the width of the columns
/// it goes under. A field the table has no column for is bound to `_`, said
/// out loud rather than left out.
trait Entry<const N: usize> {
    /// The columns this entry's fields sit under.
    const HEADERS: [&'static str; N];

    /// This entry as one row, in the order the columns are in.
    fn row(&self) -> [String; N];
}

/// The rows of a list of entries, in the shape [`Answer::rows`] takes.
fn rows_of<const N: usize, E: Entry<N>>(entries: &[E]) -> Vec<Vec<String>> {
    entries.iter().map(|entry| entry.row().to_vec()).collect()
}

/// The payload of `sheets --json`.
#[derive(Serialize)]
struct Sheets {
    sheets: Vec<SheetEntry>,
}

#[derive(Serialize)]
struct SheetEntry {
    /// The sheet's name, in the package's own spelling.
    name: String,
    /// `visible`, `hidden` or `veryHidden`.
    state: &'static str,
}

impl SheetEntry {
    fn of(sheet: &Sheet) -> Self {
        SheetEntry {
            name: sheet.name.clone(),
            state: sheet.state.as_str(),
        }
    }
}

impl Entry<2> for SheetEntry {
    const HEADERS: [&'static str; 2] = ["NAME", "STATE"];

    fn row(&self) -> [String; 2] {
        let SheetEntry { name, state } = self;
        [name.clone(), (*state).to_owned()]
    }
}

/// What `sheets` answers: the package's sheets, in workbook order.
pub fn sheets(workbook: &Workbook) -> Result<Answer> {
    let sheets: Vec<SheetEntry> = workbook.sheets().iter().map(SheetEntry::of).collect();
    let rows = rows_of(&sheets);
    Answer::rows(Sheets { sheets }, &SheetEntry::HEADERS, rows)
}

/// The payload of `names --json`.
#[derive(Serialize)]
struct Names {
    names: Vec<NameEntry>,
}

#[derive(Serialize)]
struct NameEntry {
    /// The name as the package declares it.
    name: String,
    /// `workbook` or `sheet`.
    scope: &'static str,
    /// The sheet a sheet-scoped name belongs to; `null` for a
    /// workbook-scoped one. Kept apart from `scope` so that a sheet called
    /// "workbook" is not mistaken for a scope.
    scope_sheet: Option<String>,
    /// The reference text exactly as the package holds it.
    refers_to: String,
    /// The one cell the name stands for, or `null`.
    anchor: Option<AnchorEntry>,
    /// Why there is no anchor, or `null` when there is one.
    reason: Option<&'static str>,
}

#[derive(Serialize)]
struct AnchorEntry {
    /// The sheet, in the package's own spelling.
    sheet: String,
    /// The cell in A1 form.
    cell: String,
    /// Both together, quoted as a reference: what `get` would accept back.
    address: String,
}

impl NameEntry {
    /// The two enums a defined name carries are matched here and nowhere
    /// else: what it resolved to, and what it is scoped to.
    fn of(name: &DefinedName) -> Self {
        let (anchor, reason) = match &name.resolved {
            Resolved::Anchor(address) => (
                Some(AnchorEntry {
                    sheet: address.sheet.clone(),
                    cell: address.cell.a1(),
                    address: address.to_string(),
                }),
                None,
            ),
            Resolved::Unresolvable(why) => (None, Some(why.as_str())),
        };
        let (scope, scope_sheet) = match &name.scope {
            Scope::Workbook => ("workbook", None),
            Scope::Sheet(sheet) => ("sheet", Some(sheet.clone())),
        };
        NameEntry {
            name: name.name.clone(),
            scope,
            scope_sheet,
            refers_to: name.refers_to.clone(),
            anchor,
            reason,
        }
    }
}

impl Entry<5> for NameEntry {
    const HEADERS: [&'static str; 5] = ["NAME", "SCOPE", "REFERS TO", "ANCHOR", "REASON"];

    /// Every row has all five fields, one of the last two always empty, so a
    /// field is always the same field to `cut`. The scope column carries the
    /// sheet where there is one, because a table has room for one column and
    /// the sheet is the more telling of the two.
    fn row(&self) -> [String; 5] {
        let NameEntry {
            name,
            scope,
            scope_sheet,
            refers_to,
            anchor,
            reason,
        } = self;
        [
            name.clone(),
            scope_sheet.clone().unwrap_or_else(|| (*scope).to_owned()),
            refers_to.clone(),
            anchor
                .as_ref()
                .map(|anchor| anchor.address.clone())
                .unwrap_or_default(),
            reason.unwrap_or_default().to_owned(),
        ]
    }
}

/// What `names` answers: the defined names, in the order the package declares
/// them.
pub fn names(workbook: &Workbook) -> Result<Answer> {
    let names: Vec<NameEntry> = workbook.defined_names().iter().map(NameEntry::of).collect();
    let rows = rows_of(&names);
    Answer::rows(Names { names }, &NameEntry::HEADERS, rows)
}

/// The payload of `get --json`.
#[derive(Serialize)]
struct Cells {
    cells: Vec<CellEntry>,
}

#[derive(Serialize)]
struct CellEntry {
    /// The operand exactly as it was given.
    target: String,
    /// The defined name the operand went through, in the package's own
    /// spelling, or `null` when the operand was an address.
    name: Option<String>,
    /// The sheet, in the package's own spelling.
    sheet: String,
    /// The cell in A1 form.
    cell: String,
    /// Both together, quoted as a reference: what `get` would accept back.
    address: String,
    /// The type the value is stored as: `n`, `s`, `str`, `inlineStr`, `b`,
    /// `e`, `d`, or `empty` for a cell that stores no value.
    #[serde(rename = "type")]
    kind: &'static str,
    /// The value, typed: a number, a string, a boolean, or `null` for a cell
    /// that stores no value.
    value: serde_json::Value,
    /// The same value as the table spells it, which is not always how JSON
    /// spells it: `serde_json` writes 1e300 as `1e+300` where a double's own
    /// Display writes three hundred digits. A row derived from the payload's
    /// JSON would move those bytes, so the two spellings are worked out from
    /// the one value and kept side by side.
    #[serde(skip)]
    value_text: String,
    /// The exact stored text; for a shared-string cell, the string index.
    /// `null` for a cell that stores no value.
    raw: Option<String>,
    /// The cell's formula, or `null`.
    formula: Option<FormulaEntry>,
    /// The style index, or `null` for a cell the part does not hold.
    style: Option<u32>,
}

#[derive(Serialize)]
struct FormulaEntry {
    /// The formula text as stored, without a leading `=`. A shared child
    /// stores none of its own, and carries the empty string.
    text: String,
    /// `plain`, `shared_master`, `shared_child`, `array` or `data_table`.
    role: &'static str,
    /// The range a shared master or an array formula covers, else `null`.
    range: Option<String>,
    /// The shared group a master or a child belongs to, else `null`.
    group: Option<u32>,
}

impl FormulaEntry {
    fn of(formula: &Formula) -> Self {
        FormulaEntry {
            text: formula.text.clone(),
            role: formula.role.as_str(),
            range: formula.range.clone(),
            group: formula.group,
        }
    }

    /// The four columns a formula fills. A cell with none leaves all four
    /// empty, which is what [`Default`] gives.
    fn fields(&self) -> [String; 4] {
        let FormulaEntry {
            text,
            role,
            range,
            group,
        } = self;
        [
            text.clone(),
            (*role).to_owned(),
            range.clone().unwrap_or_default(),
            group.map(|si| si.to_string()).unwrap_or_default(),
        ]
    }
}

impl CellEntry {
    /// What the cell holds is turned into both of its spellings here, and its
    /// formula unwrapped here, so neither is done twice.
    fn of(report: &CellReport) -> Self {
        let (value, value_text) = match &report.value {
            Value::Number(number) => (number_json(*number), number.to_string()),
            Value::Text(text) => (serde_json::Value::String(text.clone()), text.clone()),
            Value::Bool(yes) => (serde_json::Value::Bool(*yes), yes.to_string()),
            Value::Empty => (serde_json::Value::Null, String::new()),
        };
        CellEntry {
            target: report.target.clone(),
            name: report.name.clone(),
            sheet: report.address.sheet.clone(),
            cell: report.address.cell.a1(),
            address: report.address.to_string(),
            kind: report.kind.as_str(),
            value,
            value_text,
            raw: report.raw.clone(),
            formula: report.formula.as_ref().map(FormulaEntry::of),
            style: report.style,
        }
    }
}

impl Entry<11> for CellEntry {
    const HEADERS: [&'static str; 11] = [
        "TARGET", "NAME", "ADDRESS", "TYPE", "VALUE", "RAW", "STYLE", "FORMULA", "ROLE", "RANGE",
        "GROUP",
    ];

    /// Every row has all eleven fields, whatever the cell holds, so a field
    /// is always the same field to `cut`; the formula's three trail at the end
    /// because most cells have none. The sheet and the cell have no column of
    /// their own because the address column carries both.
    fn row(&self) -> [String; 11] {
        let CellEntry {
            target,
            name,
            sheet: _,
            cell: _,
            address,
            kind,
            value: _,
            value_text,
            raw,
            formula,
            style,
        } = self;
        let [text, role, range, group] = formula
            .as_ref()
            .map(FormulaEntry::fields)
            .unwrap_or_default();
        [
            target.clone(),
            name.clone().unwrap_or_default(),
            address.clone(),
            (*kind).to_owned(),
            value_text.clone(),
            raw.clone().unwrap_or_default(),
            style.map(|index| index.to_string()).unwrap_or_default(),
            text,
            role,
            range,
            group,
        ]
    }
}

/// What `get` answers: one cell per target, in the order the targets were
/// given.
pub fn cells(reports: &[CellReport]) -> Result<Answer> {
    let cells: Vec<CellEntry> = reports.iter().map(CellEntry::of).collect();
    let rows = rows_of(&cells);
    Answer::rows(Cells { cells }, &CellEntry::HEADERS, rows)
}

/// The payload of a writing verb under `--json`.
#[derive(Serialize)]
struct WriteReport {
    /// One result per operation, in the order the operations were given.
    operations: Vec<OperationEntry>,
    /// Which parts of the package the batch changed, added and removed.
    parts: PartsEntry,
    /// Where the result was written, or would have been under a dry run.
    output: String,
    /// Whether nothing was written because this was a dry run.
    dry_run: bool,
}

#[derive(Serialize)]
struct OperationEntry {
    /// The operand exactly as it was given, or `null` for an operation that
    /// names none: the calculation flag is the workbook's, not a cell's.
    target: Option<String>,
    /// The defined name the operand went through, in the package's own
    /// spelling, or `null` when the operand was an address.
    name: Option<String>,
    /// The sheet, in the package's own spelling, or `null` for an operation
    /// that names no cell.
    sheet: Option<String>,
    /// The cell in A1 form, or `null` for an operation that names no cell.
    cell: Option<String>,
    /// Both together, quoted as a reference: what `get` would accept back.
    /// `null` for an operation that names no cell.
    address: Option<String>,
    /// Whether the operation changed a byte. A write of the value already
    /// there did not.
    changed: bool,
}

#[derive(Serialize)]
struct PartsEntry {
    /// The parts whose bytes differ from the ones read, in path order.
    changed: Vec<String>,
    /// The parts the batch created.
    added: Vec<String>,
    /// The parts the batch removed.
    removed: Vec<String>,
}

/// What a writing verb answers: one row per operation, in the order the
/// operations were given. What the envelope says about the parts and the
/// output path has no row of its own: the stable interface is `--json`.
impl OperationEntry {
    fn of(operation: &OperationReport) -> Self {
        let at = operation.address.as_ref();
        OperationEntry {
            target: operation.target.clone(),
            name: operation.name.clone(),
            sheet: at.map(|address| address.sheet.clone()),
            cell: at.map(|address| address.cell.a1()),
            address: at.map(ToString::to_string),
            changed: operation.changed,
        }
    }
}

impl Entry<4> for OperationEntry {
    const HEADERS: [&'static str; 4] = ["TARGET", "NAME", "ADDRESS", "CHANGED"];

    /// An operation naming no cell leaves the address column empty, as a cell
    /// with no formula leaves the formula columns empty: a field is always the
    /// same field to `cut`. The sheet and the cell have no column of their own
    /// because the address column carries both.
    fn row(&self) -> [String; 4] {
        let OperationEntry {
            target,
            name,
            sheet: _,
            cell: _,
            address,
            changed,
        } = self;
        [
            target.clone().unwrap_or_default(),
            name.clone().unwrap_or_default(),
            address.clone().unwrap_or_default(),
            changed.to_string(),
        ]
    }
}

pub fn written(report: &Report) -> Result<Answer> {
    let operations: Vec<OperationEntry> =
        report.operations.iter().map(OperationEntry::of).collect();
    let rows = rows_of(&operations);
    let payload = WriteReport {
        operations,
        parts: PartsEntry {
            changed: report.parts.changed.clone(),
            added: report.parts.added.clone(),
            removed: report.parts.removed.clone(),
        },
        output: report.output.display().to_string(),
        dry_run: report.dry_run,
    };
    Answer::rows(payload, &OperationEntry::HEADERS, rows)
}

/// The columns of `calc` reading the flag.
const CALC_HEADERS: [&str; 1] = ["FULL CALC ON LOAD"];

/// The payload of `calc --json` reading the flag.
#[derive(Serialize)]
struct Calculation {
    /// Whether the workbook is flagged to recalculate fully when it opens.
    full_calc_on_load: bool,
}

/// What `calc` answers when it is only asked: whether the workbook is flagged
/// to recalculate fully on load. Setting the flag is a writing verb and
/// answers as one.
pub fn calculation(full_calc_on_load: bool) -> Result<Answer> {
    Answer::rows(
        Calculation { full_calc_on_load },
        &CALC_HEADERS,
        vec![vec![full_calc_on_load.to_string()]],
    )
}

/// The payload of `props get --json`.
#[derive(Serialize)]
struct Properties {
    properties: Vec<PropertyEntry>,
}

#[derive(Serialize)]
struct PropertyEntry {
    /// The name, as the package spells it.
    name: String,
    /// The variant type, as the package spells it: `lpwstr`, `i4`, `bool`,
    /// `filetime`, or whatever else is there. Not one of xlsplice's write
    /// types, because what is reported is what was found.
    #[serde(rename = "type")]
    kind: String,
    /// The value, typed as far as the variant allows. A moment comes back as
    /// the text the package holds, because JSON has no moment.
    value: serde_json::Value,
    /// The same value as the table spells it, kept beside the JSON one for
    /// the reason [`CellEntry::value_text`] is.
    #[serde(skip)]
    value_text: String,
}

impl PropertyEntry {
    /// The variant a property's value is is matched here and nowhere else,
    /// into both of the spellings the two shapes want.
    fn of(property: &Property) -> Self {
        let (value, value_text) = match &property.value {
            properties::Value::Text(text) | properties::Value::Moment(text) => {
                (serde_json::Value::String(text.clone()), text.clone())
            }
            properties::Value::Integer(number) => {
                (serde_json::Value::from(*number), number.to_string())
            }
            properties::Value::Real(number) => (number_json(*number), number_text(*number)),
            properties::Value::Bool(yes) => (serde_json::Value::Bool(*yes), yes.to_string()),
        };
        PropertyEntry {
            name: property.name.clone(),
            kind: property.variant.clone(),
            value,
            value_text,
        }
    }
}

impl Entry<3> for PropertyEntry {
    const HEADERS: [&'static str; 3] = ["NAME", "TYPE", "VALUE"];

    fn row(&self) -> [String; 3] {
        let PropertyEntry {
            name,
            kind,
            value: _,
            value_text,
        } = self;
        [name.clone(), kind.clone(), value_text.clone()]
    }
}

/// What `props get` answers: every custom document property, in the order the
/// package holds them.
pub fn properties(found: &[Property]) -> Result<Answer> {
    let properties: Vec<PropertyEntry> = found.iter().map(PropertyEntry::of).collect();
    let rows = rows_of(&properties);
    Answer::rows(Properties { properties }, &PropertyEntry::HEADERS, rows)
}

/// The payload of `diff --json`.
#[derive(Serialize)]
struct Comparison {
    /// Whether the two packages hold the same parts with the same bytes.
    identical: bool,
    /// Every part either package holds, and what became of it.
    parts: Vec<DifferenceEntry>,
}

#[derive(Serialize)]
struct DifferenceEntry {
    /// The part's path, as the container holds it.
    part: String,
    /// `identical`, `differs`, `added` or `removed`.
    status: &'static str,
}

impl DifferenceEntry {
    fn of(part: &PartStatus) -> Self {
        DifferenceEntry {
            part: part.part.clone(),
            status: part.status.as_str(),
        }
    }
}

impl Entry<2> for DifferenceEntry {
    const HEADERS: [&'static str; 2] = ["PART", "STATUS"];

    fn row(&self) -> [String; 2] {
        let DifferenceEntry { part, status } = self;
        [part.clone(), (*status).to_owned()]
    }
}

/// What `diff` answers: one row per part, the first package's parts in the
/// order it holds them and the second's own after them.
///
/// Whether the two are the same is in the envelope rather than in a row of its
/// own: it is a fact about the comparison and not about any part, and a table
/// of parts with one row that is not a part would be a table a caller has to
/// filter.
pub fn difference(found: &Difference) -> Result<Answer> {
    let parts: Vec<DifferenceEntry> = found.parts.iter().map(DifferenceEntry::of).collect();
    let rows = rows_of(&parts);
    Answer::rows(
        Comparison {
            identical: found.identical,
            parts,
        },
        &DifferenceEntry::HEADERS,
        rows,
    )
}

/// The payload of `help --json`.
#[derive(Serialize)]
struct HelpTopic {
    /// The topic asked for, as it is asked for.
    topic: &'static str,
    /// The whole of what it says, newlines and all.
    text: String,
}

/// What `help` answers: the topic's text, which is the same text in both
/// shapes because a topic is prose rather than a list of things.
pub fn topic(topic: Topic) -> Result<Answer> {
    let text = topic.text();
    Answer::line(
        HelpTopic {
            topic: topic.as_str(),
            text: text.clone(),
        },
        text.trim_end(),
    )
}

/// The payload of `version --json`.
#[derive(Serialize)]
struct Version {
    version: &'static str,
}

/// What `version` answers: the version of this build, which is the version of
/// the crate the library is part of.
pub fn version() -> Result<Answer> {
    let version = env!("CARGO_PKG_VERSION");
    Answer::line(Version { version }, format!("xlsplice {version}"))
}

/// A number as JSON.
///
/// An integral value is written without a decimal point, which is the form
/// the package holds it in and the form a write puts back, so a value read
/// out of one cell is the value written into another. Beyond the range where
/// a double counts in whole numbers there is nothing to be gained by it, so
/// the plain form takes over. A cell reports only finite numbers, so the last
/// arm is a guard the type cannot carry rather than a case that arises: an
/// answer says null sooner than it panics.
fn number_json(number: f64) -> serde_json::Value {
    const EXACT: f64 = 9_007_199_254_740_992.0;
    if number.fract() == 0.0 && number.abs() <= EXACT {
        return serde_json::Value::Number((number as i64).into());
    }
    serde_json::Number::from_f64(number).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    const HEADERS: [&str; 2] = ["NAME", "STATE"];

    #[derive(Serialize)]
    struct Payload {
        sheets: u32,
    }

    fn row(fields: &[&str]) -> Vec<String> {
        fields.iter().map(|field| (*field).to_owned()).collect()
    }

    #[test]
    fn a_row_that_is_not_as_wide_as_the_headers_is_refused() {
        let narrow = Answer::rows(Payload { sheets: 1 }, &HEADERS, vec![row(&["Data"])]);
        let wide = Answer::rows(
            Payload { sheets: 1 },
            &HEADERS,
            vec![row(&["Data", "visible", "extra"])],
        );

        for refused in [narrow, wide] {
            let err = refused.expect_err("a row of the wrong width must be refused");
            assert_eq!(err.code(), ErrorCode::Internal);
        }
    }

    #[test]
    fn the_offending_row_is_named_by_its_place_in_the_answer() {
        let err = Answer::rows(
            Payload { sheets: 2 },
            &HEADERS,
            vec![row(&["Data", "visible"]), row(&["Notes"])],
        )
        .expect_err("the second row is a field short");

        assert!(err.message().contains("row 1"), "{}", err.message());
    }

    #[test]
    fn rows_of_the_right_width_and_no_rows_at_all_are_both_answers() {
        assert!(Answer::rows(Payload { sheets: 0 }, &HEADERS, Vec::new()).is_ok());
        assert!(
            Answer::rows(
                Payload { sheets: 1 },
                &HEADERS,
                vec![row(&["Data", "visible"])]
            )
            .is_ok()
        );
    }

    #[test]
    fn a_payload_that_is_not_an_object_has_nowhere_to_go_in_the_envelope() {
        let err = Answer::line("just a string", "text").expect_err("a string is not an object");

        assert_eq!(err.code(), ErrorCode::Internal);
    }

    #[test]
    fn an_integral_number_carries_no_decimal_point_and_a_fractional_one_does() {
        assert_eq!(number_json(1.0).to_string(), "1");
        assert_eq!(number_json(-3.0).to_string(), "-3");
        assert_eq!(number_json(2.5).to_string(), "2.5");
    }

    #[test]
    fn a_number_too_large_to_count_in_whole_numbers_stays_a_double() {
        let huge = number_json(1e300);

        assert!(huge.is_f64(), "{huge} must stay a double");
        assert_eq!(huge.as_f64(), Some(1e300));
    }

    /// A cell report holding `value` and nothing else of interest.
    fn holding(value: Value) -> CellReport {
        CellReport {
            target: "Sheet1!A1".to_owned(),
            name: None,
            address: crate::reference::Address {
                sheet: "Sheet1".to_owned(),
                cell: crate::reference::Cell::parse("A1").expect("the test asks for a cell"),
            },
            kind: crate::worksheet::StoredType::Number,
            value,
            raw: None,
            formula: None,
            style: None,
        }
    }

    /// The value column of a cell's row.
    fn value_column(value: Value) -> String {
        CellEntry::of(&holding(value)).row()[4].clone()
    }

    #[test]
    fn a_value_in_a_row_is_written_the_way_a_person_would_read_it() {
        assert_eq!(value_column(Value::Number(1.0)), "1");
        assert_eq!(value_column(Value::Number(2.5)), "2.5");
        assert_eq!(value_column(Value::Bool(true)), "true");
        assert_eq!(value_column(Value::Text("hello".to_owned())), "hello");
        assert_eq!(value_column(Value::Empty), "");
    }

    /// The two shapes spell a large non-integral number differently, and both
    /// spellings are the ones they have always had: JSON writes an exponent,
    /// and the row writes the number out. A row taken from the payload's JSON
    /// would move three hundred bytes of a caller's output, which is why the
    /// entry carries both rather than deriving one from the other.
    #[test]
    fn a_number_too_large_for_a_row_to_shorten_is_still_written_out_in_full() {
        let row = value_column(Value::Number(1e300));

        assert_eq!(row, 1e300_f64.to_string());
        assert_eq!(row.len(), 301, "one digit and three hundred noughts");
        assert!(!row.contains('e'), "the row writes no exponent: {row}");
        assert_eq!(
            number_json(1e300).to_string(),
            "1e+300",
            "and the envelope writes nothing else"
        );
    }
}
