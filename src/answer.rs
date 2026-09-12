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

use serde::Serialize;

use crate::batch::{OperationReport, Report};
use crate::cells::{CellReport, Value};
use crate::error::{Error, Result};
use crate::workbook::{DefinedName, Resolved, Scope, Sheet, Workbook};

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

/// The columns of `sheets`.
const SHEET_HEADERS: [&str; 2] = ["NAME", "STATE"];

/// The columns of `names`.
const NAME_HEADERS: [&str; 5] = ["NAME", "SCOPE", "REFERS TO", "ANCHOR", "REASON"];

/// The columns of `get`.
const CELL_HEADERS: [&str; 11] = [
    "TARGET", "NAME", "ADDRESS", "TYPE", "VALUE", "RAW", "STYLE", "FORMULA", "ROLE", "RANGE",
    "GROUP",
];

/// The columns of `set`, and of every writing verb after it.
const WRITE_HEADERS: [&str; 4] = ["TARGET", "NAME", "ADDRESS", "CHANGED"];

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

/// What `sheets` answers: the package's sheets, in workbook order.
pub fn sheets(workbook: &Workbook) -> Result<Answer> {
    let payload = Sheets {
        sheets: workbook
            .sheets()
            .iter()
            .map(|sheet| SheetEntry {
                name: sheet.name.clone(),
                state: sheet.state.as_str(),
            })
            .collect(),
    };
    let rows = workbook.sheets().iter().map(sheet_row).collect();
    Answer::rows(payload, &SHEET_HEADERS, rows)
}

fn sheet_row(sheet: &Sheet) -> Vec<String> {
    vec![sheet.name.clone(), sheet.state.as_str().to_owned()]
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

/// What `names` answers: the defined names, in the order the package declares
/// them.
pub fn names(workbook: &Workbook) -> Result<Answer> {
    let payload = Names {
        names: workbook.defined_names().iter().map(name_entry).collect(),
    };
    let rows = workbook.defined_names().iter().map(name_row).collect();
    Answer::rows(payload, &NAME_HEADERS, rows)
}

fn name_entry(name: &DefinedName) -> NameEntry {
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
    NameEntry {
        name: name.name.clone(),
        scope: scope_kind(&name.scope),
        scope_sheet: match &name.scope {
            Scope::Workbook => None,
            Scope::Sheet(sheet) => Some(sheet.clone()),
        },
        refers_to: name.refers_to.clone(),
        anchor,
        reason,
    }
}

/// A name's row. Every row has all five fields, one of the last two always
/// empty, so a field is always the same field to `cut`.
fn name_row(name: &DefinedName) -> Vec<String> {
    let (anchor, reason) = match &name.resolved {
        Resolved::Anchor(address) => (address.to_string(), String::new()),
        Resolved::Unresolvable(why) => (String::new(), why.as_str().to_owned()),
    };
    vec![
        name.name.clone(),
        match &name.scope {
            Scope::Workbook => "workbook".to_owned(),
            Scope::Sheet(sheet) => sheet.clone(),
        },
        name.refers_to.clone(),
        anchor,
        reason,
    ]
}

/// How the envelope names a scope, apart from the sheet it names.
fn scope_kind(scope: &Scope) -> &'static str {
    match scope {
        Scope::Workbook => "workbook",
        Scope::Sheet(_) => "sheet",
    }
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

/// What `get` answers: one cell per target, in the order the targets were
/// given.
pub fn cells(reports: &[CellReport]) -> Result<Answer> {
    let payload = Cells {
        cells: reports.iter().map(cell_entry).collect(),
    };
    let rows = reports.iter().map(cell_row).collect();
    Answer::rows(payload, &CELL_HEADERS, rows)
}

fn cell_entry(report: &CellReport) -> CellEntry {
    CellEntry {
        target: report.target.clone(),
        name: report.name.clone(),
        sheet: report.address.sheet.clone(),
        cell: report.address.cell.a1(),
        address: report.address.to_string(),
        kind: report.kind.as_str(),
        value: match &report.value {
            Value::Number(number) => number_json(*number),
            Value::Text(text) => serde_json::Value::String(text.clone()),
            Value::Bool(yes) => serde_json::Value::Bool(*yes),
            Value::Empty => serde_json::Value::Null,
        },
        raw: report.raw.clone(),
        formula: report.formula.as_ref().map(|formula| FormulaEntry {
            text: formula.text.clone(),
            role: formula.role.as_str(),
            range: formula.range.clone(),
            group: formula.group,
        }),
        style: report.style,
    }
}

/// A cell's row. Every row has all eleven fields, whatever the cell holds, so
/// a field is always the same field to `cut`; the formula's three trail at the
/// end because most cells have none.
fn cell_row(report: &CellReport) -> Vec<String> {
    let (text, role, range, group) = match &report.formula {
        None => (String::new(), String::new(), String::new(), String::new()),
        Some(formula) => (
            formula.text.clone(),
            formula.role.as_str().to_owned(),
            formula.range.clone().unwrap_or_default(),
            formula.group.map(|si| si.to_string()).unwrap_or_default(),
        ),
    };
    vec![
        report.target.clone(),
        report.name.clone().unwrap_or_default(),
        report.address.to_string(),
        report.kind.as_str().to_owned(),
        value_text(&report.value),
        report.raw.clone().unwrap_or_default(),
        report.style.map(|s| s.to_string()).unwrap_or_default(),
        text,
        role,
        range,
        group,
    ]
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
pub fn written(report: &Report) -> Result<Answer> {
    let payload = WriteReport {
        operations: report.operations.iter().map(operation_entry).collect(),
        parts: PartsEntry {
            changed: report.parts.changed.clone(),
            added: report.parts.added.clone(),
            removed: report.parts.removed.clone(),
        },
        output: report.output.display().to_string(),
        dry_run: report.dry_run,
    };
    let rows = report.operations.iter().map(operation_row).collect();
    Answer::rows(payload, &WRITE_HEADERS, rows)
}

fn operation_entry(operation: &OperationReport) -> OperationEntry {
    OperationEntry {
        target: operation.target.clone(),
        name: operation.name.clone(),
        sheet: operation.address.sheet.clone(),
        cell: operation.address.cell.a1(),
        address: operation.address.to_string(),
        changed: operation.changed,
    }
}

fn operation_row(operation: &OperationReport) -> Vec<String> {
    vec![
        operation.target.clone(),
        operation.name.clone().unwrap_or_default(),
        operation.address.to_string(),
        operation.changed.to_string(),
    ]
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

/// A value as one field of a row. A number is written in the shortest form
/// that reads back as itself, and a boolean as the word Excel uses.
fn value_text(value: &Value) -> String {
    match value {
        Value::Number(number) => number.to_string(),
        Value::Text(text) => text.clone(),
        Value::Bool(yes) => yes.to_string(),
        Value::Empty => String::new(),
    }
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

    #[test]
    fn a_value_in_a_row_is_written_the_way_a_person_would_read_it() {
        assert_eq!(value_text(&Value::Number(1.0)), "1");
        assert_eq!(value_text(&Value::Number(2.5)), "2.5");
        assert_eq!(value_text(&Value::Bool(true)), "true");
        assert_eq!(value_text(&Value::Text("hello".to_owned())), "hello");
        assert_eq!(value_text(&Value::Empty), "");
    }
}
