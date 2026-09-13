//! Comparing two packages, part by part.
//!
//! What a write did is a question about parts: which of them differ from the
//! ones that were there, which are new, and which have gone. Everything else
//! about a package is downstream of that, and a caller checking a hydration
//! wants to see the short list rather than a byte offset.
//!
//! So this reads both containers itself and compares the bytes of each part.
//! It is deliberately not clever: no per-node detail, no attempt to say what
//! inside a part moved. A part is the same or it is not, which is the level
//! xlsplice's own guarantee is stated at, and it is the level a caller can
//! act on.

use crate::error::Result;
use crate::package::Package;

/// What became of one part between the two packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Both packages hold it, byte for byte the same.
    Identical,
    /// Both packages hold it, and their bytes differ.
    Differs,
    /// Only the second package holds it.
    Added,
    /// Only the first package holds it.
    Removed,
}

impl Status {
    /// The status as it is reported, stable and lower-case.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Identical => "identical",
            Status::Differs => "differs",
            Status::Added => "added",
            Status::Removed => "removed",
        }
    }
}

/// One part, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartStatus {
    /// The part's path, as the container holds it.
    pub part: String,
    /// What became of it.
    pub status: Status,
}

/// What two packages come to, part by part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// Every part either package holds: the first package's in the order it
    /// holds them, then the parts only the second holds, in its order.
    pub parts: Vec<PartStatus>,
    /// Whether the two hold the same parts with the same bytes. The container
    /// around them may still differ — the order of the entries, their
    /// compression, the moments they carry — because none of that is what a
    /// package says.
    pub identical: bool,
}

/// Compare `before` and `after`, part by part.
///
/// The parts of `before` come first, in the order it holds them, so that a
/// caller reading the list reads the package it started from; the parts only
/// `after` holds follow, in its order.
pub fn compare(before: &mut Package, after: &mut Package) -> Result<Difference> {
    let (theirs, mine) = (after.part_paths().to_vec(), before.part_paths().to_vec());
    let mut parts = Vec::with_capacity(mine.len() + theirs.len());
    for part in &mine {
        let status = match after.has_part(part) {
            false => Status::Removed,
            true => match before.read_part(part)? == after.read_part(part)? {
                true => Status::Identical,
                false => Status::Differs,
            },
        };
        parts.push(PartStatus {
            part: part.clone(),
            status,
        });
    }
    for part in theirs.iter().filter(|part| !before.has_part(part)) {
        parts.push(PartStatus {
            part: part.clone(),
            status: Status::Added,
        });
    }
    let identical = parts.iter().all(|part| part.status == Status::Identical);
    Ok(Difference { parts, identical })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package holding `parts`, written into a fresh temporary directory
    /// that goes when the test does.
    struct Written {
        dir: std::path::PathBuf,
    }

    impl Written {
        fn new(label: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "xlsplice-diff-{}-{}-{label}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("a test must be able to write a directory");
            Written { dir }
        }

        fn package(&self, name: &str, parts: &[(&str, &str)]) -> Package {
            let path = self.dir.join(name);
            let options = zip::write::SimpleFileOptions::default();
            let mut writer = zip::ZipWriter::new(
                std::fs::File::create(&path).expect("a test must be able to write a package"),
            );
            for (part, text) in parts {
                use std::io::Write;
                writer
                    .start_file(*part, options)
                    .expect("a part must start");
                writer
                    .write_all(text.as_bytes())
                    .expect("a part must be written");
            }
            writer.finish().expect("the container must close");
            Package::open(&path).expect("the test package must open")
        }
    }

    impl Drop for Written {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The statuses of a comparison, by part, in the order it reports them.
    fn statuses(difference: &Difference) -> Vec<(&str, &'static str)> {
        difference
            .parts
            .iter()
            .map(|part| (part.part.as_str(), part.status.as_str()))
            .collect()
    }

    #[test]
    fn two_packages_holding_the_same_parts_are_identical() {
        let written = Written::new("same");
        let parts = [("a.xml", "<a/>"), ("b.xml", "<b/>")];
        let (mut before, mut after) = (
            written.package("before.xlsx", &parts),
            written.package("after.xlsx", &parts),
        );

        let difference = compare(&mut before, &mut after).expect("both packages are readable");

        assert!(difference.identical);
        assert_eq!(
            statuses(&difference),
            [("a.xml", "identical"), ("b.xml", "identical")]
        );
    }

    #[test]
    fn a_part_whose_bytes_differ_is_reported_as_differing() {
        let written = Written::new("differs");
        let mut before = written.package("before.xlsx", &[("a.xml", "<a/>"), ("b.xml", "<b/>")]);
        let mut after = written.package("after.xlsx", &[("a.xml", "<a/>"), ("b.xml", "<b>1</b>")]);

        let difference = compare(&mut before, &mut after).expect("both packages are readable");

        assert!(!difference.identical);
        assert_eq!(
            statuses(&difference),
            [("a.xml", "identical"), ("b.xml", "differs")]
        );
    }

    /// The first package's parts come first, in its own order, and the second
    /// package's own follow.
    #[test]
    fn a_part_only_one_of_them_holds_is_added_or_removed() {
        let written = Written::new("added");
        let mut before = written.package("before.xlsx", &[("a.xml", "<a/>"), ("gone.xml", "<g/>")]);
        let mut after = written.package("after.xlsx", &[("a.xml", "<a/>"), ("new.xml", "<n/>")]);

        let difference = compare(&mut before, &mut after).expect("both packages are readable");

        assert!(!difference.identical);
        assert_eq!(
            statuses(&difference),
            [
                ("a.xml", "identical"),
                ("gone.xml", "removed"),
                ("new.xml", "added"),
            ]
        );
    }

    /// The container is not what a package says, so two packages holding the
    /// same parts in a different order still hold the same parts.
    #[test]
    fn the_order_the_container_holds_the_parts_in_is_not_a_difference() {
        let written = Written::new("order");
        let mut before = written.package("before.xlsx", &[("a.xml", "<a/>"), ("b.xml", "<b/>")]);
        let mut after = written.package("after.xlsx", &[("b.xml", "<b/>"), ("a.xml", "<a/>")]);

        let difference = compare(&mut before, &mut after).expect("both packages are readable");

        assert!(difference.identical);
        assert_eq!(
            statuses(&difference),
            [("a.xml", "identical"), ("b.xml", "identical")],
            "the first package's order is the order they are reported in"
        );
    }

    #[test]
    fn two_packages_holding_nothing_are_identical() {
        let written = Written::new("empty");
        let (mut before, mut after) = (
            written.package("before.xlsx", &[]),
            written.package("after.xlsx", &[]),
        );

        let difference = compare(&mut before, &mut after).expect("both packages are readable");

        assert!(difference.identical);
        assert_eq!(difference.parts, Vec::new());
    }
}
