//! Contract tests for `props`: reading, writing and taking out the custom
//! document properties a package carries about itself.
//!
//! The feature fixture carries one property of each of the four types Excel
//! offers, which is what makes it the fixture these run over; the plain
//! fixture carries no custom properties part at all, which is what makes it
//! the one the creating tests run over.
//!
//! This is the only verb that puts a part into a package, so what the report
//! says about that, and what the package says about the part afterwards, is
//! asserted here.

mod support;

use serde_json::json;
use support::{
    Workspace, assert_only_these_differ, assert_same_bytes, assert_spliced, copy_of, envelope,
    exit_code, fixture, in_text, json, op, part, part_text, run, stderr, stdout, timestamp,
    under_json,
};
use xlsplice::batch::Batch;
use xlsplice::batch::Destination;
use xlsplice::batch::WriteType;
use xlsplice::verb::{self, Trace};

const CUSTOM: &str = "docProps/custom.xml";
const CONTENT_TYPES: &str = "[Content_Types].xml";
const ROOT_RELS: &str = "_rels/.rels";

/// What `props get FILE --json` reports.
fn reported(package: &std::path::Path) -> serde_json::Value {
    let out = run(&["props", "get", &package.display().to_string(), "--json"]);
    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    json(&out)["properties"].clone()
}

#[test]
fn reading_reports_every_type_with_a_value_of_that_type() {
    let package = copy_of("read", "feature.xlsx");

    assert_eq!(
        reported(&package),
        json!([
            {"name": "Stamp.Text", "type": "lpwstr", "value": "xlsplice"},
            {"name": "Stamp.Number", "type": "i4", "value": 42},
            {"name": "Stamp.Flag", "type": "bool", "value": true},
            {"name": "Stamp.Date", "type": "filetime", "value": "2026-09-11T10:00:00Z"},
        ])
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn reading_is_one_tab_separated_row_per_property_down_a_pipe() {
    let package = copy_of("rows", "feature.xlsx");

    let out = run(&["props", "get", &package.display().to_string()]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "Stamp.Text\tlpwstr\txlsplice\n\
         Stamp.Number\ti4\t42\n\
         Stamp.Flag\tbool\ttrue\n\
         Stamp.Date\tfiletime\t2026-09-11T10:00:00Z\n"
    );
}

/// A package with no custom properties has none, which is a thing a package
/// is rather than something wrong with it.
#[test]
fn a_package_carrying_no_properties_reports_none() {
    let package = copy_of("none", "plain.xlsx");

    assert_eq!(reported(&package), json!([]));
    assert_eq!(
        stdout(&run(&["props", "get", &package.display().to_string()])),
        ""
    );
}

/// Each of the four write types, written over the property of that type the
/// fixture already carries: only what the property holds moves, so its
/// identifier and every other byte of the part stay put.
#[test]
fn writing_each_type_over_one_already_there_keeps_its_identifier() {
    for (name, write_type, value, was, becomes) in [
        (
            "Stamp.Text",
            WriteType::Text,
            "rewritten",
            "<vt:lpwstr>xlsplice</vt:lpwstr>",
            "<vt:lpwstr>rewritten</vt:lpwstr>",
        ),
        (
            "Stamp.Number",
            WriteType::Number,
            "99",
            "<vt:i4>42</vt:i4>",
            "<vt:i4>99</vt:i4>",
        ),
        (
            "Stamp.Flag",
            WriteType::Bool,
            "false",
            "<vt:bool>true</vt:bool>",
            "<vt:bool>false</vt:bool>",
        ),
        (
            "Stamp.Date",
            WriteType::Date,
            "2030-01-02T03:04:05",
            "<vt:filetime>2026-09-11T10:00:00Z</vt:filetime>",
            "<vt:filetime>2030-01-02T03:04:05Z</vt:filetime>",
        ),
    ] {
        let package = copy_of(name, "feature.xlsx");

        let out = under_json(verb::run(
            &package,
            &Batch {
                operations: vec![op::stamping(name, write_type, value)],
            },
            &Destination::InPlace,
            false,
            &Trace::Off,
        ));

        assert_eq!(out.exit, 0, "{name}: {}", out.stdout);
        assert_eq!(
            envelope(&out)["parts"]["changed"],
            json!([CUSTOM]),
            "{name}"
        );
        assert_only_these_differ(&fixture("feature.xlsx"), &package, &[CUSTOM]);
        assert_spliced(
            &part_text(&fixture("feature.xlsx"), CUSTOM),
            &part_text(&package, CUSTOM),
            was,
            becomes,
        );
    }
}

/// A number that is whole and in range is a 32-bit integer, as Excel writes
/// one; anything else is a real.
#[test]
fn a_number_is_the_integer_variant_where_it_fits_and_the_real_one_otherwise() {
    for (value, expected) in [
        ("7", json!({"type": "i4", "value": 7})),
        ("-2.5", json!({"type": "r8", "value": -2.5})),
        ("3000000000", json!({"type": "r8", "value": 3000000000i64})),
    ] {
        let package = copy_of("numbers", "feature.xlsx");

        let out = under_json(verb::run(
            &package,
            &Batch {
                operations: vec![op::stamping("Stamp.Number", WriteType::Number, value)],
            },
            &Destination::InPlace,
            false,
            &Trace::Off,
        ));

        assert_eq!(out.exit, 0, "{value}: {}", out.stdout);
        let found = &reported(&package)[1];
        assert_eq!(found["type"], expected["type"], "{value}");
        assert_eq!(found["value"], expected["value"], "{value}");
    }
}

#[test]
fn a_property_that_is_not_there_is_added_after_the_last_with_the_next_identifier() {
    let package = copy_of("add", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Stamp.New", WriteType::Text, "added")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[CUSTOM]);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), CUSTOM),
        &part_text(&package, CUSTOM),
        "</Properties>",
        concat!(
            r#"<property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="6" "#,
            r#"name="Stamp.New"><vt:lpwstr>added</vt:lpwstr></property></Properties>"#
        ),
    );
}

/// Several added at once run their identifiers on from each other, which is
/// what the batch settles rather than any one operation.
#[test]
fn several_properties_added_in_one_batch_take_consecutive_identifiers() {
    let package = copy_of("add-several", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::stamping("One", WriteType::Text, "a"),
                op::stamping("Two", WriteType::Bool, "true"),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let written = part_text(&package, CUSTOM);
    assert!(written.contains(r#"pid="6" name="One"#), "{written}");
    assert!(written.contains(r#"pid="7" name="Two"#), "{written}");
    assert_eq!(
        envelope(&out)["operations"]
            .as_array()
            .expect("one entry per operation")
            .len(),
        2
    );
}

/// The one place xlsplice puts a part into a package: the part, its content
/// type and the relationship reaching it go in together, and the report says
/// so.
#[test]
fn setting_on_a_package_with_no_part_adds_the_part_and_both_declarations() {
    let package = copy_of("create", "plain.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Reference", WriteType::Text, "R-1")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        envelope(&out)["parts"],
        json!({
            "changed": [CONTENT_TYPES, ROOT_RELS],
            "added": [CUSTOM],
            "removed": [],
        })
    );
    let comparison = support::compare(&fixture("plain.xlsx"), &package);
    assert_eq!(comparison.differs, [CONTENT_TYPES, ROOT_RELS]);
    assert_eq!(comparison.added, [CUSTOM]);
    assert_eq!(comparison.removed, Vec::<String>::new());
    assert!(
        comparison.order_kept,
        "the parts already there did not move"
    );
    assert_eq!(
        part_text(&package, CUSTOM),
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n",
            "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/",
            "custom-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/",
            "2006/docPropsVTypes\"><property fmtid=\"{D5CDD505-2E9C-101B-9397-08002B2CF9AE}\" ",
            "pid=\"2\" name=\"Reference\"><vt:lpwstr>R-1</vt:lpwstr></property></Properties>"
        )
    );
    assert_spliced(
        &part_text(&fixture("plain.xlsx"), CONTENT_TYPES),
        &part_text(&package, CONTENT_TYPES),
        "</Types>",
        concat!(
            r#"<Override PartName="/docProps/custom.xml" ContentType="application/vnd."#,
            r#"openxmlformats-officedocument.custom-properties+xml"/></Types>"#
        ),
    );
    assert_spliced(
        &part_text(&fixture("plain.xlsx"), ROOT_RELS),
        &part_text(&package, ROOT_RELS),
        "</Relationships>",
        concat!(
            r#"<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/"#,
            r#"officeDocument/2006/relationships/custom-properties" "#,
            r#"Target="docProps/custom.xml"/></Relationships>"#
        ),
    );
}

/// A created part has no entry of its own to take a moment from, so it takes
/// the zip epoch, and it goes in last because the parts already there keep
/// the order they had.
#[test]
fn a_created_part_is_the_last_entry_and_carries_the_zip_epoch() {
    let package = copy_of("created-entry", "plain.xlsx");

    under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Reference", WriteType::Text, "R-1")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(
        support::parts(&package)
            .last()
            .expect("the package holds parts")
            .path,
        CUSTOM
    );
    assert_eq!(timestamp(&package, CUSTOM), "1980-01-01 00:00:00");
    assert_eq!(
        timestamp(&package, "xl/workbook.xml"),
        timestamp(&fixture("plain.xlsx"), "xl/workbook.xml"),
        "a part copied across keeps the moment it had"
    );
}

/// Several properties on a package with none go into the one part written for
/// them, and it is written once.
#[test]
fn several_properties_on_a_package_with_no_part_go_into_the_one_part() {
    let package = copy_of("create-several", "plain.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::stamping("One", WriteType::Text, "a"),
                op::stamping("Two", WriteType::Number, "2"),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(envelope(&out)["parts"]["added"], json!([CUSTOM]));
    assert_eq!(
        reported(&package),
        json!([
            {"name": "One", "type": "lpwstr", "value": "a"},
            {"name": "Two", "type": "i4", "value": 2},
        ])
    );
}

#[test]
fn writing_a_property_the_value_it_already_holds_changes_nothing() {
    let package = copy_of("unchanged", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Stamp.Text", WriteType::Text, "xlsplice")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(envelope(&out)["operations"][0]["changed"], json!(false));
    assert_eq!(envelope(&out)["parts"]["changed"], json!([]));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn unsetting_takes_out_only_the_property_named() {
    let package = copy_of("unset", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::unstamping("Stamp.Flag")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_only_these_differ(&fixture("feature.xlsx"), &package, &[CUSTOM]);
    assert_spliced(
        &part_text(&fixture("feature.xlsx"), CUSTOM),
        &part_text(&package, CUSTOM),
        concat!(
            r#"<property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="4" "#,
            r#"name="Stamp.Flag"><vt:bool>true</vt:bool></property>"#
        ),
        "",
    );
    assert_eq!(
        reported(&package)
            .as_array()
            .expect("a list of properties")
            .len(),
        3
    );
}

#[test]
fn unsetting_a_property_that_is_not_there_is_not_found() {
    for (label, fixture_name) in [("missing", "feature.xlsx"), ("no-part", "plain.xlsx")] {
        let package = copy_of(label, fixture_name);

        let out = under_json(verb::run(
            &package,
            &Batch {
                operations: vec![op::unstamping("Nope")],
            },
            &Destination::InPlace,
            false,
            &Trace::Off,
        ));

        assert_eq!(out.exit, 3, "{label}: {}", out.stdout);
        assert_eq!(
            envelope(&out)["error"]["code"],
            json!("not_found"),
            "{label}"
        );
        assert_same_bytes(&fixture(fixture_name), &package);
    }
}

/// A name is matched exactly, so one differing in case is another property
/// and is added rather than written over.
#[test]
fn a_name_differing_in_case_is_another_property() {
    let package = copy_of("case", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("stamp.text", WriteType::Text, "other")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let found = reported(&package);
    assert_eq!(found[0]["name"], json!("Stamp.Text"));
    assert_eq!(found[0]["value"], json!("xlsplice"));
    assert_eq!(found[4]["name"], json!("stamp.text"));
}

/// The cell rule, applied to properties: one property cannot be asked two
/// things by one batch.
#[test]
fn two_operations_naming_one_property_are_refused_before_anything_is_read() {
    let package = copy_of("repeated", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::stamping("Stamp.Text", WriteType::Text, "one"),
                op::unstamping("Stamp.Text"),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 2);
    let body = envelope(&out);
    assert_eq!(body["error"]["code"], json!("usage"));
    let message = body["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message");
    assert!(message.contains("index 0 and 1"), "{message}");
    assert!(message.contains("Stamp.Text"), "{message}");
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// A value that is not one of what it says it is fails against the operation
/// that gave it, whatever else the batch holds.
#[test]
fn a_value_that_is_not_what_its_type_says_names_the_operation_it_came_from() {
    let package = copy_of("bad-value", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![
                op::stamping("Stamp.Text", WriteType::Text, "fine"),
                op::stamping("Stamp.Number", WriteType::Number, "twelve"),
            ],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 2);
    let message = envelope(&out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned();
    assert!(
        message.contains("index 1 (props.set Stamp.Number)"),
        "{message}"
    );
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

/// A property's date is a moment rather than the serial a cell would hold, so
/// the workbook's date system has no say in it and the phantom leap day is
/// nothing to it.
#[test]
fn a_date_on_a_property_is_a_moment_and_not_a_serial() {
    let package = copy_of("moment", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Stamp.Date", WriteType::Date, "1900-01-05")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        reported(&package)[3]["value"],
        json!("1900-01-05T00:00:00Z")
    );
}

/// Both operations reach a package through `apply`, which is the shape a
/// caller writes a batch in.
#[test]
fn both_operations_work_through_apply() {
    let workspace = Workspace::new("through-apply");
    let package = workspace.copy_of("feature.xlsx");
    let batch = workspace.file(
        "batch.json",
        br#"[
            {"op": "props.set", "name": "Stamp.Text", "type": "text", "value": "through"},
            {"op": "props.set", "name": "Stamp.When", "type": "date", "value": "2026-09-13"},
            {"op": "props.unset", "name": "Stamp.Flag"}
        ]"#,
    );

    let out = run(&[
        "apply",
        &package.display().to_string(),
        &batch.display().to_string(),
        "--json",
    ]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["parts"]["changed"], json!([CUSTOM]));
    assert_eq!(
        reported(&package),
        json!([
            {"name": "Stamp.Text", "type": "lpwstr", "value": "through"},
            {"name": "Stamp.Number", "type": "i4", "value": 42},
            {"name": "Stamp.Date", "type": "filetime", "value": "2026-09-11T10:00:00Z"},
            {"name": "Stamp.When", "type": "filetime", "value": "2026-09-13T00:00:00Z"},
        ])
    );
}

#[test]
fn a_write_reaches_the_package_from_the_command_line() {
    let package = copy_of("argv", "feature.xlsx");
    let file = package.display().to_string();

    let set = run(&[
        "props",
        "set",
        &file,
        "Stamp.Text",
        "argv",
        "--type",
        "text",
    ]);
    assert_eq!(exit_code(&set), 0, "{}", stderr(&set));
    let unset = run(&["props", "unset", &file, "Stamp.Flag"]);
    assert_eq!(exit_code(&unset), 0, "{}", stderr(&unset));

    assert_eq!(
        reported(&package),
        json!([
            {"name": "Stamp.Text", "type": "lpwstr", "value": "argv"},
            {"name": "Stamp.Number", "type": "i4", "value": 42},
            {"name": "Stamp.Date", "type": "filetime", "value": "2026-09-11T10:00:00Z"},
        ])
    );
}

/// A writing verb's flags mean the same thing here as everywhere.
#[test]
fn a_dry_run_reports_what_would_change_and_writes_nothing() {
    let package = copy_of("dry", "feature.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Stamp.Text", WriteType::Text, "not written")],
        },
        &Destination::InPlace,
        true,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    let body = envelope(&out);
    assert_eq!(body["dry_run"], json!(true));
    assert_eq!(body["parts"]["changed"], json!([CUSTOM]));
    assert_same_bytes(&fixture("feature.xlsx"), &package);
}

#[test]
fn out_leaves_the_package_alone_and_writes_the_result_elsewhere() {
    let package = copy_of("out", "plain.xlsx");
    let workspace = package.workspace();
    let elsewhere = workspace.dir().join("stamped.xlsx");

    let out = under_json(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Reference", WriteType::Text, "R-1")],
        },
        &Destination::Out(elsewhere.clone()),
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_same_bytes(&fixture("plain.xlsx"), &package);
    assert_eq!(reported(&elsewhere)[0]["name"], json!("Reference"));
}

#[test]
fn the_report_is_one_row_per_operation_down_a_pipe() {
    let package = copy_of("write-rows", "feature.xlsx");

    let out = in_text(verb::run(
        &package,
        &Batch {
            operations: vec![op::stamping("Stamp.Text", WriteType::Text, "row")],
        },
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout, "Stamp.Text\t\t\ttrue\n",
        "the property stands in the target column, and it resolves to no cell"
    );
}

#[test]
fn the_help_lists_the_three_actions_and_what_each_takes() {
    let out = run(&["props", "--help"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    for wanted in ["get", "set", "unset"] {
        assert!(
            stdout(&out).contains(wanted),
            "props --help must list {wanted}"
        );
    }
    for (action, wanted) in [
        ("get", vec!["FILE"]),
        (
            "set",
            vec!["FILE", "NAME", "VALUE", "--type", "--out", "--dry-run"],
        ),
        ("unset", vec!["FILE", "NAME", "--out", "--dry-run"]),
    ] {
        let help = run(&["props", action, "--help"]);
        assert_eq!(exit_code(&help), 0, "{action}: {}", stderr(&help));
        for one in wanted {
            assert!(
                stdout(&help).contains(one),
                "props {action} --help must list {one}: {}",
                stdout(&help)
            );
        }
    }
}

/// The part a package holds is not moved by a verb that has nothing to do
/// with it.
#[test]
fn a_cell_write_does_not_touch_the_properties() {
    let package = copy_of("cells", "feature.xlsx");

    let out = under_json(verb::set(
        &package,
        "Inputs!A1",
        WriteType::Number,
        "5",
        false,
        &Destination::InPlace,
        false,
        &Trace::Off,
    ));

    assert_eq!(out.exit, 0, "{}", out.stdout);
    assert_eq!(
        part(&package, CUSTOM),
        part(&fixture("feature.xlsx"), CUSTOM)
    );
}
