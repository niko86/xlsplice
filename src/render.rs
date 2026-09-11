//! What the read verbs put on stdout, in both shapes.
//!
//! The library answers in its own types; the shape of the answer on the wire
//! is decided here, in the binary, next to the envelope it goes into. A row of
//! the table and a member of the payload carry the same facts, so a caller
//! reading either sees the same package.
//!
//! The JSON shape is the stable one and may only grow: a field is never
//! removed or retyped, and a field that does not apply is `null` rather than
//! absent, so every member of a list has the same keys.

use serde::Serialize;

use xlsplice::batch::{OperationReport, Report};
use xlsplice::cells::{CellReport, Value};
use xlsplice::workbook::{DefinedName, Resolved, Scope, Sheet, Workbook};

/// The columns of `sheets`.
pub const SHEET_HEADERS: [&str; 2] = ["NAME", "STATE"];

/// The columns of `names`.
pub const NAME_HEADERS: [&str; 5] = ["NAME", "SCOPE", "REFERS TO", "ANCHOR", "REASON"];

/// The columns of `set`, and of every writing verb after it.
pub const WRITE_HEADERS: [&str; 4] = ["TARGET", "NAME", "ADDRESS", "CHANGED"];

/// The columns of `get`.
pub const CELL_HEADERS: [&str; 11] = [
    "TARGET", "NAME", "ADDRESS", "TYPE", "VALUE", "RAW", "STYLE", "FORMULA", "ROLE", "RANGE",
    "GROUP",
];

/// The payload of `sheets --json`.
#[derive(Serialize)]
pub struct Sheets {
    sheets: Vec<SheetEntry>,
}

#[derive(Serialize)]
struct SheetEntry {
    /// The sheet's name, in the package's own spelling.
    name: String,
    /// `visible`, `hidden` or `veryHidden`.
    state: &'static str,
}

/// The payload of `names --json`.
#[derive(Serialize)]
pub struct Names {
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

/// The payload of `sheets`.
pub fn sheets(workbook: &Workbook) -> Sheets {
    Sheets {
        sheets: workbook
            .sheets()
            .iter()
            .map(|sheet| SheetEntry {
                name: sheet.name.clone(),
                state: sheet.state.as_str(),
            })
            .collect(),
    }
}

/// The rows of `sheets`, in workbook order.
pub fn sheet_rows(workbook: &Workbook) -> Vec<Vec<String>> {
    workbook.sheets().iter().map(sheet_row).collect()
}

fn sheet_row(sheet: &Sheet) -> Vec<String> {
    vec![sheet.name.clone(), sheet.state.as_str().to_owned()]
}

/// The payload of `names`.
pub fn names(workbook: &Workbook) -> Names {
    Names {
        names: workbook.defined_names().iter().map(name_entry).collect(),
    }
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

/// The rows of `names`, in the order the package declares them.
pub fn name_rows(workbook: &Workbook) -> Vec<Vec<String>> {
    workbook.defined_names().iter().map(name_row).collect()
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
pub struct Cells {
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

/// The payload of `get`.
pub fn cells(reports: &[CellReport]) -> Cells {
    Cells {
        cells: reports.iter().map(cell_entry).collect(),
    }
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

/// The rows of `get`, one per target, in the order the targets were given.
pub fn cell_rows(reports: &[CellReport]) -> Vec<Vec<String>> {
    reports.iter().map(cell_row).collect()
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
pub struct WriteReport {
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

/// The payload of a writing verb.
pub fn written(report: &Report) -> WriteReport {
    WriteReport {
        operations: report.operations.iter().map(operation_entry).collect(),
        parts: PartsEntry {
            changed: report.parts.changed.clone(),
            added: report.parts.added.clone(),
            removed: report.parts.removed.clone(),
        },
        output: report.output.display().to_string(),
        dry_run: report.dry_run,
    }
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

/// The rows of a writing verb, one per operation, in the order the operations
/// were given. What the envelope says about the parts and the output path has
/// no row of its own: the stable interface is `--json`.
pub fn written_rows(report: &Report) -> Vec<Vec<String>> {
    report
        .operations
        .iter()
        .map(|operation| {
            vec![
                operation.target.clone(),
                operation.name.clone().unwrap_or_default(),
                operation.address.to_string(),
                operation.changed.to_string(),
            ]
        })
        .collect()
}

/// A number as JSON.
///
/// An integral value is written without a decimal point, which is the form
/// the package holds it in and the form a write puts back, so a value read
/// out of one cell is the value written into another. Beyond the range where
/// a double counts in whole numbers there is nothing to be gained by it, so
/// the plain form takes over. A cell reports only finite numbers, so the last
/// arm is a guard the type cannot carry rather than a case that arises: a
/// renderer answers with null sooner than it panics.
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
    use xlsplice::reference::{Address, Cell};
    use xlsplice::worksheet::StoredType;

    fn report(value: Value) -> CellReport {
        CellReport {
            target: "Inputs!A1".to_owned(),
            name: None,
            address: Address {
                sheet: "Inputs".to_owned(),
                cell: Cell::parse("A1").expect("the test asks for a cell"),
            },
            kind: StoredType::Number,
            value,
            raw: Some("1".to_owned()),
            formula: None,
            style: Some(0),
        }
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

    #[test]
    fn every_row_has_one_field_per_column_whatever_the_cell_holds() {
        for value in [Value::Number(1.0), Value::Empty] {
            assert_eq!(cell_row(&report(value)).len(), CELL_HEADERS.len());
        }
    }
}
