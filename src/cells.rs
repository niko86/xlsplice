//! Reading cells: from the targets a caller named to what the package holds
//! at each of them.
//!
//! This is where the pieces meet. [`crate::target`] says which cell and part
//! an operand names, [`crate::worksheet`] says what the cell element holds,
//! and [`crate::strings`] turns a shared-string index into text. Nothing
//! below this module knows a target was ever named.
//!
//! Every target is resolved before any part is read, so one naming a sheet
//! the package does not have fails before a single worksheet is parsed. A
//! part is then read once however many targets land on it, and the shared
//! string table, which may be the largest part in the package, is read only
//! if some cell turns out to be a shared string.

use std::io::{Read, Seek};

use roxmltree::Document;

use crate::error::{Error, Result};
use crate::package::Package;
use crate::reference::Address;
use crate::relationships::Relationships;
use crate::strings::SharedStrings;
use crate::target::{Resolution, parts_of, resolve};
use crate::workbook::Workbook;
use crate::worksheet::{Formula, Found, Stored, StoredType, Worksheet};

/// A cell's value, typed by what the cell stores.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A number, as stored, and always finite: a number cell whose text is
    /// not a number is unreadable rather than reported as something else.
    Number(f64),
    /// Text: a shared or inline string with its rich-text runs concatenated,
    /// a formula's cached string, an error value, or a stored ISO date.
    Text(String),
    /// A boolean.
    Bool(bool),
    /// The cell stores no value.
    Empty,
}

/// One target, and what the package holds at the cell it named.
#[derive(Debug, Clone, PartialEq)]
pub struct CellReport {
    /// The target exactly as it was given.
    pub target: String,
    /// The defined name it went through, in the package's own spelling, or
    /// `None` when the target was an address.
    pub name: Option<String>,
    /// The cell it resolved to, in the package's own spelling.
    pub address: Address,
    /// The type the value is stored as.
    pub kind: StoredType,
    /// The value, typed.
    pub value: Value,
    /// The exact stored text; for a shared-string cell, the string index.
    /// An inline string stores its text itself, so its raw text is that
    /// text: there is no other stored form of it to report.
    pub raw: Option<String>,
    /// The cell's formula, if it has one.
    pub formula: Option<Formula>,
    /// The style index, or `None` for a cell the part does not hold.
    pub style: Option<u32>,
}

/// Read one cell per target, in the order the targets were given.
///
/// Any target that cannot be resolved fails the whole read, because a caller
/// asking for several cells is asking about one package and a partial answer
/// would have to be told apart from a whole one.
pub fn read<R: Read + Seek>(
    package: &mut Package<R>,
    workbook: &Workbook,
    targets: &[String],
) -> Result<Vec<CellReport>> {
    let rels = Relationships::read(package, workbook.part())?;
    let resolved: Vec<Resolution> = targets
        .iter()
        .map(|target| resolve(package, &rels, workbook, target))
        .collect::<Result<_>>()?;

    let stored = read_cells(package, &resolved)?;
    // The shared string table may be the largest part in the package, so it
    // is read once, and only if some cell turned out to be a shared string.
    let wanted = stored
        .iter()
        .flatten()
        .any(|cell| cell.kind == StoredType::Shared);
    let strings = match wanted {
        true => SharedStrings::read(package, &rels)?,
        false => SharedStrings::default(),
    };
    resolved
        .into_iter()
        .zip(stored)
        .map(|(at, stored)| report(&strings, at, stored))
        .collect()
}

/// The cell element behind each resolved target, `None` where the row is
/// there and the cell is not.
///
/// Each part is parsed once however many targets landed on it, so reading a
/// column of cells costs one parse rather than one per cell.
fn read_cells<R: Read + Seek>(
    package: &mut Package<R>,
    resolved: &[Resolution],
) -> Result<Vec<Option<Stored>>> {
    let mut stored: Vec<Option<Stored>> = vec![None; resolved.len()];
    for part in parts_of(resolved.iter()) {
        let xml = package.read_part_text(&part)?;
        let document = Document::parse(xml)
            .map_err(|err| Error::unreadable(format!("not valid XML: {err}")).within(&part))?;
        let sheet = Worksheet::of(&document).map_err(|err| err.within(&part))?;
        for (index, at) in resolved.iter().enumerate() {
            if at.part != part {
                continue;
            }
            stored[index] = match sheet
                .cell(at.address.cell)
                .map_err(|err| err.within(&part))?
            {
                Found::Cell(cell) => Some(cell),
                Found::Absent => None,
            };
        }
    }
    Ok(stored)
}

fn report(strings: &SharedStrings, at: Resolution, stored: Option<Stored>) -> Result<CellReport> {
    let Some(stored) = stored else {
        return Ok(CellReport {
            target: at.target,
            name: at.name,
            address: at.address,
            kind: StoredType::Empty,
            value: Value::Empty,
            raw: None,
            formula: None,
            style: None,
        });
    };
    let value = value_of(&stored, &at.address, strings)?;
    Ok(CellReport {
        target: at.target,
        name: at.name,
        address: at.address,
        kind: stored.kind,
        value,
        raw: stored.raw,
        formula: stored.formula,
        style: Some(stored.style),
    })
}

/// The typed value a cell's stored text stands for.
///
/// A cell whose text does not say what its type promises is a part
/// disagreeing with itself, and is unreadable rather than reported as some
/// other type: a caller switching on `type` must be able to trust `value`.
fn value_of(stored: &Stored, at: &Address, strings: &SharedStrings) -> Result<Value> {
    let raw = stored.raw.as_deref().unwrap_or_default().trim();
    Ok(match stored.kind {
        StoredType::Empty => Value::Empty,
        StoredType::Number => Value::Number(
            raw.parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or_else(|| {
                    Error::unreadable(format!(
                        "cell {at} is a number holding '{raw}', which is not a number"
                    ))
                })?,
        ),
        // A boolean is spelled the way the schema spells one, which is `1` or
        // `0`; `true` and `false` are the same value in that spelling.
        StoredType::Bool => Value::Bool(match raw {
            "1" | "true" => true,
            "0" | "false" => false,
            other => {
                return Err(Error::unreadable(format!(
                    "cell {at} is a boolean holding '{other}', which is not a boolean"
                )));
            }
        }),
        StoredType::Shared => {
            let index: usize = raw.parse().map_err(|_| {
                Error::unreadable(format!(
                    "cell {at} holds shared string index '{raw}', which is not an index"
                ))
            })?;
            Value::Text(
                strings
                    .get(index)
                    .ok_or_else(|| {
                        Error::unreadable(format!(
                            "cell {at} holds shared string {index}, and the table has {}",
                            strings.len()
                        ))
                    })?
                    .to_owned(),
            )
        }
        // Everything else is the text the cell holds, whitespace and all, so
        // the stored text rather than the trimmed one.
        StoredType::FormulaString
        | StoredType::InlineString
        | StoredType::ErrorValue
        | StoredType::Date => Value::Text(stored.raw.clone().unwrap_or_default()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::reference::Cell;

    fn at() -> Address {
        Address {
            sheet: "Inputs".to_owned(),
            cell: Cell::parse("A1").expect("the test asks for a cell"),
        }
    }

    fn stored(kind: StoredType, raw: &str) -> Stored {
        Stored {
            kind,
            raw: Some(raw.to_owned()),
            formula: None,
            style: 0,
        }
    }

    fn value(kind: StoredType, raw: &str) -> Result<Value> {
        value_of(&stored(kind, raw), &at(), &SharedStrings::default())
    }

    fn rejects(kind: StoredType, raw: &str) {
        let err = value(kind, raw).expect_err(raw);
        assert_eq!(err.code(), ErrorCode::Unreadable, "{raw}");
        assert!(err.message().contains("Inputs!A1"), "{}", err.message());
    }

    #[test]
    fn a_number_cell_holding_no_number_is_unreadable_rather_than_reported_as_text() {
        // `type` and `value` are read together, so a caller switching on the
        // one must be able to trust the other.
        for raw in ["", "hello", "1,5", "inf", "NaN"] {
            rejects(StoredType::Number, raw);
        }
    }

    #[test]
    fn a_boolean_cell_takes_both_spellings_the_schema_allows_and_nothing_else() {
        assert_eq!(value(StoredType::Bool, "1").unwrap(), Value::Bool(true));
        assert_eq!(value(StoredType::Bool, "true").unwrap(), Value::Bool(true));
        assert_eq!(value(StoredType::Bool, "0").unwrap(), Value::Bool(false));
        assert_eq!(
            value(StoredType::Bool, "false").unwrap(),
            Value::Bool(false)
        );
        for raw in ["", "yes", "2"] {
            rejects(StoredType::Bool, raw);
        }
    }

    #[test]
    fn a_shared_string_index_the_table_does_not_hold_is_unreadable() {
        rejects(StoredType::Shared, "7");
        rejects(StoredType::Shared, "one");
    }

    #[test]
    fn a_number_is_read_from_the_text_around_its_whitespace() {
        assert_eq!(
            value(StoredType::Number, " 2.5 ").unwrap(),
            Value::Number(2.5)
        );
    }

    #[test]
    fn text_keeps_the_whitespace_the_cell_stored_with_it() {
        assert_eq!(
            value(StoredType::InlineString, "  padded  ").unwrap(),
            Value::Text("  padded  ".to_owned()),
            "trimming is for reading a number, not for the text itself"
        );
    }

    #[test]
    fn a_cell_holding_no_value_is_empty_whatever_its_type_says() {
        let none = Stored {
            kind: StoredType::Empty,
            raw: None,
            formula: None,
            style: 3,
        };

        assert_eq!(
            value_of(&none, &at(), &SharedStrings::default()).unwrap(),
            Value::Empty
        );
    }
}
