//! What a package says about a part besides holding it: its content type, and
//! the relationship that reaches it.
//!
//! A part is not in a package merely by being in the container. `
//! [Content_Types].xml` says what kind of thing it is, and a relationship in
//! the `_rels` beside whichever part points at it says how it is reached. A
//! part removed while either still names it leaves a package that says it
//! holds something it does not, which is the sort of inconsistency Excel
//! offers to repair.
//!
//! So the two are withdrawn together, here, and a later ticket that creates a
//! part declares them together in the same place. Nothing else in xlsplice
//! writes to either part.

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::package::Package;
use crate::relationships::rels_path;
use crate::splice::Splice;
use crate::xml::children;

/// The part that says what kind of thing every other part is.
pub const CONTENT_TYPES: &str = "[Content_Types].xml";

/// The splices that take every mention of `part` out of the package's
/// declarations, by the part each lands in.
///
/// `owner` is the part whose relationships reach `part`, which for anything
/// under `xl/` is the workbook part. A declaration that is not there is not an
/// error: a package another tool has mangled may already be missing one, and
/// withdrawing what is left is still the right answer.
pub fn withdrawn(
    package: &mut Package,
    owner: &str,
    part: &str,
) -> Result<Vec<(String, Vec<Splice>)>> {
    let mut edits = Vec::new();
    for (declaring, splices) in [
        (CONTENT_TYPES.to_owned(), override_of(package, part)?),
        (rels_path(owner), relationship_to(package, owner, part)?),
    ] {
        if !splices.is_empty() {
            edits.push((declaring, splices));
        }
    }
    Ok(edits)
}

/// The splice that takes `part`'s content-type override out.
///
/// An override names its part with a leading slash, from the root of the
/// package. A part covered by a `Default` for its extension has no override
/// and needs none taken out: the default goes on covering the parts that are
/// still there.
fn override_of(package: &mut Package, part: &str) -> Result<Vec<Splice>> {
    if !package.has_part(CONTENT_TYPES) {
        return Ok(Vec::new());
    }
    let named = format!("/{part}");
    let xml = package.read_part_text(CONTENT_TYPES)?;
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("{CONTENT_TYPES} is not valid XML: {err}")))?;
    let wanted = children(document.root_element(), "Override")
        .find(|node| node.attribute("PartName") == Some(named.as_str()));
    Ok(cut(wanted, xml))
}

/// The splice that takes the relationship pointing at `part` out of `owner`'s
/// relationships.
fn relationship_to(package: &mut Package, owner: &str, part: &str) -> Result<Vec<Splice>> {
    let rels = rels_path(owner);
    if !package.has_part(&rels) {
        return Ok(Vec::new());
    }
    // A relationship's target is relative to the folder its owner sits in, so
    // it is resolved the same way reading one resolves it.
    let base = owner.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let xml = package.read_part_text(&rels)?;
    let document = Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("{rels} is not valid XML: {err}")))?;
    let wanted = children(document.root_element(), "Relationship").find(|node| {
        node.attribute("TargetMode") != Some("External")
            && node
                .attribute("Target")
                .is_some_and(|target| points_at(base, target, part))
    });
    Ok(cut(wanted, xml))
}

/// Whether a relationship target under `base` names `part`.
fn points_at(base: &str, target: &str, part: &str) -> bool {
    let absolute = match target.strip_prefix('/') {
        Some(rooted) => rooted.to_owned(),
        None if base.is_empty() => target.to_owned(),
        None => format!("{base}/{target}"),
    };
    absolute == part
}

/// The splice that cuts `node` out of the text it was parsed from, taking the
/// whitespace in front of it so a part written one element per line does not
/// keep the blank line where the element was.
fn cut(node: Option<Node>, xml: &str) -> Vec<Splice> {
    let Some(node) = node else {
        return Vec::new();
    };
    let range = node.range();
    let start = xml[..range.start]
        .trim_end_matches(|ch: char| ch.is_whitespace())
        .len();
    vec![Splice::new(start..range.end, "")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_read_against_the_folder_its_owner_sits_in() {
        assert!(points_at("xl", "calcChain.xml", "xl/calcChain.xml"));
        assert!(points_at(
            "xl",
            "worksheets/sheet1.xml",
            "xl/worksheets/sheet1.xml"
        ));
        assert!(points_at("xl", "/xl/calcChain.xml", "xl/calcChain.xml"));
        assert!(points_at("", "xl/workbook.xml", "xl/workbook.xml"));
        assert!(!points_at("xl", "calcChain.xml", "calcChain.xml"));
        assert!(!points_at("xl", "styles.xml", "xl/calcChain.xml"));
    }

    #[test]
    fn cutting_an_element_takes_the_whitespace_in_front_of_it() {
        let xml = "<Types>\n  <Override PartName=\"/a\"/>\n  <Override PartName=\"/b\"/>\n</Types>";
        let document = Document::parse(xml).expect("the test part must parse");
        let wanted = children(document.root_element(), "Override")
            .find(|node| node.attribute("PartName") == Some("/a"));

        let spliced = crate::splice::apply(xml, &cut(wanted, xml)).expect("the splice applies");

        assert_eq!(
            spliced, "<Types>\n  <Override PartName=\"/b\"/>\n</Types>",
            "the line the element was on goes with it"
        );
    }

    #[test]
    fn an_element_that_is_not_there_is_nothing_to_cut() {
        assert_eq!(cut(None, "<Types/>"), Vec::new());
    }
}
