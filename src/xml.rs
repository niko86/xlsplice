//! The two things every part reader wants from roxmltree.
//!
//! Both exist because a part in the wild is not the tidy shape a reader would
//! like. A re-serialising tool writes every element with a namespace prefix,
//! so only the local name may be compared; and an entity reference splits an
//! element's text into several nodes, so only their concatenation is the text.

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

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

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
