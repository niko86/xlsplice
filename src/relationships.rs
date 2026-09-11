//! Relationships: the `_rels` part beside a part, and what its ids point at.
//!
//! A package never names a part from another part directly. It names a
//! relationship id, and the `_rels` part beside the naming part says where
//! that id points. Every hop xlsplice makes, from the package root to the
//! workbook and from the workbook to a worksheet or the shared string table,
//! is one of these, so one reader serves them all.
//!
//! A relationship whose target is external names a URL rather than a part, so
//! it resolves to nothing at all.

use roxmltree::Document;

use crate::error::{Error, Result};
use crate::package::Package;
use crate::xml::children;

/// The relationship type of the package's main document, the workbook.
pub const OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// The relationship type of the shared string table.
pub const SHARED_STRINGS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";

/// The relationships of the package root, which name the workbook.
pub const ROOT_RELS: &str = "_rels/.rels";

/// One part's relationships, resolved against the part that owns them.
#[derive(Debug)]
pub struct Relationships {
    owner: String,
    entries: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    id: String,
    kind: String,
    target: String,
    /// An external target is a URL, not a part.
    external: bool,
}

impl Relationships {
    /// The relationships of `owner`, which is the empty string for the
    /// package root. A part with no `_rels` beside it simply has none.
    pub fn read(package: &mut Package, owner: &str) -> Result<Self> {
        let part = rels_path(owner);
        if !package.has_part(&part) {
            return Ok(Relationships {
                owner: owner.to_owned(),
                entries: Vec::new(),
            });
        }
        let xml = package.read_part_text(&part)?;
        let document = Document::parse(&xml).map_err(|err| {
            Error::unreadable(format!(
                "{}: {part} is not valid XML: {err}",
                package.path().display()
            ))
        })?;
        let entries = children(document.root_element(), "Relationship")
            .map(|node| Entry {
                id: node.attribute("Id").unwrap_or_default().to_owned(),
                kind: node.attribute("Type").unwrap_or_default().to_owned(),
                target: node.attribute("Target").unwrap_or_default().to_owned(),
                external: node.attribute("TargetMode") == Some("External"),
            })
            .collect();
        Ok(Relationships {
            owner: owner.to_owned(),
            entries,
        })
    }

    /// The part the relationship `id` points at.
    pub fn part_for_id(&self, id: &str) -> Option<String> {
        self.part_of(self.entries.iter().find(|entry| entry.id == id)?)
    }

    /// The part the first relationship of `kind` points at. A part carries at
    /// most one shared string table and one main document, so first is the
    /// only one for every type xlsplice follows by type.
    pub fn part_of_kind(&self, kind: &str) -> Option<String> {
        self.part_of(self.entries.iter().find(|entry| entry.kind == kind)?)
    }

    fn part_of(&self, entry: &Entry) -> Option<String> {
        (!entry.external && !entry.target.is_empty()).then(|| resolve(&self.owner, &entry.target))
    }
}

/// Settle on a part: the one a relationship names, if the package holds it,
/// and otherwise where Excel always puts it, if the package holds that.
///
/// A package another tool has mangled can lose a relationship while the part
/// it named is still in its conventional place, and a read has no reason to
/// refuse it. Nothing comes back when neither is there, and the caller says
/// what that means for the part it was after.
pub fn part_or_conventional(
    package: &Package,
    named: Option<String>,
    conventional: &str,
) -> Option<String> {
    named.filter(|part| package.has_part(part)).or_else(|| {
        package
            .has_part(conventional)
            .then(|| conventional.to_owned())
    })
}

/// Where the relationships of `part` live: `_rels` beside it, named after it.
/// The package root owns `_rels/.rels`, and is named by the empty string.
pub fn rels_path(part: &str) -> String {
    if part.is_empty() {
        return ROOT_RELS.to_owned();
    }
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolve a relationship target against the part that owns it.
///
/// A target beginning with `/` is already a path from the package root; any
/// other is relative to the directory holding the owner. `.` and `..` are
/// walked rather than left in the path, because a part path is compared
/// against the container's own listing and must match it exactly.
fn resolve(owner: &str, target: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    if !target.starts_with('/')
        && let Some((dir, _)) = owner.rsplit_once('/')
    {
        segments.extend(dir.split('/'));
    }
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parts_relationships_sit_in_rels_beside_it() {
        assert_eq!(rels_path(""), "_rels/.rels");
        assert_eq!(rels_path("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
        assert_eq!(
            rels_path("xl/worksheets/sheet1.xml"),
            "xl/worksheets/_rels/sheet1.xml.rels"
        );
        assert_eq!(rels_path("loose.xml"), "_rels/loose.xml.rels");
    }

    #[test]
    fn a_target_is_resolved_against_the_directory_holding_its_owner() {
        assert_eq!(
            resolve("xl/workbook.xml", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(resolve("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(
            resolve("xl/worksheets/sheet1.xml", "../drawings/drawing1.xml"),
            "xl/drawings/drawing1.xml"
        );
        assert_eq!(
            resolve("xl/workbook.xml", "./sharedStrings.xml"),
            "xl/sharedStrings.xml"
        );
    }

    #[test]
    fn a_target_from_the_package_root_ignores_where_its_owner_sits() {
        assert_eq!(
            resolve("xl/workbook.xml", "/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(resolve("", "/docProps/custom.xml"), "docProps/custom.xml");
    }
}
