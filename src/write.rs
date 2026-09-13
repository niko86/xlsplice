//! The one write path: what every writing verb does once it has said what it
//! asks of the package.
//!
//! A writing verb's own work is building the operations. `set` builds one out
//! of a target and a value; `apply` will read a batch of them; `props set`,
//! `props unset`, `clear` and `calc` will each build their own. Everything
//! after that is the same for all of them — a destination, a dry-run flag, the
//! run itself, the trace of what it did and the answer it comes back with — so
//! it is here, once, rather than copied into each verb.

use std::path::Path;

use xlsplice::answer::{self, Answer};
use xlsplice::batch::{self, Batch};

use crate::cli::Landing;
use crate::out::Out;

/// Apply `batch` to the package at `file`, land the result where `landing`
/// says, and answer with what it did.
pub fn run(file: &Path, batch: &Batch, landing: &Landing, out: &Out) -> xlsplice::Result<Answer> {
    let destination = landing.destination();
    out.trace(&format!(
        "writing {} in {}{}",
        targets(batch),
        destination.path(file).display(),
        if landing.dry_run { " (dry run)" } else { "" }
    ));
    let report = batch::run(file, batch, &destination, landing.dry_run)?;
    out.trace(&format!(
        "{} part(s) changed: {}",
        report.parts.changed.len(),
        match report.parts.changed.is_empty() {
            true => "none".to_owned(),
            false => report.parts.changed.join(", "),
        }
    ));
    answer::written(&report)
}

/// What the batch is pointed at, for the trace: every operation's target, in
/// the order they were given.
fn targets(batch: &Batch) -> String {
    batch
        .operations
        .iter()
        .map(|operation| operation.target())
        .collect::<Vec<&str>>()
        .join(", ")
}
