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

use xlsplice::workbook::{DefinedName, Resolved, Scope, Sheet, Workbook};

/// The columns of `sheets`.
pub const SHEET_HEADERS: [&str; 2] = ["NAME", "STATE"];

/// The columns of `names`.
pub const NAME_HEADERS: [&str; 5] = ["NAME", "SCOPE", "REFERS TO", "ANCHOR", "REASON"];

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
