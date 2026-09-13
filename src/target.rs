//! Targets: from what a caller pointed at to the cell and the part it names.
//!
//! A target is one operand naming a cell, by address or by defined name.
//! Resolving one is the same question whichever way it was asked and whoever
//! is asking, so the rule lives here once and both paths come through it: the
//! read path in [`crate::cells`], and the write path in [`crate::batch`].
//! Nothing above this module spells the address rule out again.
//!
//! [`crate::reference`] says what a target looks like, [`crate::workbook`]
//! says which sheet or defined name it lands on, and
//! [`crate::worksheet::part_of_sheet`] says which part that sheet's cells sit
//! in. Nothing here opens a part: a [`Resolution`] is an answer about where a
//! cell is, not about what is in it.

use std::path::Path;

use crate::error::{Error, Result};
use crate::package::Package;
use crate::reference::{Address, Target};
use crate::relationships::Relationships;
use crate::workbook::{DefinedName, NoAnchor, Resolved, Scope, Workbook};
use crate::worksheet::part_of_sheet;

/// One target, resolved to the cell and the part it names.
///
/// Reads and writes resolve a target by the same rule, so both come through
/// [`resolve`] and neither has a spelling of the address rule of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The target exactly as it was given.
    pub target: String,
    /// The defined name it went through, in the package's own spelling, or
    /// `None` when the target was an address.
    pub name: Option<String>,
    /// The cell it resolved to, in the package's own spelling.
    pub address: Address,
    /// The worksheet part that cell sits in.
    pub part: String,
}

/// Resolve one target to the cell it names and the part that cell sits in.
pub fn resolve(
    package: &Package,
    rels: &Relationships,
    workbook: &Workbook,
    target: &str,
) -> Result<Resolution> {
    let file = package.path();
    let (name, address) = match Target::parse(target) {
        Target::Address { sheet, cell } => (
            None,
            Address {
                sheet: sheet_named(file, workbook, &sheet)?,
                cell,
            },
        ),
        Target::SheetName { sheet, name } => {
            let scope = Scope::Sheet(sheet_named(file, workbook, &sheet)?);
            let defined = workbook
                .name_in_scope(&name, &scope)
                .ok_or_else(|| no_such_name(file, workbook, &name, &scope))?;
            (Some(defined.name.clone()), anchor_of(defined)?)
        }
        Target::Name(name) => {
            let defined = workbook
                .name_in_scope(&name, &Scope::Workbook)
                .ok_or_else(|| no_such_name(file, workbook, &name, &Scope::Workbook))?;
            (Some(defined.name.clone()), anchor_of(defined)?)
        }
    };
    Ok(Resolution {
        part: part_of_sheet(package, rels, workbook, &address.sheet)?,
        target: target.to_owned(),
        name,
        address,
    })
}

/// The package's own spelling of the sheet called `name`.
fn sheet_named(file: &Path, workbook: &Workbook, name: &str) -> Result<String> {
    workbook
        .sheet_named(name)
        .map(|sheet| sheet.name.clone())
        .ok_or_else(|| {
            Error::not_found(format!(
                "no sheet named '{name}' in {}; the package has: {}",
                file.display(),
                list(workbook.sheets().iter().map(|sheet| sheet.name.as_str()))
            ))
        })
}

/// The cell a defined name stands for.
///
/// A name that holds a literal or a formula is refused rather than not found,
/// because the name is there: what it holds is the problem, so the message
/// carries what it refers to. A name whose reference points at a sheet the
/// package does not have is a missing sheet like any other.
fn anchor_of(defined: &DefinedName) -> Result<Address> {
    let DefinedName {
        name, refers_to, ..
    } = defined;
    match &defined.resolved {
        Resolved::Anchor(address) => Ok(address.clone()),
        Resolved::Unresolvable(NoAnchor::UnknownSheet) => Err(Error::not_found(format!(
            "the defined name '{name}' refers to {refers_to}, whose sheet is not in this package"
        ))),
        Resolved::Unresolvable(why) => Err(Error::refused(format!(
            "the defined name '{name}' refers to {refers_to}, {} rather than a cell; \
             give an address instead.",
            match why {
                NoAnchor::RefError => "a deleted reference",
                NoAnchor::Constant => "a constant",
                _ => "a formula",
            }
        ))),
    }
}

fn no_such_name(file: &Path, workbook: &Workbook, name: &str, scope: &Scope) -> Error {
    let where_ = match scope {
        Scope::Workbook => "scoped to the workbook".to_owned(),
        Scope::Sheet(sheet) => format!("scoped to sheet '{sheet}'"),
    };
    Error::not_found(format!(
        "no defined name '{name}' {where_}; {} has: {}",
        file.display(),
        list(workbook.names_in_scope(scope).into_iter())
    ))
}

/// Names in a message, or a plain statement that there are none.
fn list<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let listed: Vec<&str> = names.collect();
    if listed.is_empty() {
        "none".to_owned()
    } else {
        listed.join(", ")
    }
}

/// Each part the resolved cells sit in, once, in the order first named.
///
/// A read resolves every target before it opens a part, so this is what says
/// which parts it has to open. A write does not ask: each of its operations
/// names the parts it wants edited, and the batch gathers those.
pub fn parts_of<'a>(resolved: impl IntoIterator<Item = &'a Resolution>) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for at in resolved {
        if !parts.contains(&at.part) {
            parts.push(at.part.clone());
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Cell;

    #[test]
    fn each_part_the_targets_land_in_is_listed_once_in_the_order_first_named() {
        let landing = |part: &str| Resolution {
            target: String::new(),
            name: None,
            address: Address {
                sheet: "Inputs".to_owned(),
                cell: Cell::parse("A1").expect("the test asks for a cell"),
            },
            part: part.to_owned(),
        };
        let resolved = [
            landing("sheet2.xml"),
            landing("sheet1.xml"),
            landing("sheet2.xml"),
        ];

        assert_eq!(parts_of(resolved.iter()), ["sheet2.xml", "sheet1.xml"]);
    }

    #[test]
    fn a_list_of_nothing_says_so_rather_than_trailing_off() {
        assert_eq!(list(std::iter::empty()), "none");
        assert_eq!(list(["a", "b"].into_iter()), "a, b");
    }
}
