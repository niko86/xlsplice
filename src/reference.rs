//! Reference syntax: A1 addresses, sheet-name quoting, and what a defined
//! name's `refersTo` points at.
//!
//! This is the sixty lines ADR-0002 keeps out of a crate. Every reference the
//! tool accepts or reports passes through here, so one rule decides what an
//! address is: `$` is not significant, letters are matched case-insensitively,
//! and a sheet name is quoted on output only when it has to be.

use std::fmt;

/// The last column of the grid, `XFD`.
pub const MAX_COLUMN: u32 = 16_384;

/// The last row of the grid.
pub const MAX_ROW: u32 = 1_048_576;

/// One cell of a sheet, by column and row, both counted from one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cell {
    row: u32,
    column: u32,
}

impl Cell {
    /// A cell at a column and row, if both are on the grid.
    pub fn new(column: u32, row: u32) -> Option<Self> {
        ((1..=MAX_COLUMN).contains(&column) && (1..=MAX_ROW).contains(&row))
            .then_some(Cell { row, column })
    }

    /// The column, counted from one, so `A` is 1.
    pub fn column(self) -> u32 {
        self.column
    }

    /// The row, counted from one.
    pub fn row(self) -> u32 {
        self.row
    }

    /// Parse an A1 reference such as `A1` or `$XFD$1048576`, and nothing
    /// else: trailing text means this was never an address.
    pub fn parse(text: &str) -> Option<Self> {
        let (cell, rest) = read_cell(text)?;
        rest.is_empty().then_some(cell)
    }

    /// The cell in A1 form, without `$`.
    pub fn a1(self) -> String {
        let mut letters = String::new();
        let mut remaining = self.column;
        while remaining > 0 {
            let offset = (remaining - 1) % 26;
            letters.push(char::from(b'A' + offset as u8));
            remaining = (remaining - 1) / 26;
        }
        let mut a1: String = letters.chars().rev().collect();
        a1.push_str(&self.row.to_string());
        a1
    }
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.a1())
    }
}

/// One cell of one sheet: the thing a defined name resolves to, and the thing
/// a `Sheet!A1` token names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    /// The sheet, in the package's own spelling.
    pub sheet: String,
    /// The cell on that sheet.
    pub cell: Cell,
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}!{}", quote_sheet_name(&self.sheet), self.cell)
    }
}

/// A sheet name as it appears inside a reference: quoted, with the quotes
/// doubled inside, only when a bare name would not lex back to the same thing.
pub fn quote_sheet_name(name: &str) -> String {
    if lexes_back_bare(name) {
        return name.to_owned();
    }
    format!("\'{}\'", name.replace('\'', "\'\'"))
}

/// Whether writing `name` without quotes would read back as that sheet, and
/// not as a cell, a boolean, or an R1C1 letter.
fn lexes_back_bare(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_')
        && name.chars().all(is_bare_sheet_char)
        && Cell::parse(name).is_none()
        && !looks_like_r1c1(name)
        && !["TRUE", "FALSE"]
            .iter()
            .any(|word| name.eq_ignore_ascii_case(word))
}

/// Whether the name reads as an R1C1 reference. Excel quotes these even
/// though they are otherwise plain words: `R`, `C`, `R1`, `RC`, `R1C1`.
fn looks_like_r1c1(name: &str) -> bool {
    let rest = match name.bytes().next() {
        Some(b'R' | b'r') => &name[1..],
        Some(b'C' | b'c') => return name[1..].bytes().all(|byte| byte.is_ascii_digit()),
        _ => return false,
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let after = &rest[digits..];
    match after.bytes().next() {
        None => true,
        Some(b'C' | b'c') => after[1..].bytes().all(|byte| byte.is_ascii_digit()),
        _ => false,
    }
}

/// Whether `ch` may appear in a sheet name written without quotes.
fn is_bare_sheet_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '.'
}

/// What a defined name's `refersTo` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefersTo {
    /// A cell or range. `sheet` is the name as written, still to be matched
    /// against the package's sheets, and is absent when the reference carries
    /// no sheet of its own.
    Area {
        sheet: Option<String>,
        top_left: Cell,
    },
    /// The reference, or the first area of it, is `#REF!`.
    RefError,
    /// A literal: a number, a quoted string, a boolean, or an error value.
    Constant,
    /// Anything else: a function call, an operator, a three-dimensional or
    /// external reference. Not resolvable to one cell.
    Formula,
}

/// Classify a defined name's `refersTo`.
///
/// Only the first area is read, because the anchor is the top-left cell of
/// the first area; a union's later areas change nothing. Anything that is not
/// exactly one area, optionally followed by a union, is a formula: a
/// three-dimensional or external reference resolves to no single cell, and
/// saying so is better than guessing at one.
pub fn parse_refers_to(text: &str) -> RefersTo {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text).trim();

    // A deleted sheet writes its name as `#REF!` too, so the check comes
    // before the sheet prefix rather than inside the area.
    if text.starts_with("#REF!") {
        return RefersTo::RefError;
    }
    if let Some((area, rest)) = read_area(text)
        && (rest.is_empty() || rest.starts_with(','))
    {
        return area;
    }
    if is_constant(text) {
        RefersTo::Constant
    } else {
        RefersTo::Formula
    }
}

/// One end of an area, before it is known whether it stands alone or pairs
/// with another across a `:`.
#[derive(Debug, Clone, Copy)]
enum Endpoint {
    Cell(Cell),
    /// A whole column, as in the `$C` of `$C:$E`.
    Column(u32),
    /// A whole row, as in the `$4` of `$4:$9`.
    Row(u32),
    RefError,
}

/// What an area narrows to once both of its ends are known.
#[derive(Debug, Clone, Copy)]
enum Corner {
    Cell(Cell),
    RefError,
}

/// Read one area, and give back what follows it.
fn read_area(text: &str) -> Option<(RefersTo, &str)> {
    let (sheet, rest) = read_sheet_prefix(text);
    let (first, rest) = read_endpoint(rest)?;
    let (corner, rest) = match rest.strip_prefix(':') {
        Some(tail) => {
            let (second, rest) = read_endpoint(tail)?;
            (span(first, second)?, rest)
        }
        None => (alone(first)?, rest),
    };
    let area = match corner {
        Corner::RefError => RefersTo::RefError,
        Corner::Cell(top_left) => RefersTo::Area { sheet, top_left },
    };
    Some((area, rest))
}

/// An endpoint with no `:` after it. A lone column or row is a name or a
/// number, not a reference.
fn alone(end: Endpoint) -> Option<Corner> {
    match end {
        Endpoint::Cell(cell) => Some(Corner::Cell(cell)),
        Endpoint::RefError => Some(Corner::RefError),
        Endpoint::Column(_) | Endpoint::Row(_) => None,
    }
}

/// The top-left corner of the area between two endpoints. A range may be
/// written from any corner, so both ends are taken at their minimum, and the
/// two ends must be of the same kind.
fn span(first: Endpoint, second: Endpoint) -> Option<Corner> {
    Some(match (first, second) {
        (Endpoint::RefError, _) | (_, Endpoint::RefError) => Corner::RefError,
        (Endpoint::Cell(a), Endpoint::Cell(b)) => {
            Corner::Cell(Cell::new(a.column().min(b.column()), a.row().min(b.row()))?)
        }
        (Endpoint::Column(a), Endpoint::Column(b)) => Corner::Cell(Cell::new(a.min(b), 1)?),
        (Endpoint::Row(a), Endpoint::Row(b)) => Corner::Cell(Cell::new(1, a.min(b))?),
        _ => return None,
    })
}

fn read_endpoint(text: &str) -> Option<(Endpoint, &str)> {
    if let Some(rest) = text.strip_prefix("#REF!") {
        return Some((Endpoint::RefError, rest));
    }
    // A cell first, so that `A1` is not read as the column `A` with `1` left
    // over.
    if let Some((cell, rest)) = read_cell(text) {
        return Some((Endpoint::Cell(cell), rest));
    }
    if let Some((column, rest)) = read_column(text) {
        return Some((Endpoint::Column(column), rest));
    }
    let (row, rest) = read_row(text)?;
    Some((Endpoint::Row(row), rest))
}

/// Read the `Sheet!` in front of an area, quoted or bare, and give back the
/// name with its doubled quotes undone. A reference may carry no sheet, in
/// which case nothing is consumed.
fn read_sheet_prefix(text: &str) -> (Option<String>, &str) {
    if let Some(body) = text.strip_prefix('\'') {
        let mut name = String::new();
        let mut rest = body;
        loop {
            let Some(quote) = rest.find('\'') else {
                return (None, text);
            };
            name.push_str(&rest[..quote]);
            rest = &rest[quote + 1..];
            match rest.strip_prefix('\'') {
                Some(tail) => {
                    name.push('\'');
                    rest = tail;
                }
                None => {
                    return match rest.strip_prefix('!') {
                        Some(tail) => (Some(name), tail),
                        None => (None, text),
                    };
                }
            }
        }
    }

    let end = text
        .find(|ch: char| !is_bare_sheet_char(ch))
        .unwrap_or(text.len());
    match text[end..].strip_prefix('!') {
        Some(tail) if end > 0 => (Some(text[..end].to_owned()), tail),
        _ => (None, text),
    }
}

/// Whether the whole text is one quoted string, its own quotes doubled
/// inside. `"a"` is; `"a"&"b"` merely begins and ends with a quote.
fn is_string_literal(text: &str) -> bool {
    let Some(mut rest) = text.strip_prefix('"') else {
        return false;
    };
    loop {
        let Some(quote) = rest.find('"') else {
            return false;
        };
        rest = &rest[quote + 1..];
        match rest.strip_prefix('"') {
            Some(tail) => rest = tail,
            None => return rest.is_empty(),
        }
    }
}

fn read_cell(text: &str) -> Option<(Cell, &str)> {
    let (column, rest) = read_column(text)?;
    let (row, rest) = read_row(rest)?;
    Some((Cell::new(column, row)?, rest))
}

/// Read a column's letters, `$` and case both insignificant. `XFD` is three
/// letters, so a longer run is a word rather than a column.
fn read_column(text: &str) -> Option<(u32, &str)> {
    let rest = text.strip_prefix('$').unwrap_or(text);
    let letters = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let (head, tail) = rest.split_at(letters);
    let column = head.bytes().fold(0, |column, letter| {
        column * 26 + u32::from(letter.to_ascii_uppercase() - b'A') + 1
    });
    (column <= MAX_COLUMN).then_some((column, tail))
}

fn read_row(text: &str) -> Option<(u32, &str)> {
    let rest = text.strip_prefix('$').unwrap_or(text);
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let (head, tail) = rest.split_at(digits);
    let row: u32 = head.parse().ok()?;
    (1..=MAX_ROW).contains(&row).then_some((row, tail))
}

/// Whether the whole text is one literal value. `#REF!` is taken before this
/// is reached, so the remaining `#` literals are error constants.
fn is_constant(text: &str) -> bool {
    if is_string_literal(text) {
        return true;
    }
    // An array constant: `{1,2,3}`, `{"a","b"}`, `{1;2}`.
    if text.len() >= 2 && text.starts_with('{') && text.ends_with('}') {
        return true;
    }
    if text.starts_with('#') {
        return true;
    }
    if ["TRUE", "FALSE"]
        .iter()
        .any(|word| text.eq_ignore_ascii_case(word))
    {
        return true;
    }
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E'))
        && text.parse::<f64>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(column: u32, row: u32) -> Cell {
        Cell::new(column, row).expect("the test asks for a cell on the grid")
    }

    fn area(sheet: &str, column: u32, row: u32) -> RefersTo {
        RefersTo::Area {
            sheet: Some(sheet.to_owned()),
            top_left: cell(column, row),
        }
    }

    #[test]
    fn an_a1_reference_parses_to_its_column_and_row() {
        for (text, column, row) in [
            ("A1", 1, 1),
            ("B2", 2, 2),
            ("Z1", 26, 1),
            ("AA1", 27, 1),
            ("XFD1048576", MAX_COLUMN, MAX_ROW),
        ] {
            assert_eq!(Cell::parse(text), Cell::new(column, row), "{text}");
        }
    }

    #[test]
    fn dollars_are_not_significant_and_letters_are_case_insensitive() {
        let plain = Cell::parse("B2");
        for text in ["$B$2", "B$2", "$B2", "b2", "$b$2"] {
            assert_eq!(Cell::parse(text), plain, "{text}");
        }
    }

    #[test]
    fn a_reference_off_the_grid_or_malformed_is_not_a_cell() {
        for text in [
            "", "A", "1", "A0", "XFE1", "A1048577", "AAAA1", "A1:B2", "A 1", "1A", "A1x", "$",
            "$$A$1",
        ] {
            assert_eq!(Cell::parse(text), None, "{text} must not parse");
        }
    }

    #[test]
    fn a_cell_renders_back_to_the_a1_it_came_from() {
        for text in ["A1", "B2", "Z100", "AA1", "XFD1048576"] {
            assert_eq!(Cell::parse(text).expect(text).a1(), text);
        }
    }

    #[test]
    fn a_sheet_name_is_quoted_only_when_a_bare_name_would_not_lex_back() {
        for bare in ["Sheet1", "Data", "_hidden", "a.b", "Ünïcode"] {
            assert_eq!(quote_sheet_name(bare), bare, "{bare} needs no quotes");
        }
        for (name, quoted) in [
            ("My Sheet", "'My Sheet'"),
            ("It's", "'It''s'"),
            ("2024", "'2024'"),
            ("A1", "'A1'"),
            ("TRUE", "'TRUE'"),
            ("C", "'C'"),
            ("a-b", "'a-b'"),
            ("", "''"),
        ] {
            assert_eq!(quote_sheet_name(name), quoted, "{name}");
        }
    }

    #[test]
    fn an_address_renders_with_its_sheet_quoted_as_needed() {
        let plain = Address {
            sheet: "Sheet1".to_owned(),
            cell: cell(2, 2),
        };
        assert_eq!(plain.to_string(), "Sheet1!B2");
        let spaced = Address {
            sheet: "My Sheet".to_owned(),
            cell: cell(1, 1),
        };
        assert_eq!(spaced.to_string(), "'My Sheet'!A1");
    }

    #[test]
    fn a_refers_to_naming_one_cell_gives_that_cell() {
        assert_eq!(parse_refers_to("Sheet1!$A$1"), area("Sheet1", 1, 1));
        assert_eq!(parse_refers_to("Sheet1!A1"), area("Sheet1", 1, 1));
        assert_eq!(parse_refers_to("=Sheet1!$A$1"), area("Sheet1", 1, 1));
        assert_eq!(parse_refers_to("  Sheet1!$A$1  "), area("Sheet1", 1, 1));
    }

    #[test]
    fn a_range_resolves_to_its_top_left_however_it_is_written() {
        assert_eq!(parse_refers_to("Sheet1!$B$2:$D$5"), area("Sheet1", 2, 2));
        assert_eq!(
            parse_refers_to("Sheet1!$D$5:$B$2"),
            area("Sheet1", 2, 2),
            "a range written backwards still has its top-left corner"
        );
        assert_eq!(parse_refers_to("Sheet1!$B$5:$D$2"), area("Sheet1", 2, 2));
    }

    #[test]
    fn a_whole_column_or_row_anchors_at_the_grids_edge() {
        assert_eq!(parse_refers_to("Sheet1!$C:$E"), area("Sheet1", 3, 1));
        assert_eq!(parse_refers_to("Sheet1!$4:$9"), area("Sheet1", 1, 4));
    }

    #[test]
    fn a_quoted_sheet_name_is_unquoted_with_its_doubled_quotes_undone() {
        assert_eq!(parse_refers_to("'My Sheet'!$A$1"), area("My Sheet", 1, 1));
        assert_eq!(parse_refers_to("'It''s'!$A$1"), area("It's", 1, 1));
        assert_eq!(parse_refers_to("'2024'!$A$1"), area("2024", 1, 1));
    }

    #[test]
    fn a_union_resolves_to_the_first_areas_top_left() {
        assert_eq!(
            parse_refers_to("Sheet1!$C$3,Sheet1!$A$1"),
            area("Sheet1", 3, 3)
        );
    }

    #[test]
    fn a_reference_without_a_sheet_carries_no_sheet() {
        assert_eq!(
            parse_refers_to("$A$1"),
            RefersTo::Area {
                sheet: None,
                top_left: cell(1, 1),
            }
        );
    }

    #[test]
    fn a_ref_error_is_reported_as_one_wherever_it_appears() {
        for text in ["#REF!", "Sheet1!#REF!", "#REF!#REF!", "'Gone'!#REF!"] {
            assert_eq!(parse_refers_to(text), RefersTo::RefError, "{text}");
        }
    }

    #[test]
    fn a_literal_is_a_constant() {
        for text in [
            "42", "-1.5", "1E+10", "\"text\"", "\"\"", "TRUE", "FALSE", "true", "#N/A", "#VALUE!",
            "#DIV/0!",
        ] {
            assert_eq!(parse_refers_to(text), RefersTo::Constant, "{text}");
        }
    }

    #[test]
    fn an_array_constant_is_a_constant() {
        for text in ["{1,2,3}", r#"{"a","b"}"#, "{1;2}"] {
            assert_eq!(parse_refers_to(text), RefersTo::Constant, "{text}");
        }
    }

    #[test]
    fn a_string_constant_may_hold_its_own_quotes_doubled() {
        assert_eq!(parse_refers_to(r#""say ""hi""""#), RefersTo::Constant);
    }

    #[test]
    fn text_joined_by_an_operator_is_a_formula_however_it_begins_and_ends() {
        for text in [r#""a"&"b""#, r#""a"&Sheet1!$A$1"#, r#""unterminated"#] {
            assert_eq!(parse_refers_to(text), RefersTo::Formula, "{text}");
        }
    }

    #[test]
    fn a_sheet_name_that_reads_as_an_r1c1_reference_is_quoted() {
        for name in ["R", "C", "R1", "C1", "RC", "R1C1", "RC1", "R1C", "r1c1"] {
            assert_eq!(quote_sheet_name(name), format!("'{name}'"), "{name}");
        }
        for name in ["Rate", "Cost", "Region", "Rc_1", "Custom"] {
            assert_eq!(
                quote_sheet_name(name),
                name,
                "{name} is a word, not a reference"
            );
        }
    }

    #[test]
    fn anything_that_is_not_a_plain_area_or_a_literal_is_a_formula() {
        for text in [
            "SUM(Sheet1!$A$1:$A$9)",
            "OFFSET(Sheet1!$A$1,0,0,10,1)",
            "Sheet1:Sheet3!$A$1",
            "[1]Sheet1!$A$1",
            "Sheet1!$A$1*2",
            "Sheet1!$A$1 Sheet1!$B$1",
            "Other",
            "",
        ] {
            assert_eq!(parse_refers_to(text), RefersTo::Formula, "{text}");
        }
    }
}
