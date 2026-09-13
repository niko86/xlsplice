//! Part edits: what the package does with a part created, one removed, and
//! one read twice.
//!
//! A splice is one species of part edit and the whole of what `set` asks for,
//! so it is asserted beside `set` in `set.rs`. Here are the other two, which
//! no verb reaches yet: they are asked of `Package::rebuild` directly, so
//! that the vocabulary #10 and #12 will need is held to the same byte-level
//! guarantee as the one that is in use. What the container came out holding
//! is read by the comparator in `support`, never by the tool itself.

mod support;

use std::collections::BTreeMap;

use support::{Workspace, compare, part, part_text, timestamp};
use xlsplice::batch::{Opened, Operation, WriteType};
use xlsplice::package::{Content, Package};

const SHEET1: &str = "xl/worksheets/sheet1.xml";
const SHEET3: &str = "xl/worksheets/sheet3.xml";
const CREATED: &str = "docProps/custom.xml";

/// The zip epoch, which is the earliest moment a container can spell and the
/// stamp a created part takes, so that two runs of one batch produce the same
/// bytes.
const ZIP_EPOCH: &str = "1980-01-01 00:00:00";

/// Rebuild the feature package with `edits`, and give back both paths: the
/// package as it was, and the one the rebuild produced.
fn rebuilt(
    label: &str,
    edits: &BTreeMap<String, Content>,
) -> (Workspace, std::path::PathBuf, std::path::PathBuf) {
    let workspace = Workspace::new(label);
    let before = workspace.feature_package("feature.xlsx");
    let mut package = Package::open(&before).expect("the package must open");
    let bytes = package.rebuild(edits).expect("the container must rebuild");
    let after = workspace.file("after.xlsx", &bytes);
    (workspace, before, after)
}

#[test]
fn a_created_part_is_added_at_the_zip_epoch_and_every_other_part_is_copied_raw() {
    let edits = BTreeMap::from([(
        CREATED.to_owned(),
        Content::Bytes(b"<properties/>".to_vec()),
    )]);

    let (_workspace, before, after) = rebuilt("create", &edits);

    let comparison = compare(&before, &after);
    assert_eq!(comparison.added, [CREATED], "the part was created");
    assert_eq!(
        comparison.differs,
        Vec::<String>::new(),
        "creating a part moved nothing else"
    );
    assert_eq!(
        comparison.identical.len(),
        support::parts(&before).len(),
        "every part that was there was copied raw"
    );
    assert!(comparison.order_kept, "the parts were reordered");
    assert_eq!(part_text(&after, CREATED), "<properties/>");
    assert_eq!(
        timestamp(&after, CREATED),
        ZIP_EPOCH,
        "a created part carries the one stamp a rebuild can give it"
    );
    assert_eq!(
        part(&after, CREATED).method,
        "Deflated",
        "a created part is stored the way Excel stores one"
    );
}

#[test]
fn a_removed_part_is_left_out_and_every_other_part_is_copied_raw() {
    let edits = BTreeMap::from([(SHEET3.to_owned(), Content::Gone)]);

    let (_workspace, before, after) = rebuilt("remove", &edits);

    let comparison = compare(&before, &after);
    assert_eq!(comparison.removed, [SHEET3], "the part was removed");
    assert_eq!(
        comparison.differs,
        Vec::<String>::new(),
        "removing a part moved nothing else"
    );
    assert_eq!(
        comparison.added,
        Vec::<String>::new(),
        "removing a part added none"
    );
    assert_eq!(
        comparison.identical.len(),
        support::parts(&before).len() - 1,
        "every part that stayed was copied raw"
    );
    assert!(comparison.order_kept, "the parts were reordered");
}

/// Two operations landing on one worksheet read it once between them: the
/// package memoises the text of a part it has read, which is what lets an
/// operation own its own reading without every operation paying for it
/// (ADR-0005).
#[test]
fn a_part_two_operations_both_read_is_read_once() {
    let workspace = Workspace::new("memoised");
    let package = workspace.feature_package("feature.xlsx");
    let mut opened = Opened::open(&package).expect("the package must open");
    let write = |target: &str| Operation::Set {
        target: target.to_owned(),
        write_type: WriteType::Number,
        value: "7".to_owned(),
    };

    let opening = opened.reads();
    write("Inputs!A1")
        .edits(0, &mut opened)
        .expect("A1 must be writable");
    let after_one = opened.reads();
    write("Inputs!A2")
        .edits(1, &mut opened)
        .expect("A2 must be writable");

    assert_eq!(
        opening, 3,
        "opening a package reads the root relationships, the workbook part \
         they name, and that part's own relationships"
    );
    assert_eq!(
        after_one,
        opening + 1,
        "the first operation read the worksheet its cell sits in"
    );
    assert_eq!(
        opened.reads(),
        after_one,
        "the second operation read no part the first had not"
    );
}

/// The same part named by two operations is spliced once, with both edits in
/// it, and the report still says what each operation did.
#[test]
fn two_operations_on_one_part_are_spliced_into_it_together() {
    let workspace = Workspace::new("together");
    let package = workspace.feature_package("feature.xlsx");
    let before = part_text(&package, SHEET1);

    let report = xlsplice::batch::run(
        &package,
        &xlsplice::batch::Batch {
            operations: vec![
                Operation::Set {
                    target: "Inputs!A1".to_owned(),
                    write_type: WriteType::Number,
                    value: "11".to_owned(),
                },
                Operation::Set {
                    target: "Inputs!A3".to_owned(),
                    write_type: WriteType::Number,
                    value: "13".to_owned(),
                },
            ],
        },
        &xlsplice::batch::Destination::InPlace,
        false,
    )
    .expect("both cells hold plain numbers");

    assert_eq!(report.parts.changed, [SHEET1], "one part, spliced once");
    assert!(report.operations.iter().all(|operation| operation.changed));
    let after = part_text(&package, SHEET1);
    assert_eq!(
        after,
        before
            .replacen(r#"<c r="A1"><v>1</v></c>"#, r#"<c r="A1"><v>11</v></c>"#, 1)
            .replacen(
                r#"<c r="A3"><v>-3</v></c>"#,
                r#"<c r="A3"><v>13</v></c>"#,
                1
            ),
        "both edits landed and every other byte stayed"
    );
}
