//! Custom document properties: the named, typed values a package carries
//! about itself.
//!
//! They live in `docProps/custom.xml`, one `<property>` each, carrying a name
//! and one child element saying what type the value is and holding it. The
//! type is one of the variant types OOXML borrows from OLE automation, which
//! is why the child is `<vt:lpwstr>` rather than `<text>`.
//!
//! A package need not have the part at all. Setting a property on one that
//! has none creates it, which is the one place xlsplice puts a part into a
//! package: the part, its content type and the relationship that reaches it
//! go in together, because a package naming a part it does not hold is what
//! Excel offers to repair.
//!
//! Every property carries an identifier, `pid`, unique within the part and
//! counting from 2. A property replaced keeps the one it had, so that
//! anything holding a reference to it still reaches it; a property added
//! takes the next one free.

use std::io::{Read, Seek};

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::package::Package;
use crate::relationships::{PACKAGE_ROOT, Relationships, part_or_conventional};
use crate::splice::{Element, Splice, appended};
use crate::xml::{children, escape, number_text, text_of};

/// Where Excel puts the part.
pub const CONVENTIONAL_PART: &str = "docProps/custom.xml";

/// What the relationship reaching the part says it is.
pub const CUSTOM_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";

/// What `[Content_Types].xml` says the part is.
pub const CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.custom-properties+xml";

/// The namespace the variant types are in.
const VARIANT_TYPES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes";

/// The namespace the part itself is in.
const CUSTOM_PROPERTIES_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";

/// The format identifier every custom property carries. It names the property
/// set the properties belong to, and for the user-defined set it is this one
/// constant, which is why it is transcribed rather than read from anywhere.
const FMTID: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";

/// The first identifier a property may take. 0 and 1 are reserved by the
/// property-set format, so Excel's first custom property is 2.
const FIRST_PID: u32 = 2;

/// One custom property: what it is called, what type the package says it is,
/// and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    /// The name, as the package spells it.
    pub name: String,
    /// The variant type, as found: the local name of the element holding the
    /// value, such as `lpwstr` or `filetime`. Reported as found rather than
    /// mapped onto xlsplice's own names, because a caller reading a property
    /// xlsplice did not write wants to know what is actually there.
    pub variant: String,
    /// The value, read as far as the variant type allows.
    pub value: Value,
}

/// A property's value, as far as a type can be put on it.
///
/// The variants a package uses are many and the types a caller can act on are
/// few, so a value that is not one of these is the text the package holds.
/// That is not a loss: the variant is reported beside it, so nothing about
/// what is there has gone missing.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A string, or anything whose variant xlsplice does not read.
    Text(String),
    /// A whole number.
    Integer(i64),
    /// A number that is not whole, or is too big for the whole-number
    /// variant.
    Real(f64),
    /// A boolean.
    Bool(bool),
    /// A moment, ISO 8601 in UTC, as a `filetime` holds one.
    Moment(String),
}

impl Value {
    /// The number `number` becomes as a property value.
    ///
    /// A whole number that fits the 32-bit variant is written as one, because
    /// that is what Excel writes and what everything reading properties
    /// expects an integer to be; anything else is a real, which holds it
    /// exactly as far as a double can.
    pub fn number(number: f64) -> Value {
        let whole = number.fract() == 0.0 && (i32::MIN as f64..=i32::MAX as f64).contains(&number);
        match whole {
            true => Value::Integer(number as i64),
            false => Value::Real(number),
        }
    }

    /// The variant type this value is written as.
    pub fn variant(&self) -> &'static str {
        match self {
            Value::Text(_) => "lpwstr",
            Value::Integer(_) => "i4",
            Value::Real(_) => "r8",
            Value::Bool(_) => "bool",
            Value::Moment(_) => "filetime",
        }
    }

    /// The value as the part spells it, escaped and ready to go between the
    /// variant element's tags.
    fn written(&self) -> String {
        match self {
            Value::Text(text) => escape(text),
            Value::Integer(number) => number.to_string(),
            Value::Real(number) => number_text(*number),
            Value::Bool(yes) => yes.to_string(),
            Value::Moment(moment) => moment.clone(),
        }
    }

    /// The value the variant `variant` holding `text` stands for.
    ///
    /// A value that does not read as the variant says it should is the text
    /// itself: a part another tool wrote badly is still a part whose
    /// properties can be listed.
    fn read(variant: &str, text: &str) -> Value {
        let trimmed = text.trim();
        match variant {
            "i1" | "i2" | "i4" | "i8" | "int" | "ui1" | "ui2" | "ui4" | "ui8" | "uint" => trimmed
                .parse()
                .map(Value::Integer)
                .unwrap_or_else(|_| Value::Text(text.to_owned())),
            "r4" | "r8" | "decimal" => trimmed
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .map(Value::Real)
                .unwrap_or_else(|| Value::Text(text.to_owned())),
            "bool" => match trimmed {
                "true" | "1" => Value::Bool(true),
                "false" | "0" => Value::Bool(false),
                _ => Value::Text(text.to_owned()),
            },
            "filetime" | "date" => Value::Moment(trimmed.to_owned()),
            _ => Value::Text(text.to_owned()),
        }
    }
}

/// Where the package's custom document properties are, and whether it holds
/// them at all.
///
/// A package with none still answers with a path: it is where the part would
/// go, which is what something adding a property needs to know. The part is
/// followed by its relationship from the package root, falling back to where
/// Excel puts it, as every part xlsplice follows by type is.
pub fn part_of<R: Read + Seek>(package: &mut Package<R>) -> Result<(String, bool)> {
    let rels = Relationships::read(package, PACKAGE_ROOT)?;
    let found = part_or_conventional(
        package,
        rels.part_of_kind(CUSTOM_PROPERTIES),
        CONVENTIONAL_PART,
    );
    Ok(match found {
        Some(part) => (part, true),
        None => (CONVENTIONAL_PART.to_owned(), false),
    })
}

/// Every custom property the part holds, in the order it holds them.
pub fn read(xml: &str) -> Result<Vec<Property>> {
    let document = parsed(xml)?;
    Ok(children(document.root_element(), "property")
        .filter_map(|node| {
            let name = node.attribute("name")?;
            let value = node.children().find(|child| child.is_element())?;
            let variant = value.tag_name().name();
            Some(Property {
                name: name.to_owned(),
                variant: variant.to_owned(),
                value: Value::read(variant, &text_of(value)),
            })
        })
        .collect())
}

/// Whether the part holds a property called `name`.
pub fn holds(xml: &str, name: &str) -> Result<bool> {
    let document = parsed(xml)?;
    Ok(property_named(document.root_element(), name).is_some())
}

/// The splices that make the property called `name` hold `value` — none at
/// all where it already does — or `None` where the part holds no such
/// property.
///
/// A property already there keeps its element, its identifier and its name,
/// and only what it holds is written over, so replacing one moves nothing
/// else in the part. Adding one is [`added`], which the batch does rather
/// than one operation: see there for why.
pub fn written(xml: &str, name: &str, value: &Value) -> Result<Option<Vec<Splice>>> {
    let document = parsed(xml)?;
    let root = document.root_element();
    let Some(node) = property_named(root, name) else {
        return Ok(None);
    };
    let variant = written_variant(root, value);
    let element = Element::of(node, xml)?;
    Ok(Some(match element.content_text() == variant {
        true => Vec::new(),
        false => vec![element.content_splice(&variant)],
    }))
}

/// The splice that adds `new` to the part, in the order given, each with the
/// next identifier free.
///
/// Every one of them is added at once rather than one at a time, because the
/// identifiers have to run on from each other and the part says what the
/// first free one is. A property already in the part is not added here: that
/// is [`written`].
pub fn added(xml: &str, new: &[(&str, &Value)]) -> Result<Vec<Splice>> {
    if new.is_empty() {
        return Ok(Vec::new());
    }
    let document = parsed(xml)?;
    let root = document.root_element();
    let prefix = Element::of(root, xml)?.prefix();
    let mut written = String::new();
    for (offset, (name, value)) in new.iter().enumerate() {
        let pid = next_pid(root) + offset as u32;
        written.push_str(&format!(
            r#"<{prefix}property fmtid="{FMTID}" pid="{pid}" name="{}">{}</{prefix}property>"#,
            in_an_attribute(name),
            written_variant(root, value),
        ));
    }
    Ok(vec![appended(root, xml, &written)?])
}

/// The splices that take the property called `name` out, or `None` where the
/// part holds no such property.
///
/// `None` rather than an empty list, because unsetting a property that is not
/// there is a caller pointing at something the package does not have, and
/// that is the caller's to be told about rather than a batch that changed
/// nothing.
pub fn unset(xml: &str, name: &str) -> Result<Option<Vec<Splice>>> {
    let document = parsed(xml)?;
    let Some(node) = property_named(document.root_element(), name) else {
        return Ok(None);
    };
    let range = node.range();
    let start = xml[..range.start]
        .trim_end_matches(|ch: char| ch.is_whitespace())
        .len();
    Ok(Some(vec![Splice::new(start..range.end, "")]))
}

/// A whole custom properties part holding `new`, for a package that has none.
///
/// Written the way Excel writes one, declaration and all, because a part
/// xlsplice puts into a package should be indistinguishable from the part
/// that would have been there had the properties been set in Excel.
pub fn part_holding(new: &[(&str, &Value)]) -> Vec<u8> {
    let mut written = String::new();
    for (offset, (name, value)) in new.iter().enumerate() {
        written.push_str(&format!(
            r#"<property fmtid="{FMTID}" pid="{pid}" name="{}"><vt:{variant}>{value}</vt:{variant}></property>"#,
            in_an_attribute(name),
            pid = FIRST_PID + offset as u32,
            variant = value.variant(),
            value = value.written(),
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
         <Properties xmlns=\"{CUSTOM_PROPERTIES_NS}\" xmlns:vt=\"{VARIANT_TYPES}\">\
         {written}</Properties>"
    )
    .into_bytes()
}

/// The property called `name`, compared as the package compares them: exactly,
/// because two properties differing only in case are two properties.
fn property_named<'a, 'input>(root: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    children(root, "property").find(|node| node.attribute("name") == Some(name))
}

/// The variant element holding `value`, prefixed the way this part prefixes
/// the variant namespace.
fn written_variant(root: Node, value: &Value) -> String {
    let (prefix, declaration) = variant_prefix(root);
    let variant = value.variant();
    format!(
        "<{prefix}{variant}{declaration}>{}</{prefix}{variant}>",
        value.written()
    )
}

/// How this part writes an element of the variant namespace: the prefix to put
/// in front of the name, and a declaration to go on the element where the part
/// binds the namespace nowhere.
///
/// A part Excel wrote binds `vt`, and one a re-serialising tool wrote may bind
/// something else or make it the default. A part that binds it nowhere is
/// malformed, and rather than write an element in the wrong namespace the
/// declaration goes on the element itself.
fn variant_prefix(root: Node) -> (String, &'static str) {
    match root
        .namespaces()
        .find(|namespace| namespace.uri() == VARIANT_TYPES)
    {
        Some(namespace) => (
            namespace
                .name()
                .map(|name| format!("{name}:"))
                .unwrap_or_default(),
            "",
        ),
        None => (
            "vt:".to_owned(),
            concat!(
                " xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/",
                "docPropsVTypes\""
            ),
        ),
    }
}

/// The lowest identifier no property has taken, never below the first one a
/// custom property may have.
fn next_pid(root: Node) -> u32 {
    let taken = children(root, "property")
        .filter_map(|node| node.attribute("pid"))
        .filter_map(|pid| pid.parse::<u32>().ok())
        .max();
    match taken {
        Some(highest) if highest >= FIRST_PID => highest + 1,
        _ => FIRST_PID,
    }
}

/// Text as an attribute value in double quotes.
fn in_an_attribute(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

fn parsed(xml: &str) -> Result<Document<'_>> {
    let document = Document::parse(xml).map_err(|err| {
        Error::unreadable(format!(
            "the custom properties part is not valid XML: {err}"
        ))
    })?;
    match document.root_element().tag_name().name() {
        "Properties" => Ok(document),
        other => Err(Error::unreadable(format!(
            "the custom properties part's root element is <{other}>, not <Properties>"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splice;

    /// A part shaped the way Excel writes one, holding `body`.
    fn part(body: &str) -> String {
        format!(
            r#"<Properties xmlns="{CUSTOM_PROPERTIES_NS}" xmlns:vt="{VARIANT_TYPES}">{body}</Properties>"#
        )
    }

    /// One property element, as Excel writes it.
    fn property(pid: u32, name: &str, variant: &str, value: &str) -> String {
        format!(
            r#"<property fmtid="{FMTID}" pid="{pid}" name="{name}"><vt:{variant}>{value}</vt:{variant}></property>"#
        )
    }

    /// The part with the four types the feature fixture carries.
    fn four_types() -> String {
        part(&format!(
            "{}{}{}{}",
            property(2, "Stamp.Text", "lpwstr", "xlsplice"),
            property(3, "Stamp.Number", "i4", "42"),
            property(4, "Stamp.Flag", "bool", "true"),
            property(5, "Stamp.Date", "filetime", "2026-09-11T10:00:00Z"),
        ))
    }

    /// What the batch does between them: write the property over where it is
    /// there, and add it where it is not.
    fn set_in(xml: &str, name: &str, value: &Value) -> String {
        let splices = match written(xml, name, value).expect("the part must be readable") {
            Some(splices) => splices,
            None => added(xml, &[(name, value)]).expect("the part must be readable"),
        };
        splice::apply(xml, &splices).expect("the splices must apply")
    }

    #[test]
    fn every_property_is_read_with_its_variant_and_a_value_of_that_type() {
        let read = read(&four_types()).expect("the part must be readable");

        assert_eq!(
            read,
            vec![
                Property {
                    name: "Stamp.Text".to_owned(),
                    variant: "lpwstr".to_owned(),
                    value: Value::Text("xlsplice".to_owned()),
                },
                Property {
                    name: "Stamp.Number".to_owned(),
                    variant: "i4".to_owned(),
                    value: Value::Integer(42),
                },
                Property {
                    name: "Stamp.Flag".to_owned(),
                    variant: "bool".to_owned(),
                    value: Value::Bool(true),
                },
                Property {
                    name: "Stamp.Date".to_owned(),
                    variant: "filetime".to_owned(),
                    value: Value::Moment("2026-09-11T10:00:00Z".to_owned()),
                },
            ]
        );
    }

    /// The variant is reported as found whatever it is, and a value xlsplice
    /// cannot type is the text the package holds.
    #[test]
    fn a_variant_that_is_not_one_of_the_four_is_reported_as_its_text() {
        let xml = part(&format!(
            "{}{}",
            property(2, "Blob", "blob", "AAECAw=="),
            property(3, "Wrong", "i4", "not a number"),
        ));

        let read = read(&xml).expect("the part must be readable");

        assert_eq!(read[0].variant, "blob");
        assert_eq!(read[0].value, Value::Text("AAECAw==".to_owned()));
        assert_eq!(read[1].variant, "i4", "the variant is still what it says");
        assert_eq!(read[1].value, Value::Text("not a number".to_owned()));
    }

    #[test]
    fn a_property_with_no_name_or_no_value_is_no_property() {
        let xml = part(
            r#"<property pid="2"><vt:lpwstr>x</vt:lpwstr></property><property pid="3" name="Empty"/>"#,
        );

        assert_eq!(read(&xml).expect("readable"), Vec::new());
    }

    /// Only what the property holds is written over, so its identifier, its
    /// name and every other byte of the part stay where they were.
    #[test]
    fn replacing_a_property_keeps_its_identifier_and_moves_nothing_else() {
        let xml = four_types();

        let written = set_in(&xml, "Stamp.Number", &Value::number(7.0));

        assert_eq!(
            written,
            xml.replace("<vt:i4>42</vt:i4>", "<vt:i4>7</vt:i4>")
        );
    }

    #[test]
    fn a_property_that_is_not_there_is_appended_with_the_next_identifier() {
        let xml = four_types();

        let written = set_in(&xml, "Stamp.New", &Value::Text("added".to_owned()));

        assert_eq!(
            written,
            xml.replace(
                "</Properties>",
                &format!(
                    "{}</Properties>",
                    property(6, "Stamp.New", "lpwstr", "added")
                )
            )
        );
    }

    #[test]
    fn the_first_property_of_an_empty_part_takes_the_first_identifier() {
        let xml = part("");

        let written = set_in(&xml, "Only", &Value::Text("one".to_owned()));

        assert_eq!(written, part(&property(2, "Only", "lpwstr", "one")));
    }

    /// Each of the four write types the command line offers, as the part
    /// spells it.
    #[test]
    fn each_write_type_goes_in_as_the_variant_excel_would_have_written() {
        for (value, expected) in [
            (
                Value::Text("hello".to_owned()),
                "<vt:lpwstr>hello</vt:lpwstr>",
            ),
            (Value::number(42.0), "<vt:i4>42</vt:i4>"),
            (Value::number(2.5), "<vt:r8>2.5</vt:r8>"),
            (Value::number(3e9), "<vt:r8>3000000000</vt:r8>"),
            (Value::Bool(true), "<vt:bool>true</vt:bool>"),
            (Value::Bool(false), "<vt:bool>false</vt:bool>"),
            (
                Value::Moment("2026-09-11T10:00:00Z".to_owned()),
                "<vt:filetime>2026-09-11T10:00:00Z</vt:filetime>",
            ),
        ] {
            let written = set_in(&part(""), "P", &value);

            assert!(written.contains(expected), "{value:?}: {written}");
        }
    }

    /// A whole number is a 32-bit integer where it fits and a real where it
    /// does not, because that is the range the variant holds.
    #[test]
    fn a_whole_number_past_the_thirty_two_bit_variant_is_written_as_a_real() {
        assert_eq!(
            Value::number(i32::MAX as f64),
            Value::Integer(2_147_483_647)
        );
        assert_eq!(
            Value::number(i32::MIN as f64),
            Value::Integer(-2_147_483_648)
        );
        assert_eq!(
            Value::number(i32::MAX as f64 + 1.0),
            Value::Real(2_147_483_648.0)
        );
        assert_eq!(Value::number(-0.5), Value::Real(-0.5));
    }

    #[test]
    fn setting_a_property_to_what_it_already_says_asks_for_nothing() {
        let xml = four_types();

        assert_eq!(
            written(&xml, "Stamp.Text", &Value::Text("xlsplice".to_owned())).expect("readable"),
            Some(Vec::new())
        );
        assert_eq!(
            written(&xml, "Stamp.Number", &Value::number(42.0)).expect("readable"),
            Some(Vec::new())
        );
    }

    #[test]
    fn text_that_would_be_markup_is_escaped_in_the_value_and_in_the_name() {
        let written = set_in(&part(""), r#"A"B&C"#, &Value::Text("x<y".to_owned()));

        assert!(written.contains(r#"name="A&quot;B&amp;C""#), "{written}");
        assert!(
            written.contains("<vt:lpwstr>x&lt;y</vt:lpwstr>"),
            "{written}"
        );
    }

    #[test]
    fn unsetting_takes_the_property_out_and_leaves_the_others() {
        let xml = four_types();

        let splices = unset(&xml, "Stamp.Flag")
            .expect("readable")
            .expect("the property is there");

        assert_eq!(
            splice::apply(&xml, &splices).expect("the splices apply"),
            xml.replace(&property(4, "Stamp.Flag", "bool", "true"), "")
        );
    }

    #[test]
    fn unsetting_a_property_that_is_not_there_answers_that_it_is_not() {
        assert_eq!(unset(&four_types(), "Nope").expect("readable"), None);
    }

    /// Two properties differing only in case are two properties, as they are
    /// to Excel.
    #[test]
    fn a_name_is_matched_exactly() {
        let xml = four_types();

        assert_eq!(unset(&xml, "stamp.text").expect("readable"), None);
        assert!(set_in(&xml, "STAMP.TEXT", &Value::Text("x".to_owned())).contains("STAMP.TEXT"));
    }

    /// A part a re-serialising tool wrote prefixes everything, and what goes
    /// into it is prefixed the same way.
    #[test]
    fn a_part_written_with_other_prefixes_keeps_them() {
        let xml = format!(
            r#"<cp:Properties xmlns:cp="{CUSTOM_PROPERTIES_NS}" xmlns:v="{VARIANT_TYPES}"><cp:property fmtid="{FMTID}" pid="2" name="A"><v:lpwstr>1</v:lpwstr></cp:property></cp:Properties>"#
        );

        let written = set_in(&xml, "B", &Value::Text("2".to_owned()));

        assert!(written.contains(r#"<cp:property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="B"><v:lpwstr>2</v:lpwstr></cp:property>"#), "{written}");
    }

    /// A part binding the variant namespace nowhere gets the binding on the
    /// element written into it, rather than an element in no namespace.
    #[test]
    fn a_part_binding_no_variant_namespace_takes_the_binding_with_the_value() {
        let xml = format!(r#"<Properties xmlns="{CUSTOM_PROPERTIES_NS}"/>"#);

        let written = set_in(&xml, "A", &Value::Text("1".to_owned()));

        assert!(
            written.contains(&format!(
                r#"<vt:lpwstr xmlns:vt="{VARIANT_TYPES}">1</vt:lpwstr>"#
            )),
            "{written}"
        );
    }

    /// Several added at once run their identifiers on from each other, which
    /// is why they are added together rather than one at a time.
    #[test]
    fn several_properties_added_at_once_take_consecutive_identifiers() {
        let xml = four_types();
        let (one, two) = (
            Value::Text("a".to_owned()),
            Value::Moment("2026-09-11T10:00:00Z".to_owned()),
        );

        let splices = added(&xml, &[("First", &one), ("Second", &two)]).expect("readable");

        assert_eq!(
            splice::apply(&xml, &splices).expect("the splices apply"),
            xml.replace(
                "</Properties>",
                &format!(
                    "{}{}</Properties>",
                    property(6, "First", "lpwstr", "a"),
                    property(7, "Second", "filetime", "2026-09-11T10:00:00Z"),
                )
            )
        );
    }

    #[test]
    fn adding_nothing_asks_for_nothing() {
        assert_eq!(added(&four_types(), &[]).expect("readable"), Vec::new());
    }

    #[test]
    fn a_whole_part_written_for_a_package_with_none_is_the_part_excel_writes() {
        let bytes = part_holding(&[("Stamp.Text", &Value::Text("xlsplice".to_owned()))]);

        assert_eq!(
            String::from_utf8(bytes).expect("the part is UTF-8"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
             <Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\" \
             xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\">\
             <property fmtid=\"{D5CDD505-2E9C-101B-9397-08002B2CF9AE}\" pid=\"2\" name=\"Stamp.Text\">\
             <vt:lpwstr>xlsplice</vt:lpwstr></property></Properties>"
        );
    }

    #[test]
    fn a_whole_part_may_be_written_holding_several() {
        let (one, two) = (Value::number(1.0), Value::Bool(false));

        let bytes = part_holding(&[("A", &one), ("B", &two)]);

        let text = String::from_utf8(bytes).expect("the part is UTF-8");
        assert!(
            text.contains(r#"pid="2" name="A"><vt:i4>1</vt:i4>"#),
            "{text}"
        );
        assert!(
            text.contains(r#"pid="3" name="B"><vt:bool>false</vt:bool>"#),
            "{text}"
        );
    }

    #[test]
    fn a_part_that_is_not_a_properties_part_is_unreadable() {
        let err = read("<worksheet/>").expect_err("not a properties part");

        assert_eq!(err.code(), crate::error::ErrorCode::Unreadable);
    }

    /// An identifier a package spells oddly is not one to trust; the next one
    /// free is still above every one taken.
    #[test]
    fn the_next_identifier_is_above_every_one_taken() {
        let xml = part(
            r#"<property pid="9" name="A"><vt:lpwstr>1</vt:lpwstr></property><property name="B"><vt:lpwstr>2</vt:lpwstr></property>"#,
        );

        assert!(set_in(&xml, "C", &Value::Text("3".to_owned())).contains(r#"pid="10""#));
    }
}
