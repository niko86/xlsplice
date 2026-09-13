//! The calc chain: the order Excel last calculated a workbook's formulas in,
//! and taking a cell out of it.
//!
//! `xl/calcChain.xml` is a cache. It holds one entry per formula cell, in the
//! order they were calculated, so that Excel can start from where it left off
//! rather than working the dependency graph out again. Nothing depends on it
//! being right, but an entry naming a cell that no longer holds a formula is
//! an inconsistency Excel may complain about, so a formula replaced takes its
//! entry with it.
//!
//! An entry names its sheet by the number the workbook gives that sheet, and
//! an entry that names none takes the one before it — which is why removing an
//! entry is not simply cutting it out: the entry after it may have been
//! leaning on the one that went.

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::reference::Cell;
use crate::splice::{Element, Splice};
use crate::xml::children;

/// Where Excel puts the calc chain when the relationships do not say.
pub const CONVENTIONAL_PART: &str = "xl/calcChain.xml";

/// The relationship type a calc chain is reached by.
pub const CALC_CHAIN: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain";

/// What taking one cell out of the chain comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The splices that take the entry out. Empty where the chain holds no
    /// entry for that cell, which a stale chain may well not.
    pub splices: Vec<Splice>,
    /// How many entries the chain would hold afterwards. A chain down to none
    /// is not a chain, and the caller takes the part out of the package.
    pub remaining: usize,
}

/// The splices that take the entry for `cell` on the sheet numbered `sheet` out
/// of the chain whose text is `xml`.
///
/// An entry that names no sheet is on the sheet the entry before it named, so
/// removing an entry that names one, where the next does not, would move the
/// next entry to another sheet. It is given the number it was leaning on.
pub fn without(xml: &str, sheet: u32, cell: Cell) -> Result<Removal> {
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("the calc chain is not valid XML: {err}")))?;
    let root = document.root_element();
    if root.tag_name().name() != "calcChain" {
        return Err(Error::unreadable(format!(
            "the calc chain part's root element is <{}>, not <calcChain>",
            root.tag_name().name()
        )));
    }

    let entries: Vec<Node> = children(root, "c").collect();
    let mut on: Vec<u32> = Vec::with_capacity(entries.len());
    let mut running = 0;
    for entry in &entries {
        running = match entry.attribute("i") {
            None => running,
            Some(number) => number.parse().map_err(|_| {
                Error::unreadable(format!(
                    "a calc chain entry is on sheet '{number}', which is not a sheet number"
                ))
            })?,
        };
        on.push(running);
    }

    let wanted = entries.iter().zip(&on).position(|(entry, sheet_of)| {
        *sheet_of == sheet && entry.attribute("r") == Some(cell.a1().as_str())
    });
    let Some(wanted) = wanted else {
        return Ok(Removal {
            splices: Vec::new(),
            remaining: entries.len(),
        });
    };

    let mut splices = vec![Splice::new(entries[wanted].range(), "")];
    // The entry after this one may have been taking its sheet from it. It
    // keeps the sheet it had, spelled out, rather than inheriting whatever now
    // precedes it.
    if let Some(next) = entries.get(wanted + 1)
        && next.attribute("i").is_none()
    {
        let element = Element::of(*next, xml)?;
        splices.extend(element.attribute_splice("i", Some(&on[wanted + 1].to_string()), &["r"]));
    }
    Ok(Removal {
        splices,
        remaining: entries.len() - 1,
    })
}

/// How many entries the chain whose text is `xml` holds.
///
/// Asked of the chain as a whole batch leaves it: two operations may each take
/// an entry out, and neither can see the other's, so whether the last one has
/// gone is a question about the part rather than about any one of them.
pub fn entries_in(xml: &str) -> Result<usize> {
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("the calc chain is not valid XML: {err}")))?;
    Ok(children(document.root_element(), "c").count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splice;

    const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    fn chain(entries: &str) -> String {
        format!(r#"<calcChain xmlns="{NS}">{entries}</calcChain>"#)
    }

    fn at(a1: &str) -> Cell {
        Cell::parse(a1).expect("the test asks for a cell")
    }

    fn without_cell(xml: &str, sheet: u32, a1: &str) -> (String, usize) {
        let removal = without(xml, sheet, at(a1)).expect("the chain must be readable");
        let spliced = splice::apply(xml, &removal.splices).expect("the splices must apply");
        (spliced, removal.remaining)
    }

    #[test]
    fn an_entry_is_taken_out_and_every_other_byte_stays() {
        let xml = chain(r#"<c r="E2" i="1" l="1"/><c r="E3" i="1"/><c r="D1" i="1"/>"#);

        let (spliced, remaining) = without_cell(&xml, 1, "E3");

        assert_eq!(
            spliced,
            chain(r#"<c r="E2" i="1" l="1"/><c r="D1" i="1"/>"#)
        );
        assert_eq!(remaining, 2);
    }

    /// A cell on another sheet may have the same address, so the sheet is
    /// half of what names an entry.
    #[test]
    fn an_entry_on_another_sheet_is_another_entry() {
        let xml = chain(r#"<c r="A1" i="1"/><c r="A1" i="2"/>"#);

        let (spliced, remaining) = without_cell(&xml, 2, "A1");

        assert_eq!(spliced, chain(r#"<c r="A1" i="1"/>"#));
        assert_eq!(remaining, 1);
    }

    /// An entry that names no sheet is on the sheet the entry before it
    /// named, so one can be matched without naming a sheet of its own.
    #[test]
    fn an_entry_that_names_no_sheet_is_on_the_one_before_it() {
        let xml = chain(r#"<c r="A1" i="1"/><c r="A2"/><c r="A3" i="2"/>"#);

        let (spliced, _) = without_cell(&xml, 1, "A2");

        assert_eq!(spliced, chain(r#"<c r="A1" i="1"/><c r="A3" i="2"/>"#));
    }

    /// Taking out an entry that others were leaning on must not move them to
    /// another sheet, so the one after it is given the sheet it was on.
    #[test]
    fn the_entry_after_one_removed_keeps_the_sheet_it_was_on() {
        let xml = chain(r#"<c r="A1" i="1"/><c r="B1" i="2"/><c r="C1"/>"#);

        let (spliced, _) = without_cell(&xml, 2, "B1");

        assert_eq!(
            spliced,
            chain(r#"<c r="A1" i="1"/><c r="C1" i="2"/>"#),
            "C1 was on sheet 2 and stays on sheet 2"
        );
    }

    /// A chain may name a cell that no longer holds a formula, or not name one
    /// that does. Nothing depends on it being right, so there is nothing to do
    /// and nothing to complain about.
    #[test]
    fn a_chain_with_no_entry_for_the_cell_has_nothing_to_take_out() {
        let xml = chain(r#"<c r="A1" i="1"/>"#);

        let removal = without(&xml, 1, at("Z9")).expect("the chain must be readable");

        assert_eq!(removal.splices, Vec::new());
        assert_eq!(removal.remaining, 1);
    }

    #[test]
    fn a_chain_down_to_its_last_entry_says_so() {
        let xml = chain(r#"<c r="A1" i="1"/>"#);

        let (spliced, remaining) = without_cell(&xml, 1, "A1");

        assert_eq!(remaining, 0);
        assert_eq!(entries_in(&spliced).expect("still valid XML"), 0);
    }

    #[test]
    fn a_part_that_is_not_a_calc_chain_is_unreadable() {
        let err = without(r#"<worksheet/>"#, 1, at("A1")).expect_err("not a calc chain");

        assert_eq!(err.code(), crate::error::ErrorCode::Unreadable);
    }
}
