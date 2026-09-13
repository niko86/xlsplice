//! The workbook's calculation settings: whether it is flagged to recalculate
//! fully when it is opened.
//!
//! `<calcPr>` in the workbook part carries them, and `fullCalcOnLoad` is the
//! one that matters here. A package whose cached values were written by
//! something other than Excel is a package whose values cannot be trusted, so
//! flagging it tells Excel to work them out again on the way in rather than
//! show what the cache says.
//!
//! Nothing else sets the flag. A cell write leaves it exactly as it found it,
//! because a tool that quietly changed how a workbook calculates would be
//! changing something it was not asked to.

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::splice::{Element, Splice};
use crate::xml::children;

/// The element the settings live on.
const CALC_PR: &str = "calcPr";

/// What `fullCalcOnLoad` is spelled as when it is set. The schema's boolean
/// also allows `true`, which is read but not written: `1` is what Excel
/// writes.
const SET: &str = "1";

/// The elements `calcPr` follows in the schema's order for a workbook. A new
/// one goes after the last of them the workbook carries, so that a package
/// xlsplice added it to is a package Excel would have written.
const AFTER: [&str; 4] = [
    "sheets",
    "functionGroups",
    "externalReferences",
    "definedNames",
];

/// The attributes `fullCalcOnLoad` follows in the schema's order for
/// `calcPr`.
const AFTER_ON_CALC_PR: [&str; 2] = ["calcId", "calcMode"];

/// Whether the workbook part whose text is `xml` is flagged to recalculate
/// fully on load.
///
/// A workbook with no `calcPr` is not flagged, and neither is one whose flag
/// says so: a boolean attribute the schema spells `1` or `0`, and which Excel
/// also writes as `true`.
pub fn full_calc_on_load(xml: &str) -> Result<bool> {
    let document = parsed(xml)?;
    Ok(flag_of(&root_of(&document)?))
}

/// The splices that make the flag say `wanted` in the workbook part whose
/// text is `xml`, or none at all where it already does.
///
/// Where there is no `calcPr` to carry the flag, one is written in its place
/// in the schema's order; where there is, only the attribute moves. Turning
/// the flag off takes the attribute away rather than writing a `0`, because
/// not flagged is what Excel leaves behind, and an element that ends up
/// carrying nothing is still left where it is: an empty `calcPr` is what a
/// workbook that has been calculated looks like.
pub fn set_full_calc_on_load(xml: &str, wanted: bool) -> Result<Vec<Splice>> {
    let document = parsed(xml)?;
    let root = root_of(&document)?;
    if flag_of(&root) == wanted {
        return Ok(Vec::new());
    }
    let Some(node) = children(root, CALC_PR).next() else {
        // Nothing to turn off, so nothing can reach here but turning it on.
        return written_in(root, xml);
    };
    let element = Element::of(node, xml)?;
    let value = wanted.then_some(SET);
    Ok(element
        .attribute_splice("fullCalcOnLoad", value, &AFTER_ON_CALC_PR)
        .into_iter()
        .collect())
}

/// The splice that writes a `calcPr` carrying the flag into a workbook that
/// has none, in the place the schema gives it.
fn written_in(root: Node, xml: &str) -> Result<Vec<Splice>> {
    let element = Element::of(root, xml)?;
    let prefix = element.prefix();
    let written = format!(r#"<{prefix}{CALC_PR} fullCalcOnLoad="{SET}"/>"#);
    let after = root
        .children()
        .filter(|node| node.is_element() && AFTER.contains(&node.tag_name().name()))
        .map(|node| node.range().end)
        .max();
    Ok(match after {
        Some(at) => vec![Splice::new(at..at, written)],
        // A workbook carrying none of the elements calcPr follows is one the
        // schema would not accept anyway; the flag still goes in, first.
        None => vec![element.first_child_splice(&written)],
    })
}

fn parsed(xml: &str) -> Result<Document<'_>> {
    Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("the workbook part is not valid XML: {err}")))
}

fn root_of<'a>(document: &'a Document<'a>) -> Result<Node<'a, 'a>> {
    let root = document.root_element();
    match root.tag_name().name() {
        "workbook" => Ok(root),
        other => Err(Error::unreadable(format!(
            "the workbook part's root element is <{other}>, not <workbook>"
        ))),
    }
}

/// Whether `root`'s calculation settings say the workbook recalculates fully
/// on load.
fn flag_of(root: &Node) -> bool {
    children(*root, CALC_PR)
        .next()
        .and_then(|node| node.attribute("fullCalcOnLoad"))
        .is_some_and(|flag| matches!(flag, "1" | "true"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splice;

    const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    fn workbook(body: &str) -> String {
        format!(r#"<workbook xmlns="{NS}">{body}</workbook>"#)
    }

    fn set(xml: &str) -> String {
        let splices = set_full_calc_on_load(xml, true).expect("the workbook must be readable");
        splice::apply(xml, &splices).expect("the splices must apply")
    }

    #[test]
    fn a_workbook_with_no_settings_is_not_flagged() {
        let xml = workbook("<sheets/>");

        assert!(!full_calc_on_load(&xml).expect("readable"));
    }

    #[test]
    fn the_flag_is_read_in_both_the_spellings_the_schema_allows() {
        for (flag, expected) in [("1", true), ("true", true), ("0", false), ("false", false)] {
            let xml = workbook(&format!(r#"<sheets/><calcPr fullCalcOnLoad="{flag}"/>"#));
            assert_eq!(
                full_calc_on_load(&xml).expect("readable"),
                expected,
                "{flag}"
            );
        }
    }

    /// The attribute goes in after the ones the schema puts before it, and
    /// every other byte of the element stays where it was.
    #[test]
    fn setting_on_a_self_closing_element_adds_the_attribute_in_its_place() {
        let xml = workbook(r#"<sheets/><calcPr calcId="191029"/>"#);

        assert_eq!(
            set(&xml),
            workbook(r#"<sheets/><calcPr calcId="191029" fullCalcOnLoad="1"/>"#)
        );
    }

    #[test]
    fn setting_on_an_open_close_element_leaves_its_content_alone() {
        let xml = workbook(r#"<sheets/><calcPr calcId="191029"><extLst/></calcPr>"#);

        assert_eq!(
            set(&xml),
            workbook(r#"<sheets/><calcPr calcId="191029" fullCalcOnLoad="1"><extLst/></calcPr>"#)
        );
    }

    #[test]
    fn setting_where_there_is_no_element_writes_one_after_the_defined_names() {
        let xml = workbook(
            r#"<sheets/><definedNames><definedName name="A">B</definedName></definedNames><extLst/>"#,
        );

        assert_eq!(
            set(&xml),
            workbook(
                r#"<sheets/><definedNames><definedName name="A">B</definedName></definedNames><calcPr fullCalcOnLoad="1"/><extLst/>"#
            )
        );
    }

    #[test]
    fn setting_where_there_are_no_defined_names_writes_one_after_the_sheets() {
        let xml = workbook(r#"<workbookPr/><sheets/><extLst/>"#);

        assert_eq!(
            set(&xml),
            workbook(r#"<workbookPr/><sheets/><calcPr fullCalcOnLoad="1"/><extLst/>"#)
        );
    }

    /// A part another tool prefixed keeps its prefix, as everything xlsplice
    /// writes does.
    #[test]
    fn an_element_written_in_takes_the_prefix_the_part_uses() {
        let xml = format!(r#"<x:workbook xmlns:x="{NS}"><x:sheets/></x:workbook>"#);

        assert!(
            set(&xml).contains(r#"<x:calcPr fullCalcOnLoad="1"/>"#),
            "{}",
            set(&xml)
        );
    }

    #[test]
    fn setting_a_flag_already_set_asks_for_nothing() {
        let xml = workbook(r#"<sheets/><calcPr fullCalcOnLoad="1"/>"#);

        assert_eq!(
            set_full_calc_on_load(&xml, true).expect("readable"),
            Vec::new()
        );
    }

    /// The flag off is the attribute gone, which is what a workbook Excel has
    /// calculated looks like. The element stays, because an empty `calcPr` is
    /// a thing Excel writes.
    #[test]
    fn turning_the_flag_off_takes_the_attribute_away() {
        let xml = workbook(r#"<sheets/><calcPr calcId="1" fullCalcOnLoad="1"/>"#);
        let splices = set_full_calc_on_load(&xml, false).expect("readable");

        assert_eq!(
            splice::apply(&xml, &splices).expect("the splices apply"),
            workbook(r#"<sheets/><calcPr calcId="1"/>"#)
        );
    }

    #[test]
    fn turning_off_a_flag_that_is_already_off_asks_for_nothing() {
        for body in ["<sheets/>", r#"<sheets/><calcPr calcId="1"/>"#] {
            let xml = workbook(body);
            assert_eq!(
                set_full_calc_on_load(&xml, false).expect("readable"),
                Vec::new(),
                "{body}"
            );
        }
    }

    #[test]
    fn a_part_that_is_not_a_workbook_is_unreadable() {
        let err = full_calc_on_load("<worksheet/>").expect_err("not a workbook");

        assert_eq!(err.code(), crate::error::ErrorCode::Unreadable);
    }
}
