//! Part edits: what the package does with a part created and one removed.
//!
//! A splice is one species of part edit and the whole of what `set` asks for,
//! so it is asserted beside `set` in `set.rs`, and what a batch of several
//! makes of one part is in `batches.rs`. Here are the other two, which no
//! verb reaches yet: they are asked of `Package::rebuild` directly, so that
//! the vocabulary #10 and #12 will need is held to the same byte-level
//! guarantee as the one that is in use. What the container came out holding
//! is read by the comparator in `support`, never by the tool itself.
//!
//! These are about real containers, so they are about files: what a rebuild
//! keeps of a part it did not touch is a question about compressed bytes,
//! methods and timestamps, which is what a fixture on disk has and a package
//! a test wrote itself does not. The memo these tests used to watch is a
//! promise about one package rather than about a container, and is held to
//! in `batch.rs`, where the package it is asked of is built where it is read.

mod support;

use std::collections::BTreeMap;

use support::container::{CUSTOM_PROPERTIES, SHEET3, compare, part, part_text, timestamp};
use support::workspace::{Copied, built};
use xlsplice::package::{Content, Package};

/// The zip epoch, which is the earliest moment a container can spell and the
/// stamp a created part takes, so that two runs of one batch produce the same
/// bytes.
const ZIP_EPOCH: &str = "1980-01-01 00:00:00";

/// Rebuild the feature package with `edits`, and give back both paths: the
/// package as it was, and the one the rebuild produced.
fn rebuilt(label: &str, edits: &BTreeMap<String, Content>) -> (Copied, std::path::PathBuf) {
    let before = built(label, |w| w.feature_package("feature.xlsx"));
    let mut package = Package::open(&before).expect("the package must open");
    let bytes = package.rebuild(edits).expect("the container must rebuild");
    let after = before.workspace().file("after.xlsx", &bytes);
    (before, after)
}

#[test]
fn a_created_part_is_added_at_the_zip_epoch_and_every_other_part_is_copied_raw() {
    let edits = BTreeMap::from([(
        CUSTOM_PROPERTIES.to_owned(),
        Content::Bytes(b"<properties/>".to_vec()),
    )]);

    let (before, after) = rebuilt("create", &edits);

    let comparison = compare(&before, &after);
    assert_eq!(
        comparison.added,
        [CUSTOM_PROPERTIES],
        "the part was created"
    );
    assert_eq!(
        comparison.differs,
        Vec::<String>::new(),
        "creating a part moved nothing else"
    );
    assert_eq!(
        comparison.identical.len(),
        support::container::parts(&before).len(),
        "every part that was there was copied raw"
    );
    assert!(comparison.order_kept, "the parts were reordered");
    assert_eq!(part_text(&after, CUSTOM_PROPERTIES), "<properties/>");
    assert_eq!(
        timestamp(&after, CUSTOM_PROPERTIES),
        ZIP_EPOCH,
        "a created part carries the one stamp a rebuild can give it"
    );
    assert_eq!(
        part(&after, CUSTOM_PROPERTIES).method,
        "Deflated",
        "a created part is stored the way Excel stores one"
    );
}

#[test]
fn a_removed_part_is_left_out_and_every_other_part_is_copied_raw() {
    let edits = BTreeMap::from([(SHEET3.to_owned(), Content::Gone)]);

    let (before, after) = rebuilt("remove", &edits);

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
        support::container::parts(&before).len() - 1,
        "every part that stayed was copied raw"
    );
    assert!(comparison.order_kept, "the parts were reordered");
}
