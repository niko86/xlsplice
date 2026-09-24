//! One worksheet part: which part holds a sheet's cells, and what one cell
//! stores.
//!
//! Nothing here interprets a value. A cell is reported as the part holds it:
//! the stored type, the exact raw text, the formula with the role its
//! attributes give it, and the style index. Turning a shared-string index
//! into text needs the shared string table, which is another part, so that
//! belongs to the caller in [`crate::cells`].
//!
//! Two absences are not the same absence. A row the part holds, with no cell
//! element for the column asked for, is an absent cell and reads as empty; a
//! row the part does not hold at all means the cell lies outside every row
//! there is, and the caller has nothing to report. To a write the difference
//! is what has to be put in and where: a cell into its row, or a row holding
//! it into the sheet data.

use std::io::{Read, Seek};

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::package::Package;
use crate::reference::Cell;
use crate::relationships::Relationships;
use crate::splice::{Element, Splice, appended, inserted_before};
use crate::strings::string_item_text;
use crate::workbook::Workbook;
use crate::xml::{children, escape, number_text, text_of};

/// The type a cell's value is stored as, spelled as the part spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredType {
    /// A number. What a cell with no type attribute holds.
    Number,
    /// An index into the shared string table.
    Shared,
    /// A string a formula produced.
    FormulaString,
    /// A string held in the cell itself.
    InlineString,
    /// A boolean, stored as `1` or `0`.
    Bool,
    /// An error value, such as `#DIV/0!`.
    ErrorValue,
    /// An ISO 8601 date, stored as text rather than a serial.
    Date,
    /// No value at all: the cell is absent, or holds none.
    Empty,
}

impl StoredType {
    /// The type as the part spells it; `empty` for a cell that stores no
    /// value, which the schema has no spelling for because it is an absence.
    pub fn as_str(self) -> &'static str {
        match self {
            StoredType::Number => "n",
            StoredType::Shared => "s",
            StoredType::FormulaString => "str",
            StoredType::InlineString => "inlineStr",
            StoredType::Bool => "b",
            StoredType::ErrorValue => "e",
            StoredType::Date => "d",
            StoredType::Empty => "empty",
        }
    }

    /// The type a cell's `t` attribute declares.
    fn declared(attribute: &str, at: Cell) -> Result<Self> {
        Ok(match attribute {
            "n" => StoredType::Number,
            "s" => StoredType::Shared,
            "str" => StoredType::FormulaString,
            "inlineStr" => StoredType::InlineString,
            "b" => StoredType::Bool,
            "e" => StoredType::ErrorValue,
            "d" => StoredType::Date,
            other => {
                return Err(Error::unreadable(format!(
                    "cell {at} has type '{other}'; the stored types are \
                     n, s, str, inlineStr, b, e and d"
                )));
            }
        })
    }
}

/// What a formula is to the cells around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormulaRole {
    /// The cell's own formula, shared with nothing.
    Plain,
    /// The one cell of a shared group that carries the formula text; the
    /// others in its range take their formula from it.
    SharedMaster,
    /// A cell that takes its formula from its group's master.
    SharedChild,
    /// A formula entered over a range as an array.
    Array,
    /// A what-if data table.
    DataTable,
}

impl FormulaRole {
    /// The role as it is reported, stable and snake-case.
    pub fn as_str(self) -> &'static str {
        match self {
            FormulaRole::Plain => "plain",
            FormulaRole::SharedMaster => "shared_master",
            FormulaRole::SharedChild => "shared_child",
            FormulaRole::Array => "array",
            FormulaRole::DataTable => "data_table",
        }
    }
}

/// A cell's formula, with what its attributes make of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formula {
    /// The formula text as stored, without a leading `=`. A shared child
    /// stores none of its own, and carries the empty string.
    pub text: String,
    /// What the formula is to the cells around it.
    pub role: FormulaRole,
    /// The range a shared master or an array formula covers.
    pub range: Option<String>,
    /// The shared group a master or a child belongs to.
    pub group: Option<u32>,
}

impl Formula {
    fn of(node: Node, at: Cell) -> Result<Self> {
        let range = node.attribute("ref").map(str::to_owned);
        let role = match node.attribute("t").unwrap_or("normal") {
            "normal" => FormulaRole::Plain,
            "array" => FormulaRole::Array,
            "dataTable" => FormulaRole::DataTable,
            // Only the master of a group carries the range its children sit
            // in, so the range is what tells the two apart.
            "shared" if range.is_some() => FormulaRole::SharedMaster,
            "shared" => FormulaRole::SharedChild,
            other => {
                return Err(Error::unreadable(format!(
                    "cell {at} has a formula of type '{other}'; the types are \
                     normal, shared, array and dataTable"
                )));
            }
        };
        Ok(Formula {
            text: text_of(node),
            role,
            range,
            group: match node.attribute("si") {
                None => None,
                Some(si) => Some(si.parse().map_err(|_| {
                    Error::unreadable(format!(
                        "cell {at} has a formula in group '{si}', which is not a group index"
                    ))
                })?),
            },
        })
    }
}

/// One cell, exactly as the part holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// The type the value is stored as.
    pub kind: StoredType,
    /// The exact stored text: the contents of the value element, which for a
    /// shared-string cell is the index. A cell storing no value has none.
    pub raw: Option<String>,
    /// The cell's formula, if it has one.
    pub formula: Option<Formula>,
    /// The style index. A cell with no style attribute carries the default
    /// format, which is index 0.
    pub style: u32,
}

/// What looking for a cell in a worksheet part turned up.
///
/// Two answers, not three: a cell is there or it is not. Why it is not —
/// the row holds no such cell, the sheet data holds no such row, the part
/// holds no sheet data at all — is a distinction a read has no use for, and
/// one a caller cannot act on either, since a template carries an element
/// only for the cells something is already in. The write path does need it,
/// and takes it from [`Located`], where the element a new cell would go into
/// is the answer rather than an afterthought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The cell element, and what it stores.
    Cell(Stored),
    /// The cell is not there. An absent cell stores nothing and carries no
    /// style of its own.
    Absent,
}

/// The same answers, in terms of the element rather than its contents.
///
/// A write needs the element, because a splice is taken from where the
/// element sits in the part; a read needs only what it holds. Where the cell
/// is not there, what comes back is the element a new one would go into, so
/// that a write has somewhere to put it without looking twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Located<'a, 'input> {
    /// The cell element.
    Cell(Node<'a, 'input>),
    /// The row is there and the cell is not: the row is what a new cell goes
    /// into.
    InRow(Node<'a, 'input>),
    /// The part holds no row of that number: the sheet data is what a new row
    /// goes into.
    InSheetData(Node<'a, 'input>),
    /// The part has no sheet data element, so there is nowhere for a cell to
    /// be and nowhere to put one.
    Nowhere,
}

/// The cells of one sheet, ready to be looked in.
#[derive(Debug)]
pub struct Worksheet<'a, 'input> {
    data: Option<Node<'a, 'input>>,
    /// The column definitions, which say what style a column gives the cells
    /// in it that carry none of their own.
    cols: Option<Node<'a, 'input>>,
}

impl<'a, 'input> Worksheet<'a, 'input> {
    /// Take the sheet data of a parsed worksheet part.
    ///
    /// A part with no sheet data element has no rows, and every cell in it is
    /// outside every row; it is not rejected here, because the read path has
    /// nothing to say about it that [`Found::NoRow`] does not say already.
    pub fn of(document: &'a Document<'input>) -> Result<Self> {
        let root = document.root_element();
        if root.tag_name().name() != "worksheet" {
            return Err(Error::unreadable(format!(
                "the worksheet part's root element is <{}>, not <worksheet>",
                root.tag_name().name()
            )));
        }
        Ok(Worksheet {
            data: children(root, "sheetData").next(),
            cols: children(root, "cols").next(),
        })
    }

    /// Look `cell` up and read what it stores.
    pub fn cell(&self, cell: Cell) -> Result<Found> {
        Ok(match self.locate(cell)? {
            Located::Cell(node) => Found::Cell(stored(node, cell)?),
            // The row holds no such cell, the sheet data holds no such row,
            // or the part holds no sheet data: three ways of not being there,
            // and one answer to a read.
            Located::InRow(_) | Located::InSheetData(_) | Located::Nowhere => Found::Absent,
        })
    }

    /// Find the element `cell` sits in, or the one it would go into, without
    /// reading either.
    pub fn locate(&self, cell: Cell) -> Result<Located<'a, 'input>> {
        let Some(data) = self.data else {
            return Ok(Located::Nowhere);
        };
        let Some(row) = self.row(cell.row())? else {
            return Ok(Located::InSheetData(data));
        };
        Ok(match cell_in(row, cell)? {
            Some(node) => Located::Cell(node),
            None => Located::InRow(row),
        })
    }

    /// The style Excel would give a cell at `at` that the part does not hold.
    ///
    /// A row declaring a custom format gives its style to every cell in it,
    /// whatever the columns say; failing that, a column definition covering
    /// the column gives its style; failing both, the cell carries none and
    /// takes the default format, as every cell declaring no style does.
    pub fn style_of_an_absent_cell(&self, at: Cell) -> Result<Option<u32>> {
        if let Some(row) = self.row(at.row())?
            && truthy(row.attribute("customFormat"))
            && let Some(style) = number(row.attribute("s"))
        {
            return Ok(Some(style));
        }
        Ok(self.column_style(at.column()))
    }

    /// The style the column definitions give `column`, if any covers it.
    fn column_style(&self, column: u32) -> Option<u32> {
        let cols = self.cols?;
        children(cols, "col")
            .find(|col| {
                let (min, max) = (number(col.attribute("min")), number(col.attribute("max")));
                match (min, max) {
                    (Some(min), Some(max)) => (min..=max).contains(&column),
                    _ => false,
                }
            })
            .and_then(|col| number(col.attribute("style")))
    }

    /// The row element numbered `wanted`. A row declares its number; one that
    /// does not follows the row before it, which is the rule the schema gives
    /// for an omitted index. A row that declares one and spells it wrongly is
    /// a part disagreeing with itself, and is not quietly counted past.
    fn row(&self, wanted: u32) -> Result<Option<Node<'a, 'input>>> {
        let Some(data) = self.data else {
            return Ok(None);
        };
        let mut number = 0;
        for row in children(data, "row") {
            number = match row.attribute("r") {
                None => number + 1,
                Some(r) => r.parse().map_err(|_| {
                    Error::unreadable(format!("a row is numbered '{r}', which is not a row"))
                })?,
            };
            if number == wanted {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    /// The sheet data a row numbered `row` is to be put into, or why it is
    /// not to be put in at all.
    ///
    /// Two reasons, and both belong to the part rather than to whoever asked.
    /// A part with no sheet data element has nowhere to put a row, which is
    /// the same absence a cell in it runs into. A part that already holds the
    /// row is one nothing should be asking this of: a second row of the same
    /// number is a part disagreeing with itself, so it is refused here rather
    /// than spliced in beside the row that is there.
    fn where_a_row_goes(&self, row: u32) -> Result<Node<'a, 'input>> {
        let Some(data) = self.data else {
            return Err(Error::not_found(format!(
                "there is no row {row} and nowhere to put one: the worksheet part has no \
                 sheet data element, so it holds no rows. A worksheet part without one is \
                 not a sheet Excel wrote."
            )));
        };
        match self.row(row)? {
            None => Ok(data),
            Some(_) => Err(Error::internal(format!(
                "row {row} was to be put into the sheet, and the sheet holds it already"
            ))),
        }
    }
}

/// Parse a worksheet part's text.
///
/// The one place a worksheet part is handed to the parser, so that what a
/// failure says about one is said once. Which part of the package it was is
/// no part of this — a part's text does not carry its path — and is added by
/// whoever read it out of the container.
pub fn parsed(xml: &str) -> Result<Document<'_>> {
    Document::parse(xml)
        .map_err(|err| Error::unreadable(format!("the worksheet part is not valid XML: {err}")))
}

/// A whole number an attribute spells, or nothing where it spells none or
/// spells something that is not one. A part disagreeing with itself about a
/// style index is not worth refusing a write over: the cell simply takes no
/// style from it, which is what a cell carrying none takes anyway.
fn number(attribute: Option<&str>) -> Option<u32> {
    attribute?.parse().ok()
}

/// Whether an attribute says yes, in the two spellings the schema's boolean
/// allows.
fn truthy(attribute: Option<&str>) -> bool {
    matches!(attribute, Some("1" | "true"))
}

/// The element for a cell the part does not hold, written the way Excel
/// writes one: `r`, then `s`, then `t`, which is the order the schema gives
/// its attributes.
fn cell_element(at: Cell, style: Option<u32>, written: &Written, prefix: &str) -> String {
    let style = match style {
        Some(index) => format!(r#" s="{index}""#),
        None => String::new(),
    };
    let kind = match written.declares() {
        Some(kind) => format!(r#" t="{kind}""#),
        None => String::new(),
    };
    let content = written.content(prefix);
    let opening = format!(r#"<{prefix}c r="{}"{style}{kind}"#, at.a1());
    match content.is_empty() {
        true => format!("{opening}/>"),
        false => format!("{opening}>{content}</{prefix}c>"),
    }
}

/// The splice that puts a cell for `at` into `row`, before the first cell of
/// a greater column and after every cell of a lesser one.
///
/// `source` must be the text the row's document was parsed from.
pub fn cell_inserted(
    row: Node,
    source: &str,
    at: Cell,
    style: Option<u32>,
    written: &Written,
) -> Result<Splice> {
    let element = cell_element(at, style, written, Element::of(row, source)?.prefix());
    let splice = match first_cell_after(row, at)? {
        Some(next) => inserted_before(next, source, &element),
        None => appended(row, source, &element)?,
    };
    // Two cells put into one row go in front of the same cell, so the row
    // reads in column order because they say so and not because of the order
    // the batch happened to name them in.
    Ok(splice.ordered(at.column()))
}

/// A cell to go into a row that is being put into the sheet.
///
/// A cell going into a row that is already there is spliced straight in by
/// [`cell_inserted`]; this is what a cell answers with instead when the row
/// itself has to be put in, because how many cells the new row holds is not
/// something one cell can know.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCell {
    /// Where it goes.
    pub at: Cell,
    /// The style Excel would show it under, or none.
    pub style: Option<u32>,
    /// What goes in it.
    pub written: Written,
}

/// The splice that puts a row numbered `row` and holding `cells` into the
/// worksheet part whose text is `xml`, before the first row of a greater
/// number and after every row of a lesser one.
///
/// The part is parsed here rather than handed in parsed. Where a row goes is
/// a question about the part and nothing else, so everything it takes to
/// answer is here: the parse, the root element the part must have, and the
/// sheet data the row goes into. The splice's byte ranges are ranges of
/// `xml`, which is the text every other edit to this part is ranged against.
///
/// The cells go in in column order whatever order they are given in, which is
/// the order a row has to read in. The row carries its number and nothing
/// else: `spans` is a hint about which columns a row holds, and Excel neither
/// needs it nor minds a row without one; the rows already there keep the spans
/// they have, because a splice changes what it was told to and nothing else.
pub fn row_inserted(xml: &str, row: u32, cells: &[NewCell]) -> Result<Splice> {
    let document = parsed(xml)?;
    let data = Worksheet::of(&document)?.where_a_row_goes(row)?;
    let prefix = Element::of(data, xml)?.prefix();
    let mut ordered: Vec<&NewCell> = cells.iter().collect();
    ordered.sort_by_key(|cell| cell.at.column());
    let mut written = String::new();
    for cell in ordered {
        written.push_str(&cell_element(cell.at, cell.style, &cell.written, prefix));
    }
    let element = format!(r#"<{prefix}row r="{row}">{written}</{prefix}row>"#);
    let splice = match first_row_after(data, row)? {
        Some(next) => inserted_before(next, xml, &element),
        None => appended(data, xml, &element)?,
    };
    // Two rows put into one sheet go in at the same byte where both follow
    // the same row, so they order themselves by number.
    Ok(splice.ordered(row))
}

/// The first cell of `row` in a column greater than `wanted`'s, which is the
/// cell a new one goes in front of. Counted the way [`cell_in`] counts, so a
/// cell declaring no address is where the same rule puts it.
fn first_cell_after<'a, 'input>(
    row: Node<'a, 'input>,
    wanted: Cell,
) -> Result<Option<Node<'a, 'input>>> {
    let mut column = 0;
    for node in children(row, "c") {
        let at = match node.attribute("r") {
            Some(r) => Cell::parse(r).ok_or_else(|| {
                Error::unreadable(format!("a cell is addressed '{r}', which is not a cell"))
            })?,
            None => match Cell::new(column + 1, wanted.row()) {
                Some(at) => at,
                None => return Ok(None),
            },
        };
        column = at.column();
        if column > wanted.column() {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

/// The first row of `data` numbered above `wanted`, which is the row a new one
/// goes in front of. Counted the way [`Worksheet::row`] counts.
fn first_row_after<'a, 'input>(
    data: Node<'a, 'input>,
    wanted: u32,
) -> Result<Option<Node<'a, 'input>>> {
    let mut number = 0;
    for row in children(data, "row") {
        number = match row.attribute("r") {
            None => number + 1,
            Some(r) => r.parse().map_err(|_| {
                Error::unreadable(format!("a row is numbered '{r}', which is not a row"))
            })?,
        };
        if number > wanted {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// The cell element for `wanted` within `row`. A cell declares its address;
/// one that does not follows the cell before it in the row. A cell that
/// declares an address which is not one is a part disagreeing with itself,
/// and is not quietly counted past.
fn cell_in<'a, 'input>(row: Node<'a, 'input>, wanted: Cell) -> Result<Option<Node<'a, 'input>>> {
    let mut column = 0;
    for node in children(row, "c") {
        let at = match node.attribute("r") {
            Some(r) => Cell::parse(r).ok_or_else(|| {
                Error::unreadable(format!("a cell is addressed '{r}', which is not a cell"))
            })?,
            None => match Cell::new(column + 1, wanted.row()) {
                Some(at) => at,
                // Past the last column of the grid there is no cell left to
                // be the one wanted, and none after it either.
                None => return Ok(None),
            },
        };
        column = at.column();
        if at == wanted {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

/// Whether a declared type can hold the empty string as a value of its own.
///
/// The two text types can: a formula that produced `""` and an inline string
/// holding nothing are both cells with a value, and the value is `""`. No
/// other type can, so an empty value element declaring one of those stores
/// nothing at all rather than a value its type would refuse.
fn holds_text(declared: &str) -> bool {
    matches!(declared, "str" | "inlineStr")
}

/// What one cell element stores.
fn stored(node: Node, at: Cell) -> Result<Stored> {
    let declared = node.attribute("t").unwrap_or("n");
    // An inline string is held in the cell rather than in a value element,
    // so where the text comes from depends on the declared type.
    let raw = match declared {
        "inlineStr" => children(node, "is").next().map(string_item_text),
        _ => children(node, "v").next().map(text_of),
    };
    // A value element holding nothing means one thing or the other by the
    // type the cell declares, and only the type tells the two apart. Excel
    // writes `<v/>` on a `str` cell for a formula that calculated to the
    // empty string, which is a value and the commonest shape in the corpus;
    // openpyxl writes `<v></v>` untyped for a formula it never calculated,
    // which is not. So empty text is a value where the declared type can
    // hold one, and no value where it cannot: `""` is not a number, a
    // boolean, an error code, a date or a shared-string index (ADR-0007).
    let raw = raw.filter(|text| !text.is_empty() || holds_text(declared));
    Ok(Stored {
        // A cell with no value stores no type either, whatever it declares.
        kind: match raw {
            None => StoredType::Empty,
            Some(_) => StoredType::declared(declared, at)?,
        },
        raw,
        formula: formula_of(node, at)?,
        style: match node.attribute("s") {
            None => 0,
            Some(index) => index.parse().map_err(|_| {
                Error::unreadable(format!(
                    "cell {at} has style '{index}', which is not a style index"
                ))
            })?,
        },
    })
}

/// A value on its way into a cell, in the form it will be stored in.
///
/// The three the write path knows are the three write types a caller may ask
/// for. Reading a caller's text into one belongs to
/// [`WriteType`](crate::batch::WriteType), in the batch, with the workbook
/// open: whether a caller's text is a number is no part of a worksheet's
/// business, and a date is a number only once the workbook's date system has
/// had its say.
#[derive(Debug, Clone, PartialEq)]
pub enum Written {
    /// A number, stored without a type attribute: what a cell with no `t`
    /// holds.
    Number(f64),
    /// Text, stored in the cell as an inline string so that the shared string
    /// table is never a target (ADR-0001).
    Text(String),
    /// A boolean, stored as `1` or `0`.
    Bool(bool),
    /// Nothing at all: the cell keeps its element and its style and loses its
    /// value, its type and its formula. What `clear` writes, and what Excel
    /// leaves behind when a cell's contents are deleted.
    Nothing,
}

impl Written {
    /// Text, which is whatever was given, whitespace and all.
    pub fn text(text: &str) -> Self {
        Written::Text(text.to_owned())
    }

    /// The `t` attribute the cell carries once this is in it, or `None` for a
    /// number, which declares no type, and for nothing at all, which has none
    /// to declare.
    fn declares(&self) -> Option<&'static str> {
        match self {
            Written::Number(_) | Written::Nothing => None,
            Written::Text(_) => Some(StoredType::InlineString.as_str()),
            Written::Bool(_) => Some(StoredType::Bool.as_str()),
        }
    }

    /// The splice that puts this between the cell's tags, or closes them over
    /// nothing where there is nothing to put there.
    fn between(&self, element: &Element) -> Option<Splice> {
        match self {
            Written::Nothing => element.empty_splice(),
            _ => Some(element.content_splice(&self.content(element.prefix()))),
        }
    }

    /// What goes between the cell's tags, written with the same namespace
    /// prefix the cell itself carries. Nothing at all goes between no tags,
    /// which is [`Written::between`]'s business rather than this one's.
    fn content(&self, prefix: &str) -> String {
        match self {
            Written::Number(number) => format!("<{prefix}v>{}</{prefix}v>", number_text(*number)),
            Written::Bool(yes) => format!("<{prefix}v>{}</{prefix}v>", u8::from(*yes)),
            Written::Text(text) => format!(
                "<{prefix}is><{prefix}t{}>{}</{prefix}t></{prefix}is>",
                space_attribute(text),
                escape(text)
            ),
            Written::Nothing => String::new(),
        }
    }
}

/// `xml:space="preserve"`, where the text has whitespace at an end that a
/// reader would otherwise be free to drop. Excel writes the attribute under
/// the same rule, so text written back unchanged is written back byte for
/// byte.
fn space_attribute(text: &str) -> &'static str {
    let padded = text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace);
    if padded {
        " xml:space=\"preserve\""
    } else {
        ""
    }
}

/// The formula the cell element carries, if it carries one.
pub fn formula_of(node: Node, at: Cell) -> Result<Option<Formula>> {
    children(node, "f")
        .next()
        .map(|node| Formula::of(node, at))
        .transpose()
}

/// The splices that put `written` into the cell element `node`.
///
/// Two at most, and both inside the element: the type attribute the value
/// needs, added, changed or taken away, and everything between the tags. Every
/// other attribute the cell carries is left where it is, because nothing here
/// touches a byte outside those two places. So a cleared cell keeps its style,
/// and a cell already saying what it is asked for takes no splice at all.
///
/// `source` must be the text the node's document was parsed from.
pub fn value_splices(node: Node<'_, '_>, source: &str, written: &Written) -> Result<Vec<Splice>> {
    let element = Element::of(node, source)?;
    let mut splices = Vec::new();
    // The schema orders a cell's attributes r, s, t, so a type the cell does
    // not yet declare goes in after whichever of the first two it carries.
    if let Some(splice) = element.attribute_splice("t", written.declares(), &["r", "s"]) {
        splices.push(splice);
    }
    splices.extend(written.between(&element));
    Ok(splices)
}

/// The part holding the cells of `sheet`, which is named in the package's own
/// spelling.
///
/// The relationship is the only thing that says which part is which sheet,
/// so a sheet whose relationship is missing or dangling is not found rather
/// than guessed at. A package may hold one workbook part and one shared
/// string table, which is why those two fall back to where Excel puts them;
/// it holds a worksheet part per sheet, and their names follow the order the
/// sheets were created rather than the order their tabs sit in. Guessing
/// `sheet1.xml` for the first tab would hand back another sheet's cells on
/// any package whose sheets have been reordered, and a wrong answer is worse
/// than no answer.
pub fn part_of_sheet<R: Read + Seek>(
    package: &Package<R>,
    rels: &Relationships,
    workbook: &Workbook,
    sheet: &str,
) -> Result<String> {
    workbook
        .sheet_named(sheet)
        .and_then(|found| found.rel_id.as_deref())
        .and_then(|id| rels.part_for_id(id))
        .filter(|part| package.has_part(part))
        .ok_or_else(|| {
            Error::not_found(format!(
                "sheet '{sheet}' has no worksheet part in {}: the workbook part does not point \
                 at one, or points at a part the package does not hold.",
                package.name()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    fn sheet(body: &str) -> String {
        format!(r#"<worksheet xmlns="{NS}"><sheetData>{body}</sheetData></worksheet>"#)
    }

    fn at(xml: &str, a1: &str) -> Result<Found> {
        let document = Document::parse(xml).expect("the test part must parse");
        Worksheet::of(&document)?.cell(Cell::parse(a1).expect("the test asks for a cell"))
    }

    fn found(xml: &str, a1: &str) -> Found {
        at(xml, a1).expect("the test cell must be readable")
    }

    fn stored_at(xml: &str, a1: &str) -> Stored {
        match found(xml, a1) {
            Found::Cell(stored) => stored,
            other => panic!("{a1} is {other:?}, not a cell"),
        }
    }

    #[test]
    fn a_cell_with_no_type_attribute_holds_a_number() {
        let stored = stored_at(&sheet(r#"<row r="1"><c r="A1"><v>2.5</v></c></row>"#), "A1");

        assert_eq!(stored.kind, StoredType::Number);
        assert_eq!(stored.raw.as_deref(), Some("2.5"));
        assert_eq!(stored.style, 0);
        assert_eq!(stored.formula, None);
    }

    #[test]
    fn every_stored_type_comes_back_with_its_own_spelling_and_raw_text() {
        let body = r#"<row r="1">
            <c r="A1"><v>1</v></c>
            <c r="B1" t="s"><v>7</v></c>
            <c r="C1" t="str"><f>A1</f><v>text</v></c>
            <c r="D1" t="inlineStr"><is><t>inline</t></is></c>
            <c r="E1" t="b"><v>1</v></c>
            <c r="F1" t="e"><v>#DIV/0!</v></c>
            <c r="G1" t="d"><v>2026-09-11T00:00:00</v></c>
          </row>"#;
        let xml = sheet(body);
        let seen: Vec<_> = ["A1", "B1", "C1", "D1", "E1", "F1", "G1"]
            .into_iter()
            .map(|a1| {
                let stored = stored_at(&xml, a1);
                (stored.kind.as_str(), stored.raw.unwrap_or_default())
            })
            .collect();

        assert_eq!(
            seen,
            [
                ("n", "1".to_owned()),
                ("s", "7".to_owned()),
                ("str", "text".to_owned()),
                ("inlineStr", "inline".to_owned()),
                ("b", "1".to_owned()),
                ("e", "#DIV/0!".to_owned()),
                ("d", "2026-09-11T00:00:00".to_owned()),
            ]
        );
    }

    #[test]
    fn the_runs_of_an_inline_string_are_concatenated() {
        let stored = stored_at(
            &sheet(
                r#"<row r="1"><c r="A1" t="inlineStr"><is><r><t>in</t></r><r><t>line</t></r></is></c></row>"#,
            ),
            "A1",
        );

        assert_eq!(stored.raw.as_deref(), Some("inline"));
    }

    #[test]
    fn a_cell_holding_no_value_is_empty_and_still_carries_its_style() {
        let stored = stored_at(&sheet(r#"<row r="1"><c r="A1" s="3"/></row>"#), "A1");

        assert_eq!(stored.kind, StoredType::Empty);
        assert_eq!(stored.raw, None);
        assert_eq!(stored.style, 3);
    }

    #[test]
    fn a_formula_with_no_cached_value_is_empty_and_still_carries_its_formula() {
        let stored = stored_at(
            &sheet(r#"<row r="1"><c r="A1"><f>SUM(B1:B9)</f></c></row>"#),
            "A1",
        );

        assert_eq!(stored.kind, StoredType::Empty);
        assert_eq!(
            stored.formula.expect("the cell has a formula").text,
            "SUM(B1:B9)"
        );
    }

    #[test]
    fn an_empty_value_element_stores_no_value_either() {
        // openpyxl's spelling of an uncalculated formula: the element is
        // there, holds nothing, and the cell declares no type. `""` is not a
        // number, so this stores no value rather than a number whose text is
        // not a number. The guard on ADR-0007's other half.
        let stored = stored_at(
            &sheet(r#"<row r="1"><c r="A1"><f>SUM(B1:B9)</f><v></v></c></row>"#),
            "A1",
        );

        assert_eq!(stored.kind, StoredType::Empty);
        assert_eq!(stored.raw, None);
        assert_eq!(
            stored.formula.expect("the cell has a formula").text,
            "SUM(B1:B9)"
        );
    }

    #[test]
    fn a_str_cell_whose_value_element_is_empty_holds_the_empty_string() {
        // Excel's spelling of a formula that calculated to `""`, and the
        // commonest cell shape in the corpus. Present and empty is a value,
        // and `str` is a type that can hold it.
        let stored = stored_at(
            &sheet(r#"<row r="1"><c r="B1" t="str"><f>IF(ISNUMBER(A1),1,"")</f><v/></c></row>"#),
            "B1",
        );

        assert_eq!(stored.kind, StoredType::FormulaString);
        assert_eq!(stored.raw.as_deref(), Some(""));
        assert_eq!(
            stored.formula.expect("the cell has a formula").text,
            r#"IF(ISNUMBER(A1),1,"")"#
        );
    }

    #[test]
    fn an_inline_string_holding_nothing_is_the_empty_string() {
        let stored = stored_at(
            &sheet(r#"<row r="1"><c r="A1" t="inlineStr"><is><t></t></is></c></row>"#),
            "A1",
        );

        assert_eq!(stored.kind, StoredType::InlineString);
        assert_eq!(stored.raw.as_deref(), Some(""));
    }

    #[test]
    fn an_empty_value_element_stores_no_value_for_a_type_that_cannot_hold_it() {
        // Every type but the two text ones: `""` is not a number, a boolean,
        // an error code, a date or a shared-string index, so the cell stores
        // nothing rather than a value its type would refuse.
        for declared in ["n", "b", "e", "d", "s"] {
            let stored = stored_at(
                &sheet(&format!(
                    r#"<row r="1"><c r="A1" t="{declared}"><v/></c></row>"#
                )),
                "A1",
            );

            assert_eq!(stored.kind, StoredType::Empty, "t={declared}");
            assert_eq!(stored.raw, None, "t={declared}");
        }
    }

    #[test]
    fn a_value_element_holding_only_spaces_is_that_text() {
        // Not the same thing: a string cell may hold whitespace on purpose,
        // so only an element holding nothing at all stores no value.
        let stored = stored_at(
            &sheet(r#"<row r="1"><c r="A1" t="str"><v>  </v></c></row>"#),
            "A1",
        );

        assert_eq!(stored.raw.as_deref(), Some("  "));
    }

    #[test]
    fn a_plain_formula_has_no_range_and_no_group() {
        let formula = stored_at(
            &sheet(r#"<row r="1"><c r="A1"><f>SUM(B1:B9)</f><v>9</v></c></row>"#),
            "A1",
        )
        .formula
        .expect("the cell has a formula");

        assert_eq!(formula.role, FormulaRole::Plain);
        assert_eq!(formula.role.as_str(), "plain");
        assert_eq!(formula.range, None);
        assert_eq!(formula.group, None);
    }

    #[test]
    fn a_shared_master_carries_its_range_and_a_child_only_its_group() {
        let xml = sheet(
            r#"<row r="1"><c r="E1"><f t="shared" ref="E1:E5" si="0">A1*2</f><v>2</v></c></row>
               <row r="2"><c r="E2"><f t="shared" si="0"/><v>4</v></c></row>"#,
        );

        let master = stored_at(&xml, "E1").formula.expect("a formula");
        assert_eq!(master.role, FormulaRole::SharedMaster);
        assert_eq!(master.text, "A1*2");
        assert_eq!(master.range.as_deref(), Some("E1:E5"));
        assert_eq!(master.group, Some(0));

        let child = stored_at(&xml, "E2").formula.expect("a formula");
        assert_eq!(child.role, FormulaRole::SharedChild);
        assert_eq!(child.text, "", "a child stores no formula text of its own");
        assert_eq!(child.range, None);
        assert_eq!(child.group, Some(0));
    }

    #[test]
    fn an_array_and_a_data_table_keep_their_own_roles() {
        let xml = sheet(
            r#"<row r="1">
                 <c r="A1"><f t="array" ref="A1:A3">SUM(B1:B3*C1:C3)</f><v>6</v></c>
                 <c r="B1"><f t="dataTable" ref="B1:B3" dt2D="0"/><v>1</v></c>
               </row>"#,
        );

        assert_eq!(
            stored_at(&xml, "A1").formula.expect("a formula").role,
            FormulaRole::Array
        );
        assert_eq!(
            stored_at(&xml, "B1")
                .formula
                .expect("a formula")
                .role
                .as_str(),
            "data_table"
        );
    }

    /// A cell the row does not hold and a cell whose row is not there are
    /// both absent, because the difference is one a read cannot use and a
    /// caller cannot act on. Where the difference matters is a write, and a
    /// write asks [`Worksheet::locate`], which keeps all three answers apart.
    #[test]
    fn a_cell_is_absent_whether_its_row_holds_no_such_cell_or_there_is_no_such_row() {
        let xml = sheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#);

        assert_eq!(found(&xml, "Z1"), Found::Absent);
        assert_eq!(found(&xml, "A9"), Found::Absent);
    }

    #[test]
    fn a_part_with_no_sheet_data_has_no_rows_at_all() {
        let xml = format!(r#"<worksheet xmlns="{NS}"/>"#);

        assert_eq!(found(&xml, "A1"), Found::Absent);
    }

    #[test]
    fn rows_and_cells_that_declare_no_index_follow_the_one_before_them() {
        let xml = sheet(r#"<row><c><v>1</v></c><c><v>2</v></c></row><row><c><v>3</v></c></row>"#);

        assert_eq!(stored_at(&xml, "B1").raw.as_deref(), Some("2"));
        assert_eq!(stored_at(&xml, "A2").raw.as_deref(), Some("3"));
    }

    #[test]
    fn a_prefixed_worksheet_reads_the_same_as_an_unprefixed_one() {
        let xml = format!(
            r#"<x:worksheet xmlns:x="{NS}"><x:sheetData><x:row r="1"><x:c r="A1" t="s"><x:v>4</x:v></x:c></x:row></x:sheetData></x:worksheet>"#
        );

        assert_eq!(stored_at(&xml, "A1").raw.as_deref(), Some("4"));
    }

    #[test]
    fn a_part_that_is_not_a_worksheet_is_unreadable() {
        let document = Document::parse("<workbook/>").expect("the test part must parse");
        let err = Worksheet::of(&document).expect_err("a workbook is not a worksheet");

        assert_eq!(err.code(), ErrorCode::Unreadable);
    }

    #[test]
    fn an_index_a_part_declares_and_spells_wrongly_is_unreadable() {
        // Leaving an index out is allowed and means "the next one"; spelling
        // one wrongly is the part disagreeing with itself, and counting past
        // it would answer with the wrong cell.
        for body in [
            r#"<row r="one"><c r="A1"><v>1</v></c></row>"#,
            r#"<row r="1"><c r="A"><v>1</v></c></row>"#,
            r#"<row r="1"><c r="A1"><f t="shared" si="first"/><v>1</v></c></row>"#,
        ] {
            let err = at(&sheet(body), "A1").expect_err(body);
            assert_eq!(err.code(), ErrorCode::Unreadable, "{body}");
        }
    }

    /// The one cell of `xml` with `written` in it.
    fn written_into(xml: &str, a1: &str, written: &Written) -> String {
        let document = Document::parse(xml).expect("the test part must parse");
        let at = Cell::parse(a1).expect("the test asks for a cell");
        let node = match Worksheet::of(&document)
            .expect("a worksheet")
            .locate(at)
            .expect("the cell must be locatable")
        {
            Located::Cell(node) => node,
            other => panic!("{a1} is {other:?}, not a cell"),
        };
        let splices = value_splices(node, xml, written).expect("the cell must take the value");
        crate::splice::apply(xml, &splices).expect("the splices must apply")
    }

    #[test]
    fn a_number_is_written_with_no_type_attribute_and_the_old_one_taken_away() {
        assert_eq!(
            written_into(
                &sheet(r#"<row r="1"><c r="A1" s="2" t="s"><v>0</v></c></row>"#),
                "A1",
                &Written::Number(42.0),
            ),
            sheet(r#"<row r="1"><c r="A1" s="2"><v>42</v></c></row>"#),
        );
    }

    #[test]
    fn text_is_written_as_an_inline_string_and_the_style_is_kept() {
        assert_eq!(
            written_into(
                &sheet(r#"<row r="1"><c r="B1" s="1" t="s"><v>0</v></c></row>"#),
                "B1",
                &Written::text("hello"),
            ),
            sheet(r#"<row r="1"><c r="B1" s="1" t="inlineStr"><is><t>hello</t></is></c></row>"#),
        );
    }

    #[test]
    fn text_with_whitespace_at_an_end_says_so_and_text_without_does_not() {
        for (text, expected) in [
            (" padded ", r#"<t xml:space="preserve"> padded </t>"#),
            ("plain", r#"<t>plain</t>"#),
        ] {
            let written = written_into(
                &sheet(r#"<row r="1"><c r="A1"/></row>"#),
                "A1",
                &Written::text(text),
            );
            assert!(written.contains(expected), "{text:?} became {written}");
        }
    }

    #[test]
    fn text_that_would_be_markup_is_escaped_and_a_carriage_return_survives() {
        let written = written_into(
            &sheet(r#"<row r="1"><c r="A1"/></row>"#),
            "A1",
            &Written::text("a<b&c>d\re"),
        );

        assert!(
            written.contains("<t>a&lt;b&amp;c&gt;d&#13;e</t>"),
            "{written}"
        );
    }

    #[test]
    fn a_boolean_is_written_as_the_schema_spells_one() {
        assert_eq!(
            written_into(
                &sheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#),
                "A1",
                &Written::Bool(false),
            ),
            sheet(r#"<row r="1"><c r="A1" t="b"><v>0</v></c></row>"#),
        );
    }

    #[test]
    fn a_cell_holding_nothing_is_opened_to_take_a_value_and_keeps_its_style() {
        assert_eq!(
            written_into(
                &sheet(r#"<row r="2"><c r="B2" s="4"/></row>"#),
                "B2",
                &Written::text("filled"),
            ),
            sheet(r#"<row r="2"><c r="B2" s="4" t="inlineStr"><is><t>filled</t></is></c></row>"#),
        );
    }

    #[test]
    fn writing_the_value_already_there_changes_no_byte_at_all() {
        let xml = sheet(r#"<row r="1"><c r="A1" s="2"><v>2.5</v></c></row>"#);

        assert_eq!(written_into(&xml, "A1", &Written::Number(2.5)), xml);
    }

    #[test]
    fn a_prefixed_part_takes_its_value_in_the_same_prefix() {
        let xml = format!(
            r#"<x:worksheet xmlns:x="{NS}"><x:sheetData><x:row r="1"><x:c r="A1"><x:v>1</x:v></x:c></x:row></x:sheetData></x:worksheet>"#
        );

        assert!(
            written_into(&xml, "A1", &Written::text("hi"))
                .contains(r#"<x:c r="A1" t="inlineStr"><x:is><x:t>hi</x:t></x:is></x:c>"#),
            "{}",
            written_into(&xml, "A1", &Written::text("hi"))
        );
    }

    #[test]
    fn a_number_is_written_in_the_shortest_form_that_reads_back_as_itself() {
        for (number, expected) in [
            (42.0, "42"),
            (-3.0, "-3"),
            (2.5, "2.5"),
            (0.0, "0"),
            (-0.0, "0"),
            (1e-7, "0.0000001"),
            (0.1 + 0.2, "0.30000000000000004"),
        ] {
            assert_eq!(number_text(number), expected, "{number}");
        }
        // Shortest is worth nothing if it does not still read back as itself.
        for number in [42.0, 2.5, 1e-7, 1e300, f64::MIN, 0.1 + 0.2] {
            assert_eq!(
                number_text(number)
                    .parse::<f64>()
                    .expect("the written form must parse"),
                number,
                "{number}"
            );
        }
    }

    #[test]
    fn a_type_a_style_or_a_formula_role_outside_the_schema_is_unreadable() {
        for body in [
            r#"<row r="1"><c r="A1" t="picture"><v>1</v></c></row>"#,
            r#"<row r="1"><c r="A1" s="none"><v>1</v></c></row>"#,
            r#"<row r="1"><c r="A1"><f t="magic">A2</f><v>1</v></c></row>"#,
        ] {
            let err = at(&sheet(body), "A1").expect_err(body);
            assert_eq!(err.code(), ErrorCode::Unreadable, "{body}");
        }
    }

    /// A whole worksheet, with column definitions before its sheet data.
    fn sheet_with_cols(cols: &str, body: &str) -> String {
        format!(
            r#"<worksheet xmlns="{NS}"><cols>{cols}</cols><sheetData>{body}</sheetData></worksheet>"#
        )
    }

    /// The part with a cell for `a1` holding `written` put into it.
    fn inserted(xml: &str, a1: &str, written: &Written) -> String {
        let document = Document::parse(xml).expect("the test part must parse");
        let sheet = Worksheet::of(&document).expect("the test part is a worksheet");
        let cell = Cell::parse(a1).expect("the test asks for a cell");
        let style = sheet
            .style_of_an_absent_cell(cell)
            .expect("the styles must be readable");
        let splice = match sheet.locate(cell).expect("the cell must be locatable") {
            Located::InRow(row) => {
                cell_inserted(row, xml, cell, style, written).expect("the row is spliceable")
            }
            Located::InSheetData(_) => row_inserted(
                xml,
                cell.row(),
                &[NewCell {
                    at: cell,
                    style,
                    written: written.clone(),
                }],
            )
            .expect("the sheet data is spliceable"),
            other => panic!("{a1} is {other:?}, not an absence"),
        };
        crate::splice::apply(xml, &[splice]).expect("the splice applies")
    }

    /// The three places a cell can go in a row it is missing from.
    #[test]
    fn a_cell_goes_into_its_row_in_column_order() {
        let row = r#"<row r="1"><c r="B1"><v>1</v></c><c r="D1"><v>2</v></c></row>"#;
        let xml = sheet(row);

        for (a1, expected) in [
            (
                "A1",
                r#"<row r="1"><c r="A1"><v>9</v></c><c r="B1"><v>1</v></c><c r="D1"><v>2</v></c></row>"#,
            ),
            (
                "C1",
                r#"<row r="1"><c r="B1"><v>1</v></c><c r="C1"><v>9</v></c><c r="D1"><v>2</v></c></row>"#,
            ),
            (
                "E1",
                r#"<row r="1"><c r="B1"><v>1</v></c><c r="D1"><v>2</v></c><c r="E1"><v>9</v></c></row>"#,
            ),
        ] {
            assert_eq!(
                inserted(&xml, a1, &Written::Number(9.0)),
                sheet(expected),
                "{a1} went in the wrong place, or moved something else"
            );
        }
    }

    /// A row goes into the sheet data in row order, and carries its number
    /// and nothing else.
    #[test]
    fn a_row_goes_into_the_sheet_data_in_row_order() {
        let xml = sheet(
            r#"<row r="2"><c r="A2"><v>1</v></c></row><row r="4"><c r="A4"><v>2</v></c></row>"#,
        );

        for (a1, expected) in [
            (
                "A1",
                r#"<sheetData><row r="1"><c r="A1"><v>9</v></c></row><row r="2">"#,
            ),
            (
                "B3",
                r#"</row><row r="3"><c r="B3"><v>9</v></c></row><row r="4">"#,
            ),
            (
                "A9",
                r#"</row><row r="9"><c r="A9"><v>9</v></c></row></sheetData>"#,
            ),
        ] {
            let written = inserted(&xml, a1, &Written::Number(9.0));

            assert!(written.contains(expected), "{a1}: {written}");
        }
    }

    #[test]
    fn the_first_row_of_an_empty_sheet_data_element_opens_it() {
        for empty in ["<sheetData/>", "<sheetData></sheetData>"] {
            let xml = format!(r#"<worksheet xmlns="{NS}">{empty}</worksheet>"#);

            let written = inserted(&xml, "B2", &Written::text("hello"));

            assert_eq!(
                written,
                format!(
                    r#"<worksheet xmlns="{NS}"><sheetData><row r="2"><c r="B2" t="inlineStr">"#,
                ) + r#"<is><t>hello</t></is></c></row></sheetData></worksheet>"#,
                "{empty}"
            );
        }
    }

    /// Putting a row in reads the part itself, so what is wrong with the part
    /// is answered here rather than by whoever called. The caller used to
    /// parse, find the sheet data and hand the node over, which meant it also
    /// had to invent an answer for a part that would not parse and for one
    /// that already held the row.
    #[test]
    fn putting_a_row_in_answers_for_the_part_it_is_given() {
        let cells = [NewCell {
            at: Cell::parse("A1").expect("A1 is a cell"),
            style: None,
            written: Written::Number(9.0),
        }];

        let unparseable = row_inserted("<worksheet><sheetData>", 1, &cells)
            .expect_err("that part is not valid XML");
        let not_a_worksheet =
            row_inserted(r#"<sst xmlns="x"/>"#, 1, &cells).expect_err("that is not a worksheet");
        let held = row_inserted(
            &sheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#),
            1,
            &cells,
        )
        .expect_err("the sheet holds row 1 already");

        assert_eq!(unparseable.code(), ErrorCode::Unreadable);
        assert!(
            unparseable.message().contains("the worksheet part"),
            "{unparseable}"
        );
        assert_eq!(not_a_worksheet.code(), ErrorCode::Unreadable);
        assert_eq!(held.code(), ErrorCode::Internal, "{held}");
    }

    /// A row declaring a custom format gives its style to the cells in it,
    /// and it wins over a column definition covering the same cell.
    #[test]
    fn a_cell_in_a_row_with_a_custom_format_takes_the_rows_style() {
        let xml = sheet_with_cols(
            r#"<col min="1" max="3" style="7"/>"#,
            r#"<row r="1" s="4" customFormat="1"><c r="C1"><v>1</v></c></row>"#,
        );

        assert!(
            inserted(&xml, "A1", &Written::Number(9.0)).contains(r#"<c r="A1" s="4">"#),
            "the row's style wins over the column's"
        );
    }

    #[test]
    fn a_cell_under_a_styled_column_takes_the_columns_style() {
        let xml = sheet_with_cols(
            r#"<col min="2" max="4" style="7"/>"#,
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
        );

        assert!(inserted(&xml, "C1", &Written::Number(9.0)).contains(r#"<c r="C1" s="7">"#));
        assert!(
            inserted(&xml, "E1", &Written::Number(9.0)).contains(r#"<c r="E1">"#),
            "a column outside every definition gives no style"
        );
    }

    /// A row carrying a style but not saying it is custom-formatted is a row
    /// whose style Excel does not apply to its cells.
    #[test]
    fn a_row_style_without_a_custom_format_gives_the_cell_nothing() {
        let xml = sheet(r#"<row r="1" s="4"><c r="C1"><v>1</v></c></row>"#);

        assert!(inserted(&xml, "A1", &Written::Number(9.0)).contains(r#"<c r="A1">"#));
    }

    /// The style comes from the row a new row would have, which is no row at
    /// all, so only the columns have anything to say.
    #[test]
    fn a_cell_in_a_row_that_is_not_there_takes_only_the_columns_style() {
        let xml = sheet_with_cols(
            r#"<col min="1" max="1" style="7"/>"#,
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
        );

        assert!(inserted(&xml, "A5", &Written::Number(9.0)).contains(r#"<c r="A5" s="7">"#));
    }

    #[test]
    fn a_part_with_no_sheet_data_element_is_nowhere_to_put_a_cell() {
        let xml = format!(r#"<worksheet xmlns="{NS}"><dimension ref="A1"/></worksheet>"#);
        let document = Document::parse(&xml).expect("the test part must parse");
        let sheet = Worksheet::of(&document).expect("the test part is a worksheet");

        assert_eq!(
            sheet
                .locate(Cell::parse("A1").expect("a cell"))
                .expect("locating must not fail"),
            Located::Nowhere
        );
        assert_eq!(
            sheet
                .cell(Cell::parse("A1").expect("a cell"))
                .expect("readable"),
            Found::Absent,
            "to a read it is the same absence as a row that is not there"
        );
    }

    /// A part a re-serialising tool prefixed takes cells and rows prefixed
    /// the same way.
    #[test]
    fn what_is_written_in_takes_the_prefix_the_part_uses() {
        let xml = format!(
            r#"<x:worksheet xmlns:x="{NS}"><x:sheetData><x:row r="1"><x:c r="B1"><x:v>1</x:v></x:c></x:row></x:sheetData></x:worksheet>"#
        );

        assert!(
            inserted(&xml, "A1", &Written::Number(9.0))
                .contains(r#"<x:c r="A1"><x:v>9</x:v></x:c>"#),
            "{}",
            inserted(&xml, "A1", &Written::Number(9.0))
        );
        assert!(
            inserted(&xml, "A2", &Written::Number(9.0))
                .contains(r#"<x:row r="2"><x:c r="A2"><x:v>9</x:v></x:c></x:row>"#)
        );
    }
}
