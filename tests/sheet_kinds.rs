//! Contract tests for sheets whose part is not a worksheet (#51).
//!
//! A chartsheet or a dialog sheet is a sheet the package has, and a valid
//! one, but it holds no cells. Naming one is the caller's mistake, not the
//! package's, so every verb that resolves a sheet to its cells refuses it,
//! the same way, and says what the sheet is.
//!
//! The packages are built here rather than saved from Excel: the feature
//! package with one more sheet, reached by a relationship of another type.

mod support;

use std::path::PathBuf;

use serde_json::json;
use support::container::{WORKBOOK, WORKBOOK_RELS, assert_same_bytes};
use support::library::{envelope, targets, under_json};
use support::workspace::{
    Copied, FEATURE_CONTENT_TYPES_XML, WORKBOOK_RELS_XML, Workspace, built, feature_workbook,
};
use xlsplice::batch::{Destination, WriteType};
use xlsplice::render::Rendered;
use xlsplice::verb::{self, Trace};

const CHARTSHEET: &str = "xl/chartsheets/sheet1.xml";
const ODD_SHEET: &str = "xl/worksheets/sheet4.xml";

/// The feature package with a fourth sheet, `Extra`, whose relationship has
/// the type `kind` and whose part at `part` holds `xml`.
fn with_extra_sheet(label: &str, kind: &str, part: &str, xml: &str) -> Copied {
    let workbook = feature_workbook().replace(
        "</sheets>",
        r#"<sheet name="Extra" sheetId="4" r:id="rId9"/></sheets>"#,
    );
    let rels = WORKBOOK_RELS_XML.replace(
        "</Relationships>",
        &format!(
            r#"<Relationship Id="rId9" Type="{kind}" Target="{}"/></Relationships>"#,
            part.trim_start_matches("xl/")
        ),
    );
    let content_types = FEATURE_CONTENT_TYPES_XML.replace(
        "</Types>",
        &format!(r#"<Override PartName="/{part}" ContentType="application/xml"/></Types>"#),
    );
    built(label, |w: &Workspace| {
        let base = w.feature_package("base.xlsx");
        w.like("extra.xlsx", &base)
            .with_part(support::container::CONTENT_TYPES, &content_types)
            .with_part(WORKBOOK, &workbook)
            .with_part(WORKBOOK_RELS, &rels)
            .with_part(part, xml)
            .written()
    })
}

fn chartsheet(label: &str) -> Copied {
    with_extra_sheet(
        label,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet",
        CHARTSHEET,
        r#"<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"/></sheetViews></chartsheet>"#,
    )
}

/// A copy of `package` beside it, to hold the original bytes against.
fn kept(package: &Copied) -> PathBuf {
    let copy = package.beside("before.xlsx");
    std::fs::copy(&**package, &copy).expect("the test must be able to copy its package");
    copy
}

fn assert_refused_as(out: &Rendered, kind: &str) {
    assert_eq!(
        envelope(out)["error"]["code"],
        json!("refused"),
        "{}",
        out.stdout
    );
    let message = envelope(out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned();
    assert!(message.contains("'Extra'"), "{message}");
    assert!(message.contains(kind), "{message}");
}

#[test]
fn get_on_a_chartsheet_is_refused_and_names_what_the_sheet_is() {
    let package = chartsheet("get-chartsheet");

    let out = under_json(verb::get(&package, &targets(&["Extra!A1"]), &Trace::Off));

    assert_refused_as(&out, "chartsheet");
}

#[test]
fn set_and_clear_on_a_chartsheet_are_refused_and_write_nothing() {
    let package = chartsheet("write-chartsheet");
    let before = kept(&package);

    let set = under_json(verb::set(
        &package,
        "Extra!A1",
        WriteType::Number,
        "1",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));
    let clear = under_json(verb::clear(
        &package,
        "Extra!A1",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_refused_as(&set, "chartsheet");
    assert_refused_as(&clear, "chartsheet");
    assert_same_bytes(&before, &package);
}

#[test]
fn a_batch_touching_a_chartsheet_is_refused_whole() {
    let package = chartsheet("apply-chartsheet");
    let before = kept(&package);
    let batch = package.workspace().file(
        "batch.json",
        br#"[
          {"op": "set", "target": "Inputs!A1", "type": "number", "value": "2"},
          {"op": "set", "target": "Extra!A1", "type": "number", "value": "1"}
        ]"#,
    );

    let out = under_json(verb::apply(
        &package,
        &batch,
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_refused_as(&out, "chartsheet");
    assert_same_bytes(&before, &package);
}

#[test]
fn cells_on_a_chartsheet_is_refused_rather_than_answered_with_no_cells() {
    let package = chartsheet("cells-chartsheet");

    let out = under_json(verb::cells(&package, &targets(&["Extra"]), &Trace::Off));

    assert_refused_as(&out, "chartsheet");
}

#[test]
fn a_dialog_sheet_is_refused_the_same_way() {
    let package = with_extra_sheet(
        "dialogsheet",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/dialogsheet",
        "xl/dialogsheets/sheet1.xml",
        r#"<dialogsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#,
    );

    let out = under_json(verb::get(&package, &targets(&["Extra!A1"]), &Trace::Off));

    assert_refused_as(&out, "dialogsheet");
}

/// A part the relationship calls a worksheet and that is not one is the
/// package disagreeing with itself, which is what `unreadable` is for.
#[test]
fn a_worksheet_part_that_is_not_a_worksheet_is_still_unreadable() {
    let package = with_extra_sheet(
        "odd-worksheet",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet",
        ODD_SHEET,
        r#"<thing xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#,
    );

    let out = under_json(verb::get(&package, &targets(&["Extra!A1"]), &Trace::Off));

    assert_eq!(
        envelope(&out)["error"]["code"],
        json!("unreadable"),
        "{}",
        out.stdout
    );
}

/// The other sheets of a package holding a chartsheet read as they always
/// did: the refusal is about the sheet named, not the package.
#[test]
fn the_worksheets_beside_a_chartsheet_still_read() {
    let package = chartsheet("beside-chartsheet");

    let out = under_json(verb::get(&package, &targets(&["Inputs!A1"]), &Trace::Off));

    assert_eq!(out.exit, 0, "{}", out.stdout);
}
