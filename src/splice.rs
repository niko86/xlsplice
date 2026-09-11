//! The splice layer: byte ranges in one part's text, and the rule for
//! applying several of them at once.
//!
//! Nothing here knows what a worksheet is. Its job is the mechanics ADR-0001
//! asks for: a part is text, an edit is a range of it replaced by other text,
//! and every byte outside the ranges is the byte that was there before. The
//! ranges come from the tree roxmltree parsed out of that same text, so an
//! [`Element`] can say where an element's name, attributes and content sit
//! without re-scanning the part.
//!
//! Splices are applied from the end of the part backwards, so that a splice
//! still to be applied is still where the tree said it was (ADR-0002). Two
//! splices that overlap are a caller asking for two different things in the
//! same place, which is a bug rather than a package problem, so it is an
//! [`internal`](crate::error::ErrorCode::Internal) failure.

use std::ops::Range;

use roxmltree::Node;

use crate::error::{Error, Result};

/// One replacement: the bytes of `range` become `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    /// Where in the part the replacement lands. An empty range is an
    /// insertion at that point.
    pub range: Range<usize>,
    /// What goes there.
    pub text: String,
}

impl Splice {
    /// A replacement of `range` by `text`.
    pub fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Splice {
            range,
            text: text.into(),
        }
    }

    /// Whether applying this splice to `source` would change a byte. A splice
    /// that would not is how a write of a value already present reports
    /// itself unchanged.
    pub fn changes(&self, source: &str) -> bool {
        source.get(self.range.clone()) != Some(self.text.as_str())
    }
}

/// Apply every splice in `splices` to `source`.
///
/// Order does not matter to the caller: the splices are ordered here and
/// applied from the end backwards. Where two start at the same byte, the
/// shorter goes first, so that an insertion at the front of a replaced range
/// lands before it rather than inside it.
pub fn apply(source: &str, splices: &[Splice]) -> Result<String> {
    let mut ordered: Vec<&Splice> = splices.iter().collect();
    ordered.sort_by_key(|splice| (splice.range.start, splice.range.end));

    let mut reached = 0;
    for splice in &ordered {
        let Range { start, end } = splice.range;
        if start > end
            || end > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            return Err(Error::internal(format!(
                "a splice covers {start}..{end}, which is not a range of the \
                 {} bytes of the part it was taken from",
                source.len()
            )));
        }
        if start < reached {
            return Err(Error::internal(format!(
                "two splices overlap at byte {start}; one part cannot be \
                 asked for two different things in the same place"
            )));
        }
        reached = end;
    }

    let mut spliced = source.to_owned();
    for splice in ordered.iter().rev() {
        spliced.replace_range(splice.range.clone(), &splice.text);
    }
    Ok(spliced)
}

/// Where one element's pieces sit in the text it was parsed from.
///
/// The text is the same text the node's document was parsed from, which is
/// what makes the ranges roxmltree reports meaningful here.
#[derive(Debug, Clone)]
pub struct Element<'a, 'input> {
    node: Node<'a, 'input>,
    source: &'input str,
    /// The whole element, `<c ...>...</c>` or `<c .../>`.
    range: Range<usize>,
    /// The qualified name as the part writes it: `c`, or `x:c`.
    name: &'input str,
    /// The start tag, `<c ...>` or `<c .../>`, including its `>`.
    open: Range<usize>,
    /// Whether the element is written `<c/>` rather than `<c></c>`.
    self_closing: bool,
    /// What sits between the tags. Empty for a self-closing element, which
    /// has no between.
    content: Range<usize>,
}

impl<'a, 'input> Element<'a, 'input> {
    /// Find `node`'s pieces in `source`, the text its document was parsed
    /// from.
    ///
    /// Every failure here is internal: the node came out of a parse of this
    /// text, so a shape that cannot be found is a bug in the locating, not a
    /// package that is wrong.
    pub fn of(node: Node<'a, 'input>, source: &'input str) -> Result<Self> {
        let range = node.range();
        let text = source.get(range.clone()).ok_or_else(|| {
            Error::internal(format!(
                "element <{}> is reported at {}..{} of a part of {} bytes",
                node.tag_name().name(),
                range.start,
                range.end,
                source.len()
            ))
        })?;

        let name_start = range.start + 1;
        let name_end = text
            .char_indices()
            .skip(1)
            .find(|(_, ch)| ch.is_whitespace() || *ch == '/' || *ch == '>')
            .map(|(offset, _)| range.start + offset)
            .ok_or_else(|| Self::malformed(node, "its name never ends"))?;

        let open_end = Self::start_tag_end(source, name_end, range.end)
            .ok_or_else(|| Self::malformed(node, "its start tag has no '>'"))?;
        let open = range.start..open_end;

        let self_closing = source[..open.end].ends_with("/>");
        let content = if self_closing {
            // A self-closing element has no content, and no place to put any
            // without opening it; `content_splice` does the opening.
            open.end..open.end
        } else {
            let floor = node
                .last_child()
                .map(|child| child.range().end)
                .unwrap_or(open.end);
            let close = source[floor..range.end]
                .rfind('<')
                .map(|offset| floor + offset)
                .ok_or_else(|| Self::malformed(node, "it has no closing tag"))?;
            open.end..close
        };

        Ok(Element {
            node,
            source,
            name: &source[name_start..name_end],
            range,
            open,
            self_closing,
            content,
        })
    }

    /// The prefix the part writes this element with, colon and all, or the
    /// empty string. Elements written alongside it take the same one, so a
    /// part a re-serialising tool prefixed stays prefixed.
    pub fn prefix(&self) -> &'input str {
        match self.name.split_once(':') {
            Some((prefix, _)) => &self.name[..prefix.len() + 1],
            None => "",
        }
    }

    /// Replace everything between the element's tags with `text`, opening a
    /// self-closing element to do it.
    pub fn content_splice(&self, text: &str) -> Splice {
        if !self.self_closing {
            return Splice::new(self.content.clone(), text);
        }
        // `<c r="A1" />` closes over a space that has nothing left to
        // separate once the tag is opened, so the splice eats it.
        let start = self.back_over_whitespace(self.open.end - 2);
        Splice::new(start..self.range.end, format!(">{text}</{}>", self.name))
    }

    /// Give the attribute called `name` the value `value`, or take it away
    /// when `value` is `None`. `None` for an attribute the element does not
    /// have is nothing to do.
    ///
    /// `after` names the attributes this one follows in its schema's order, so
    /// that one being added lands where that schema would put it rather than
    /// at the end. An element carrying none of them takes it first.
    pub fn attribute_splice(
        &self,
        name: &str,
        value: Option<&str>,
        after: &[&str],
    ) -> Option<Splice> {
        let found = self
            .node
            .attributes()
            .find(|attribute| attribute.name() == name);
        match (found, value) {
            (None, None) => None,
            (None, Some(value)) => {
                let at = self.after(after);
                Some(Splice::new(at..at, format!(" {name}=\"{value}\"")))
            }
            (Some(attribute), Some(value)) => Some(Splice::new(attribute.range_value(), value)),
            (Some(attribute), None) => {
                // The space in front of an attribute belongs to it: leaving
                // it would double the one before the next attribute.
                let start = self.back_over_whitespace(attribute.range().start);
                Some(Splice::new(start..attribute.range().end, ""))
            }
        }
    }

    /// Just past the last of `named` the element carries, or just past its
    /// name when it carries none of them.
    fn after(&self, named: &[&str]) -> usize {
        self.node
            .attributes()
            .filter(|attribute| named.contains(&attribute.name()))
            .map(|attribute| attribute.range().end)
            .max()
            .unwrap_or_else(|| self.attributes_start())
    }

    /// `from`, less any whitespace immediately before it, stopping where the
    /// element's attributes begin.
    fn back_over_whitespace(&self, from: usize) -> usize {
        let mut start = from;
        while start > self.attributes_start() && self.source[..start].ends_with(char::is_whitespace)
        {
            start -= 1;
        }
        start
    }

    /// Just past the element's name: the first byte an attribute may sit at.
    fn attributes_start(&self) -> usize {
        self.range.start + 1 + self.name.len()
    }

    /// The index just past the start tag's `>`, scanning from the end of the
    /// name and stepping over quoted attribute values.
    fn start_tag_end(source: &str, from: usize, limit: usize) -> Option<usize> {
        let mut quote: Option<char> = None;
        for (offset, ch) in source[from..limit].char_indices() {
            match (quote, ch) {
                (Some(open), ch) if ch == open => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(ch),
                (None, '>') => return Some(from + offset + 1),
                (None, _) => {}
            }
        }
        None
    }

    fn malformed(node: Node, why: &str) -> Error {
        Error::internal(format!(
            "element <{}> cannot be spliced: {why}",
            node.tag_name().name()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    fn spliced(source: &str, splices: &[Splice]) -> String {
        apply(source, splices).expect("the test splices must apply")
    }

    #[test]
    fn splices_land_where_they_were_taken_however_they_were_ordered() {
        let source = "<a><b>1</b><c>2</c></a>";

        assert_eq!(
            spliced(
                source,
                &[Splice::new(14..15, "two"), Splice::new(6..7, "one")]
            ),
            "<a><b>one</b><c>two</c></a>"
        );
    }

    #[test]
    fn an_empty_range_inserts_without_replacing() {
        assert_eq!(
            spliced("<c/>", &[Splice::new(2..2, " r=\"A1\"")]),
            "<c r=\"A1\"/>"
        );
    }

    #[test]
    fn an_insertion_at_the_front_of_a_replacement_lands_before_it() {
        // The attribute and the content of a self-closing cell both start at
        // the same byte, and the attribute has to end up inside the tag.
        let source = "<c/>";
        let splices = [
            Splice::new(2..2, " t=\"b\""),
            Splice::new(2..4, "><v>1</v></c>"),
        ];

        assert_eq!(spliced(source, &splices), "<c t=\"b\"><v>1</v></c>");
    }

    #[test]
    fn two_splices_that_overlap_are_a_bug_rather_than_a_result() {
        let err = apply(
            "<a>12345</a>",
            &[Splice::new(3..6, "x"), Splice::new(5..7, "y")],
        )
        .expect_err("overlapping splices must not apply");

        assert_eq!(err.code(), crate::error::ErrorCode::Internal);
    }

    #[test]
    fn a_splice_outside_the_part_is_a_bug_rather_than_a_panic() {
        let err = apply("<a/>", &[Splice::new(2..99, "x")])
            .expect_err("a splice past the end must not apply");

        assert_eq!(err.code(), crate::error::ErrorCode::Internal);
    }

    #[test]
    fn a_splice_says_whether_it_would_change_a_byte() {
        let source = "<c r=\"A1\"><v>1</v></c>";

        assert!(!Splice::new(10..18, "<v>1</v>").changes(source));
        assert!(Splice::new(10..18, "<v>2</v>").changes(source));
    }

    /// The element of the one cell in `xml`, with the text it came from.
    fn cell(xml: &str) -> (Document<'_>, &str) {
        (Document::parse(xml).expect("the test part must parse"), xml)
    }

    fn element<'a, 'input>(
        document: &'a Document<'input>,
        source: &'input str,
    ) -> Element<'a, 'input> {
        let node = document
            .root_element()
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "c")
            .expect("the test part holds a cell");
        Element::of(node, source).expect("the cell must be locatable")
    }

    #[test]
    fn an_elements_content_is_what_sits_between_its_tags() {
        let xml = "<row><c r=\"A1\"><v>1</v></c></row>";
        let (document, source) = cell(xml);
        let element = element(&document, source);

        assert_eq!(&source[element.content.clone()], "<v>1</v>");
        assert!(!element.self_closing);
        assert_eq!(element.prefix(), "");
    }

    #[test]
    fn replacing_the_content_of_a_self_closing_element_opens_it() {
        let xml = "<row><c r=\"B2\" s=\"4\"/></row>";
        let (document, source) = cell(xml);

        assert_eq!(
            spliced(
                source,
                &[element(&document, source).content_splice("<v>1</v>")]
            ),
            "<row><c r=\"B2\" s=\"4\"><v>1</v></c></row>"
        );
    }

    #[test]
    fn the_space_a_self_closing_element_held_open_goes_with_the_slash() {
        let xml = "<row><c r=\"B2\" /></row>";
        let (document, source) = cell(xml);

        assert_eq!(
            spliced(
                source,
                &[element(&document, source).content_splice("<v>1</v>")]
            ),
            "<row><c r=\"B2\"><v>1</v></c></row>"
        );
    }

    #[test]
    fn an_empty_element_written_with_both_tags_takes_content_between_them() {
        let xml = "<row><c r=\"A1\"></c></row>";
        let (document, source) = cell(xml);

        assert_eq!(
            spliced(
                source,
                &[element(&document, source).content_splice("<v>1</v>")]
            ),
            "<row><c r=\"A1\"><v>1</v></c></row>"
        );
    }

    #[test]
    fn an_attribute_is_added_after_the_last_one_changed_in_place_and_removed_with_its_space() {
        let xml = "<row><c r=\"A1\" s=\"3\" t=\"s\"><v>0</v></c></row>";
        let (document, source) = cell(xml);
        let element = element(&document, source);

        assert_eq!(
            spliced(
                source,
                &[element
                    .attribute_splice("t", Some("b"), &["r", "s"])
                    .unwrap()]
            ),
            "<row><c r=\"A1\" s=\"3\" t=\"b\"><v>0</v></c></row>"
        );
        assert_eq!(
            spliced(
                source,
                &[element.attribute_splice("t", None, &["r", "s"]).unwrap()]
            ),
            "<row><c r=\"A1\" s=\"3\"><v>0</v></c></row>"
        );
        assert_eq!(
            spliced(
                source,
                &[element
                    .attribute_splice("cm", Some("1"), &["r", "s", "t"])
                    .unwrap()]
            ),
            "<row><c r=\"A1\" s=\"3\" t=\"s\" cm=\"1\"><v>0</v></c></row>"
        );
    }

    #[test]
    fn taking_away_an_attribute_that_is_not_there_is_nothing_to_do() {
        let xml = "<row><c r=\"A1\"><v>1</v></c></row>";
        let (document, source) = cell(xml);

        assert_eq!(
            element(&document, source).attribute_splice("t", None, &["r", "s"]),
            None
        );
    }

    #[test]
    fn an_element_with_no_attributes_takes_its_first_after_its_name() {
        let xml = "<row><c><v>1</v></c></row>";
        let (document, source) = cell(xml);

        assert_eq!(
            spliced(
                source,
                &[element(&document, source)
                    .attribute_splice("t", Some("b"), &["r", "s"])
                    .unwrap()]
            ),
            "<row><c t=\"b\"><v>1</v></c></row>"
        );
    }

    #[test]
    fn a_prefixed_element_keeps_its_prefix_and_reports_it() {
        let xml = "<x:row xmlns:x=\"urn:x\"><x:c r=\"A1\"/></x:row>";
        let (document, source) = cell(xml);
        let element = element(&document, source);

        assert_eq!(element.prefix(), "x:");
        assert_eq!(
            spliced(source, &[element.content_splice("<x:v>1</x:v>")]),
            "<x:row xmlns:x=\"urn:x\"><x:c r=\"A1\"><x:v>1</x:v></x:c></x:row>"
        );
    }

    #[test]
    fn an_attribute_whose_value_holds_a_bracket_does_not_end_the_start_tag_early() {
        let xml = "<row><c r=\"A1\" cm=\"a&gt;b\"><v>1</v></c></row>";
        let (document, source) = cell(xml);

        assert_eq!(
            &source[element(&document, source).content.clone()],
            "<v>1</v>"
        );
    }
}
