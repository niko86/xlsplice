//! The small things every part reader and writer wants: two around
//! roxmltree, and two for putting a value back.
//!
//! The readers exist because a part in the wild is not the tidy shape a
//! reader would like. A re-serialising tool writes every element with a
//! namespace prefix, so only the local name may be compared; and an entity
//! reference splits an element's text into several nodes, so only their
//! concatenation is the text.
//!
//! The writers exist because every part xlsplice puts a value into owes the
//! reader the same escaping and the same spelling of a number, and it is the
//! same question whichever part is being written.

use roxmltree::Node;

/// The direct children of `parent` that are elements called `name`. Only the
/// local name is compared, so a part written with a prefix, as every part a
/// re-serialising tool has touched is, reads the same as one without.
pub fn children<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    parent
        .children()
        .filter(move |node| node.is_element() && node.tag_name().name() == name)
}

/// Everything `node` holds as text, in document order.
///
/// Every text node under `node` is taken rather than only the first, because
/// an entity reference splits the text in two and both halves are the text.
pub fn text_of(node: Node) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect()
}

/// Text as XML character data.
///
/// The three characters that would otherwise be markup are escaped. So is a
/// carriage return, which a parser is required to turn into a line feed when
/// it reads the part back: written as itself it would not survive the trip.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\r' => escaped.push_str("&#13;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// A number in the shortest form that reads back as itself.
///
/// Rust's own form is that: the fewest decimal digits that parse back to the
/// same double, and no decimal point when the value is integral, which is how
/// a package spells a whole number. It is always positional, so a value at the
/// far end of the range is written out in full rather than with an exponent.
/// Excel writes an exponent there and both read back the same, so whether to
/// follow it is a question for the oracle suite rather than a guess here.
///
/// Negative zero is written as zero: a part has one zero, and it is not
/// spelled with a sign.
pub fn number_text(number: f64) -> String {
    let number = if number == 0.0 { 0.0 } else { number };
    number.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    #[test]
    fn text_that_would_be_markup_is_escaped_and_a_carriage_return_with_it() {
        assert_eq!(escape("a<b&c>d\re"), "a&lt;b&amp;c&gt;d&#13;e");
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn only_direct_children_of_the_asked_for_name_come_back() {
        let xml = "<a><b id='1'/><c><b id='deep'/></c><b id='2'/></a>";
        let document = Document::parse(xml).expect("the test document must parse");
        let ids: Vec<_> = children(document.root_element(), "b")
            .filter_map(|node| node.attribute("id"))
            .collect();

        assert_eq!(ids, ["1", "2"]);
    }

    #[test]
    fn a_prefixed_element_is_found_by_its_local_name() {
        let xml = "<x:a xmlns:x='urn:x'><x:b id='1'/></x:a>";
        let document = Document::parse(xml).expect("the test document must parse");

        assert_eq!(children(document.root_element(), "b").count(), 1);
    }

    #[test]
    fn text_split_by_an_entity_reference_comes_back_whole() {
        let xml = "<t>a&amp;b</t>";
        let document = Document::parse(xml).expect("the test document must parse");

        assert_eq!(text_of(document.root_element()), "a&b");
    }

    #[test]
    fn an_element_with_no_text_is_empty_rather_than_absent() {
        let document = Document::parse("<t/>").expect("the test document must parse");

        assert_eq!(text_of(document.root_element()), "");
    }
}
