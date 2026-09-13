//! The shared string table, and the one rule for reading a string item.
//!
//! A text cell usually holds an index into `xl/sharedStrings.xml` rather than
//! its own text. The same string item shape appears inline in a worksheet
//! under `<is>`, so both are read here: the text of an item is the text of its
//! own `<t>` and of the `<t>` in each of its rich-text runs, concatenated in
//! document order.
//!
//! Phonetic text under `<rPh>` is not part of the string. It is a reading
//! guide Excel shows above the characters, and a reader that swept up every
//! `<t>` beneath the item would splice it into the middle of the value.

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::package::Package;
use crate::relationships::{Relationships, SHARED_STRINGS, part_or_conventional};
use crate::xml::{children, text_of};

/// Where the shared string table sits in every package Excel writes, used
/// when the workbook's relationships do not name one.
const CONVENTIONAL_PART: &str = "xl/sharedStrings.xml";

/// A package's shared strings, in the order the table holds them.
#[derive(Debug, Default)]
pub struct SharedStrings {
    items: Vec<String>,
}

impl SharedStrings {
    /// Read the table the workbook's relationships name, falling back to
    /// where Excel always puts it. A package holds at most one table, so
    /// there is nothing the conventional path could be confused with.
    ///
    /// The relationships are the workbook's, already parsed by the caller. A
    /// package with no table has no shared strings, which is not an error: no
    /// cell can then be a shared one.
    pub fn read(package: &mut Package, workbook_rels: &Relationships) -> Result<Self> {
        let named = workbook_rels.part_of_kind(SHARED_STRINGS);
        let Some(part) = part_or_conventional(package, named, CONVENTIONAL_PART) else {
            return Ok(SharedStrings::default());
        };
        let xml = package.read_part_text(&part)?;
        SharedStrings::parse(xml).map_err(|err| err.within(&part))
    }

    /// Build the table from the text of a shared strings part.
    pub fn parse(xml: &str) -> Result<Self> {
        let document = Document::parse(xml).map_err(|err| {
            Error::unreadable(format!("the shared strings part is not valid XML: {err}"))
        })?;
        Ok(SharedStrings {
            items: children(document.root_element(), "si")
                .map(string_item_text)
                .collect(),
        })
    }

    /// How many strings the table holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the table holds no strings at all.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The string at `index`, with its rich-text runs concatenated.
    pub fn get(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(String::as_str)
    }
}

/// The text of one string item: its own `<t>` and the `<t>` of each run,
/// concatenated. Everything else the item carries, phonetic text and the
/// properties that lay it out, is not the string.
pub fn string_item_text(item: Node) -> String {
    item.children()
        .filter(|node| node.is_element())
        .flat_map(|node| match node.tag_name().name() {
            "t" => vec![text_of(node)],
            "r" => children(node, "t").map(text_of).collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    fn table(body: &str) -> SharedStrings {
        SharedStrings::parse(&format!(r#"<sst xmlns="{NS}">{body}</sst>"#))
            .expect("the test table must parse")
    }

    #[test]
    fn a_plain_item_is_its_own_text() {
        let strings = table("<si><t>hello</t></si>");

        assert_eq!(strings.get(0), Some("hello"));
        assert_eq!(strings.len(), 1);
    }

    #[test]
    fn the_runs_of_a_rich_text_item_are_concatenated_in_order() {
        let strings = table(
            r#"<si><r><rPr><b/></rPr><t>rich</t></r><r><t> and </t></r><r><t>plain</t></r></si>"#,
        );

        assert_eq!(strings.get(0), Some("rich and plain"));
    }

    #[test]
    fn phonetic_text_is_a_reading_guide_and_not_part_of_the_string() {
        let strings = table(
            r#"<si><t>東京</t><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><phoneticPr fontId="1"/></si>"#,
        );

        assert_eq!(strings.get(0), Some("東京"));
    }

    #[test]
    fn whitespace_inside_an_item_is_kept_exactly() {
        let strings = table(r#"<si><t xml:space="preserve">  padded  </t></si>"#);

        assert_eq!(strings.get(0), Some("  padded  "));
    }

    #[test]
    fn an_empty_item_is_the_empty_string_rather_than_no_string() {
        let strings = table("<si><t/></si><si/>");

        assert_eq!(strings.get(0), Some(""));
        assert_eq!(strings.get(1), Some(""));
        assert_eq!(strings.len(), 2);
    }

    #[test]
    fn an_index_the_table_does_not_hold_is_no_string() {
        let strings = table("<si><t>only</t></si>");

        assert_eq!(strings.get(1), None);
        assert!(!strings.is_empty());
    }

    #[test]
    fn a_prefixed_table_reads_the_same_as_an_unprefixed_one() {
        let strings = SharedStrings::parse(&format!(
            r#"<x:sst xmlns:x="{NS}"><x:si><x:r><x:t>a</x:t></x:r><x:r><x:t>b</x:t></x:r></x:si></x:sst>"#
        ))
        .expect("a prefixed table is still a table");

        assert_eq!(strings.get(0), Some("ab"));
    }

    #[test]
    fn a_table_that_is_not_xml_is_unreadable() {
        assert!(SharedStrings::parse("<sst>").is_err());
    }

    #[test]
    fn a_package_with_no_table_has_no_shared_strings() {
        assert!(SharedStrings::default().is_empty());
        assert_eq!(SharedStrings::default().get(0), None);
    }
}
