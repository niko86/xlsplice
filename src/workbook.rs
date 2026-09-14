//! The workbook model: which sheets a package has, and what its defined names
//! point at.
//!
//! Everything here is read out of one part, `xl/workbook.xml`, found through
//! the package's root relationships. Sheets keep their container order, which
//! is the order Excel shows their tabs in. A defined name is resolved as it is
//! read, because resolving needs the sheet list and the sheet list is right
//! here; what it resolves to is its [`Anchor`](crate::reference::Address), or
//! the reason it has none.

use std::io::{Read, Seek};

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::package::Package;
use crate::reference::{Address, RefersTo, parse_refers_to};
use crate::relationships::{OFFICE_DOCUMENT, Relationships, part_or_conventional};
use crate::xml::{children, text_of};

/// Where the workbook part sits in every package Excel writes, used when the
/// root relationships do not name one.
const CONVENTIONAL_WORKBOOK: &str = "xl/workbook.xml";

/// Whether a sheet's tab is shown, and if not, how thoroughly it is hidden.
/// The names are the ones the file itself uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetState {
    /// The tab is shown.
    Visible,
    /// The tab is hidden and a user can unhide it.
    Hidden,
    /// The tab is hidden and only the VBA editor can unhide it.
    VeryHidden,
}

impl SheetState {
    /// The state as the package spells it, which is also how xlsplice
    /// reports it.
    pub fn as_str(self) -> &'static str {
        match self {
            SheetState::Visible => "visible",
            SheetState::Hidden => "hidden",
            SheetState::VeryHidden => "veryHidden",
        }
    }
}

/// One sheet of a workbook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    /// The name, in the package's own spelling.
    pub name: String,
    /// Whether the tab is shown.
    pub state: SheetState,
    /// The relationship naming the part that holds the sheet's cells. A
    /// package another tool has mangled can lose it.
    pub rel_id: Option<String>,
    /// The number the workbook gives the sheet, which is what the calc chain
    /// calls it. Nothing else in the package refers to a sheet this way, and
    /// it is not the sheet's place in the workbook: a sheet moved keeps its
    /// number.
    pub sheet_id: Option<u32>,
}

/// Where a defined name can be seen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Visible from the whole workbook.
    Workbook,
    /// Visible from one sheet, named in the package's own spelling.
    Sheet(String),
}

/// Why a defined name resolves to no cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoAnchor {
    /// The reference is `#REF!`: whatever it named has been deleted.
    RefError,
    /// The name holds a literal value rather than a reference.
    Constant,
    /// The name holds a formula, or a reference no single cell can stand for,
    /// such as a three-dimensional or external one.
    Formula,
    /// The reference names a sheet the package does not have, or names none.
    UnknownSheet,
}

impl NoAnchor {
    /// The reason as it is reported, stable and snake-case.
    pub fn as_str(self) -> &'static str {
        match self {
            NoAnchor::RefError => "ref_error",
            NoAnchor::Constant => "constant",
            NoAnchor::Formula => "formula",
            NoAnchor::UnknownSheet => "unknown_sheet",
        }
    }
}

/// What a defined name points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The one cell the name stands for.
    Anchor(Address),
    /// No cell, and why.
    Unresolvable(NoAnchor),
}

/// One defined name, with what it refers to and what that resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedName {
    /// The name as declared.
    pub name: String,
    /// Where the name can be seen from.
    pub scope: Scope,
    /// The reference text exactly as the package holds it.
    pub refers_to: String,
    /// The anchor, or the reason there is none.
    pub resolved: Resolved,
}

/// Which day a workbook counts its date serials from.
///
/// A date is stored as a number, and which date a number is depends on the
/// workbook: the two systems are 1462 days apart. Nothing but the workbook
/// part says which is in force, which is why a date cannot be read into a
/// serial until the package is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateSystem {
    /// The default, and what Excel writes unless told otherwise. Serial 1 is
    /// 1900-01-01, and serial 60 is a 29th of February 1900 that never
    /// happened: Lotus 1-2-3 had it and Excel kept it for compatibility, so
    /// every serial from 61 onwards is one greater than the true count of
    /// days.
    #[default]
    Date1900,
    /// What `date1904` in the workbook part asks for, and what Excel for Mac
    /// wrote for years. Serial 0 is 1904-01-01 and there is no phantom day.
    Date1904,
}

impl DateSystem {
    /// The system as the workbook part names it: the value `date1904` would
    /// carry.
    pub fn as_str(self) -> &'static str {
        match self {
            DateSystem::Date1900 => "1900",
            DateSystem::Date1904 => "1904",
        }
    }
}

/// A package's sheets, defined names and date system.
#[derive(Debug)]
pub struct Workbook {
    part: String,
    sheets: Vec<Sheet>,
    defined_names: Vec<DefinedName>,
    dates: DateSystem,
}

impl Workbook {
    /// Read the workbook part out of `package` and build the model.
    pub fn read<R: Read + Seek>(package: &mut Package<R>) -> Result<Self> {
        let part = workbook_part_path(package)?;
        let xml = package.read_part_text(&part)?;
        Workbook::parse_part(xml, &part).map_err(|err| err.within(package.name()))
    }

    /// Build the model from the text of a workbook part.
    ///
    /// The part path a model built this way carries is the conventional one,
    /// because the text alone does not say where it came from. Only
    /// [`Workbook::read`] knows that, and it says so.
    pub fn parse(xml: &str) -> Result<Self> {
        Workbook::parse_part(xml, CONVENTIONAL_WORKBOOK)
    }

    /// Build the model from the text of the workbook part at `part`.
    fn parse_part(xml: &str, part: &str) -> Result<Self> {
        let document = Document::parse(xml).map_err(|err| {
            Error::unreadable(format!("the workbook part is not valid XML: {err}"))
        })?;
        let root = document.root_element();
        if root.tag_name().name() != "workbook" {
            return Err(Error::unreadable(format!(
                "the workbook part\'s root element is <{}>, not <workbook>",
                root.tag_name().name()
            )));
        }
        let sheets = read_sheets(root)?;
        let defined_names = read_defined_names(root, &sheets)?;
        Ok(Workbook {
            part: part.to_owned(),
            sheets,
            defined_names,
            dates: read_date_system(root),
        })
    }

    /// The part the workbook was read from. Every relationship the workbook
    /// owns, to a worksheet or to the shared string table, resolves against
    /// it.
    pub fn part(&self) -> &str {
        &self.part
    }

    /// Every sheet, in workbook order.
    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    /// Which day the workbook counts its date serials from.
    pub fn dates(&self) -> DateSystem {
        self.dates
    }

    /// Every defined name, in the order the package declares them.
    pub fn defined_names(&self) -> &[DefinedName] {
        &self.defined_names
    }

    /// The sheet called `name`, matched the way Excel matches it: without
    /// regard to case. The sheet that comes back carries the package's own
    /// spelling.
    pub fn sheet_named(&self, name: &str) -> Option<&Sheet> {
        sheet_named_in(&self.sheets, name)
    }

    /// The defined name called `name` and visible from `scope`, matched the
    /// way Excel matches it: without regard to case. A scope is exact, so a
    /// workbook-scoped lookup never finds a sheet-scoped name, or the other
    /// way about.
    pub fn name_in_scope(&self, name: &str, scope: &Scope) -> Option<&DefinedName> {
        let wanted = name.to_lowercase();
        self.defined_names
            .iter()
            .find(|defined| defined.scope == *scope && defined.name.to_lowercase() == wanted)
    }

    /// Every defined name visible from `scope`, in declaration order, for a
    /// message that has to say what was there instead.
    pub fn names_in_scope(&self, scope: &Scope) -> Vec<&str> {
        self.defined_names
            .iter()
            .filter(|defined| defined.scope == *scope)
            .map(|defined| defined.name.as_str())
            .collect()
    }
}

/// The path of the workbook part: what the root relationships point the
/// main document at, falling back to where Excel always puts it for a
/// package whose root relationships are missing or silent.
fn workbook_part_path<R: Read + Seek>(package: &mut Package<R>) -> Result<String> {
    let named = Relationships::read(package, "")?.part_of_kind(OFFICE_DOCUMENT);
    part_or_conventional(package, named, CONVENTIONAL_WORKBOOK).ok_or_else(|| {
        Error::unreadable(format!(
            "{} holds no workbook part: it is a zip container, but not an Excel package.",
            package.name()
        ))
    })
}

/// The date system the workbook declares.
///
/// A workbook that says nothing is on the 1900 system, which is what Excel
/// writes unless the setting was changed. The flag is a boolean the schema
/// spells `1` or `0`, and Excel also writes `true`; anything else is not the
/// flag being set, so it is not an error, it is simply not 1904.
fn read_date_system(root: Node) -> DateSystem {
    let asked = children(root, "workbookPr")
        .next()
        .and_then(|node| node.attribute("date1904"))
        .is_some_and(|flag| matches!(flag, "1" | "true"));
    match asked {
        true => DateSystem::Date1904,
        false => DateSystem::Date1900,
    }
}

fn read_sheets(root: Node) -> Result<Vec<Sheet>> {
    let Some(sheets) = children(root, "sheets").next() else {
        return Ok(Vec::new());
    };
    children(sheets, "sheet")
        .map(|node| {
            let name = node.attribute("name").ok_or_else(|| {
                Error::unreadable("a <sheet> in the workbook part has no name attribute")
            })?;
            let state = match node.attribute("state") {
                None | Some("visible") => SheetState::Visible,
                Some("hidden") => SheetState::Hidden,
                Some("veryHidden") => SheetState::VeryHidden,
                Some(other) => {
                    return Err(Error::unreadable(format!(
                        "sheet \'{name}\' has state \'{other}\'; the states are visible, \
                         hidden and veryHidden"
                    )));
                }
            };
            Ok(Sheet {
                name: name.to_owned(),
                state,
                rel_id: node
                    .attributes()
                    .find(|attribute| attribute.name() == "id")
                    .map(|attribute| attribute.value().to_owned()),
                sheet_id: node.attribute("sheetId").and_then(|id| id.parse().ok()),
            })
        })
        .collect()
}

fn read_defined_names(root: Node, sheets: &[Sheet]) -> Result<Vec<DefinedName>> {
    let Some(names) = children(root, "definedNames").next() else {
        return Ok(Vec::new());
    };
    children(names, "definedName")
        .map(|node| {
            let name = node.attribute("name").ok_or_else(|| {
                Error::unreadable("a <definedName> in the workbook part has no name attribute")
            })?;
            let scope = scope_of(node, name, sheets)?;
            let refers_to = text_of(node).trim().to_owned();
            let resolved = resolve(&refers_to, &scope, sheets);
            Ok(DefinedName {
                name: name.to_owned(),
                scope,
                refers_to,
                resolved,
            })
        })
        .collect()
}

/// A name's scope. `localSheetId` is a position in the sheet list, not a
/// sheet id, and a position the list does not have means the part disagrees
/// with itself.
fn scope_of(node: Node, name: &str, sheets: &[Sheet]) -> Result<Scope> {
    let Some(local) = node.attribute("localSheetId") else {
        return Ok(Scope::Workbook);
    };
    let position: usize = local.parse().map_err(|_| {
        Error::unreadable(format!(
            "defined name \'{name}\' has localSheetId \'{local}\', which is not a sheet position"
        ))
    })?;
    sheets
        .get(position)
        .map(|sheet| Scope::Sheet(sheet.name.clone()))
        .ok_or_else(|| {
            Error::unreadable(format!(
                "defined name \'{name}\' is scoped to sheet {position}, but the package has {} sheets",
                sheets.len()
            ))
        })
}

/// The anchor a reference resolves to, or the reason it resolves to none.
fn resolve(refers_to: &str, scope: &Scope, sheets: &[Sheet]) -> Resolved {
    let area = match parse_refers_to(refers_to) {
        RefersTo::RefError => return Resolved::Unresolvable(NoAnchor::RefError),
        RefersTo::Constant => return Resolved::Unresolvable(NoAnchor::Constant),
        RefersTo::Formula => return Resolved::Unresolvable(NoAnchor::Formula),
        RefersTo::Area { sheet, top_left } => (sheet, top_left),
    };
    let (written, cell) = area;
    let sheet = match written {
        Some(written) => sheet_named_in(sheets, &written).map(|sheet| sheet.name.clone()),
        // A reference carrying no sheet of its own means the sheet the name
        // is scoped to. A workbook-scoped name has no such sheet.
        None => match scope {
            Scope::Sheet(name) => Some(name.clone()),
            Scope::Workbook => None,
        },
    };
    match sheet {
        Some(sheet) => Resolved::Anchor(Address { sheet, cell }),
        None => Resolved::Unresolvable(NoAnchor::UnknownSheet),
    }
}

/// Match a sheet name the way Excel matches it: without regard to case.
fn sheet_named_in<'a>(sheets: &'a [Sheet], name: &str) -> Option<&'a Sheet> {
    let wanted = name.to_lowercase();
    sheets
        .iter()
        .find(|sheet| sheet.name.to_lowercase() == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::reference::Cell;

    /// A workbook part around `body`, with the namespace Excel declares.
    fn workbook_xml(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{body}</workbook>"#
        )
    }

    const THREE_SHEETS: &str = r#"<sheets>
        <sheet name="Data" sheetId="1" r:id="rId1"/>
        <sheet name="Notes" sheetId="2" state="hidden" r:id="rId2"/>
        <sheet name="Parameters" sheetId="3" state="veryHidden" r:id="rId3"/>
      </sheets>"#;

    fn parse(body: &str) -> Workbook {
        Workbook::parse(&workbook_xml(body)).expect("the test workbook must parse")
    }

    fn error(body: &str) -> Error {
        Workbook::parse(&workbook_xml(body)).expect_err("the test workbook must be rejected")
    }

    fn anchor(sheet: &str, a1: &str) -> Resolved {
        Resolved::Anchor(Address {
            sheet: sheet.to_owned(),
            cell: Cell::parse(a1).expect("the test asks for a cell on the grid"),
        })
    }

    #[test]
    fn sheets_come_back_in_workbook_order_with_their_state() {
        let workbook = parse(THREE_SHEETS);

        assert_eq!(
            workbook.sheets(),
            [
                Sheet {
                    name: "Data".to_owned(),
                    state: SheetState::Visible,
                    rel_id: Some("rId1".to_owned()),
                    sheet_id: Some(1),
                },
                Sheet {
                    name: "Notes".to_owned(),
                    state: SheetState::Hidden,
                    rel_id: Some("rId2".to_owned()),
                    sheet_id: Some(2),
                },
                Sheet {
                    name: "Parameters".to_owned(),
                    state: SheetState::VeryHidden,
                    rel_id: Some("rId3".to_owned()),
                    sheet_id: Some(3),
                },
            ]
        );
    }

    #[test]
    fn the_three_states_keep_the_spelling_the_package_uses() {
        assert_eq!(SheetState::Visible.as_str(), "visible");
        assert_eq!(SheetState::Hidden.as_str(), "hidden");
        assert_eq!(SheetState::VeryHidden.as_str(), "veryHidden");
    }

    #[test]
    fn a_prefixed_workbook_part_reads_the_same_as_an_unprefixed_one() {
        let prefixed = r#"<?xml version="1.0"?>
<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <x:sheets><x:sheet name="Data" sheetId="1"/></x:sheets>
  <x:definedNames><x:definedName name="Rate">Data!$B$2</x:definedName></x:definedNames>
</x:workbook>"#;
        let workbook = Workbook::parse(prefixed).expect("a prefixed part is still a workbook");

        assert_eq!(workbook.sheets().len(), 1);
        assert_eq!(workbook.defined_names()[0].resolved, anchor("Data", "B2"));
    }

    #[test]
    fn a_sheet_is_found_without_regard_to_case_and_answers_in_its_own_spelling() {
        let workbook = parse(THREE_SHEETS);

        for spelling in ["parameters", "PARAMETERS", "Parameters", "pArAmEtErS"] {
            assert_eq!(
                workbook
                    .sheet_named(spelling)
                    .map(|sheet| sheet.name.as_str()),
                Some("Parameters"),
                "{spelling}"
            );
        }
        assert_eq!(workbook.sheet_named("Absent"), None);
    }

    #[test]
    fn a_name_is_found_in_its_own_scope_without_regard_to_case() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Rate">Data!$A$1</definedName>
                <definedName name="Rate" localSheetId="1">Notes!$A$1</definedName>
              </definedNames>"#
        ));

        for spelling in ["Rate", "rate", "RATE"] {
            assert_eq!(
                workbook
                    .name_in_scope(spelling, &Scope::Workbook)
                    .map(|name| name.refers_to.as_str()),
                Some("Data!$A$1"),
                "{spelling}"
            );
        }
        assert_eq!(
            workbook
                .name_in_scope("rate", &Scope::Sheet("Notes".to_owned()))
                .map(|name| name.refers_to.as_str()),
            Some("Notes!$A$1"),
            "a scope is exact, so the sheet-scoped name is a different name"
        );
        assert_eq!(
            workbook.name_in_scope("Rate", &Scope::Sheet("Data".to_owned())),
            None
        );
    }

    #[test]
    fn the_names_of_a_scope_come_back_in_declaration_order() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Second" localSheetId="1">Notes!$A$2</definedName>
                <definedName name="First">Data!$A$1</definedName>
                <definedName name="Third">Data!$A$3</definedName>
              </definedNames>"#
        ));

        assert_eq!(
            workbook.names_in_scope(&Scope::Workbook),
            ["First", "Third"]
        );
        assert_eq!(
            workbook.names_in_scope(&Scope::Sheet("Notes".to_owned())),
            ["Second"]
        );
    }

    #[test]
    fn a_workbook_without_sheets_or_names_has_neither() {
        let workbook = parse("");

        assert!(workbook.sheets().is_empty());
        assert!(workbook.defined_names().is_empty());
    }

    #[test]
    fn a_workbook_scoped_name_resolves_to_the_top_left_of_its_first_area() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Merged">Data!$B$2:$C$3</definedName>
              </definedNames>"#
        ));
        let name = &workbook.defined_names()[0];

        assert_eq!(name.name, "Merged");
        assert_eq!(name.scope, Scope::Workbook);
        assert_eq!(name.refers_to, "Data!$B$2:$C$3");
        assert_eq!(name.resolved, anchor("Data", "B2"));
    }

    #[test]
    fn a_local_sheet_id_scopes_a_name_to_that_sheet_by_position() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Local" localSheetId="2">Parameters!$A$1</definedName>
              </definedNames>"#
        ));

        assert_eq!(
            workbook.defined_names()[0].scope,
            Scope::Sheet("Parameters".to_owned())
        );
    }

    #[test]
    fn a_names_sheet_is_matched_without_regard_to_case() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Loud">DATA!$A$1</definedName>
              </definedNames>"#
        ));

        assert_eq!(
            workbook.defined_names()[0].resolved,
            anchor("Data", "A1"),
            "the package's spelling is what is reported"
        );
    }

    #[test]
    fn a_sheet_scoped_name_without_a_sheet_in_its_reference_resolves_to_its_scope() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Bare" localSheetId="1">$D$4</definedName>
              </definedNames>"#
        ));

        assert_eq!(workbook.defined_names()[0].resolved, anchor("Notes", "D4"));
    }

    #[test]
    fn a_name_that_points_at_no_cell_says_why() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Gone">#REF!</definedName>
                <definedName name="Rate">0.175</definedName>
                <definedName name="Sum">SUM(Data!$A$1:$A$9)</definedName>
                <definedName name="Elsewhere">Missing!$A$1</definedName>
                <definedName name="Floating">$A$1</definedName>
              </definedNames>"#
        ));
        let reasons: Vec<_> = workbook
            .defined_names()
            .iter()
            .map(|name| match &name.resolved {
                Resolved::Unresolvable(reason) => reason.as_str(),
                Resolved::Anchor(address) => panic!("{} resolved to {address}", name.name),
            })
            .collect();

        assert_eq!(
            reasons,
            [
                "ref_error",
                "constant",
                "formula",
                "unknown_sheet",
                "unknown_sheet"
            ]
        );
    }

    #[test]
    fn the_raw_refers_to_is_reported_as_the_package_holds_it() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="Quoted">'Data'!$A$1</definedName>
              </definedNames>"#
        ));

        assert_eq!(workbook.defined_names()[0].refers_to, "'Data'!$A$1");
    }

    #[test]
    fn a_built_in_hidden_name_is_listed_like_any_other() {
        let workbook = parse(&format!(
            r#"{THREE_SHEETS}<definedNames>
                <definedName name="_xlnm.Print_Area" localSheetId="0" hidden="1">Data!$A$1:$D$9</definedName>
              </definedNames>"#
        ));

        assert_eq!(workbook.defined_names().len(), 1);
        assert_eq!(workbook.defined_names()[0].name, "_xlnm.Print_Area");
    }

    #[test]
    fn a_part_that_is_not_a_workbook_is_unreadable() {
        for xml in ["<worksheet/>", "not xml at all", "<workbook><sheets>"] {
            let err = Workbook::parse(xml).expect_err(xml);
            assert_eq!(err.code(), ErrorCode::Unreadable, "{xml}");
        }
    }

    #[test]
    fn a_malformed_sheet_or_name_is_unreadable_rather_than_guessed_at() {
        for body in [
            // No name to report.
            r#"<sheets><sheet sheetId="1"/></sheets>"#,
            // A state outside the three the schema allows.
            r#"<sheets><sheet name="Data" state="sortOfHidden"/></sheets>"#,
            // A name with nothing to call it.
            r#"<sheets><sheet name="Data"/></sheets><definedNames><definedName>Data!$A$1</definedName></definedNames>"#,
            // A scope pointing past the end of the sheet list.
            r#"<sheets><sheet name="Data"/></sheets><definedNames><definedName name="X" localSheetId="7">Data!$A$1</definedName></definedNames>"#,
            // A scope that is not a number at all.
            r#"<sheets><sheet name="Data"/></sheets><definedNames><definedName name="X" localSheetId="first">Data!$A$1</definedName></definedNames>"#,
        ] {
            assert_eq!(error(body).code(), ErrorCode::Unreadable, "{body}");
        }
    }
}
