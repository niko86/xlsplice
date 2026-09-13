//! The one write path: what every writing verb does once it has said what it
//! asks of the package.
//!
//! A writing verb's own work is building the operations. `set` builds one out
//! of a target and a value; `apply` will read a batch of them; `props set`,
//! `props unset`, `clear` and `calc` will each build their own. Everything
//! after that is the same for all of them — a destination, a dry-run flag, the
//! run itself, the trace of what it did and the answer it comes back with — so
//! it is here, once, rather than copied into each verb.

use std::io::{IsTerminal, Read};
use std::path::Path;

use xlsplice::answer::{self, Answer};
use xlsplice::batch::{self, Batch, Operation};
use xlsplice::calculation;
use xlsplice::error::Error;
use xlsplice::package::Package;
use xlsplice::properties;
use xlsplice::workbook::Workbook;

use crate::cli::Landing;
use crate::out::Out;

/// The operand that means stdin rather than a path.
const STDIN: &str = "-";

/// The `calc` verb: report the calculation flag, or say what it should be.
///
/// Reading writes nothing, so `--out` and `--dry-run` have nothing to do
/// where the flag is not being set, and saying so is better than accepting
/// them and quietly ignoring them.
pub fn calc(
    file: &Path,
    full_calc_on_load: bool,
    landing: &Landing,
    out: &Out,
) -> xlsplice::Result<Answer> {
    if full_calc_on_load {
        let batch = Batch::of(Operation::Calc {
            full_calc_on_load: true,
        });
        return run(file, &batch, landing, out);
    }
    if landing.out.is_some() || landing.dry_run {
        return Err(Error::usage(
            "calc with no --full-calc-on-load reads the flag and writes nothing, so \
             --out and --dry-run have nothing to do. Add --full-calc-on-load to set it."
                .to_owned(),
        ));
    }
    out.trace(&format!(
        "reading the calculation flag of {}",
        file.display()
    ));
    let mut package = Package::open(file)?;
    let workbook = Workbook::read(&mut package)?;
    let part = workbook.part().to_owned();
    let flag = calculation::full_calc_on_load(package.read_part_text(&part)?)
        .map_err(|err| err.within(&part))?;
    answer::calculation(flag)
}

/// The `props get` verb: every custom document property the package holds.
///
/// A package holding none answers with an empty list rather than a failure:
/// having no custom properties is a thing a package is, not something wrong
/// with it.
pub fn props(file: &Path, out: &Out) -> xlsplice::Result<Answer> {
    out.trace(&format!(
        "reading the custom document properties of {}",
        file.display()
    ));
    let mut package = Package::open(file)?;
    let (part, held) = properties::part_of(&mut package)?;
    let found = match held {
        false => Vec::new(),
        true => {
            properties::read(package.read_part_text(&part)?).map_err(|err| err.within(&part))?
        }
    };
    out.trace(&format!("{} propert(ies) in {part}", found.len()));
    answer::properties(&found)
}

/// The `apply` verb: read the batch the operand names, and run it.
///
/// Reading the batch is the only thing `apply` does that `set` does not; from
/// there it is the same path every writing verb takes.
pub fn apply(
    file: &Path,
    operand: &Path,
    landing: &Landing,
    out: &Out,
) -> xlsplice::Result<Answer> {
    out.trace(&format!(
        "reading the batch at {}",
        match operand == Path::new(STDIN) {
            true => "stdin".to_owned(),
            false => operand.display().to_string(),
        }
    ));
    let batch = batch_at(operand, stdin_is_terminal())?;
    run(file, &batch, landing, out)
}

/// The batch an operand names: a JSON array of operations in a file, or one
/// read from stdin where the operand is a single dash.
///
/// A dash with a terminal on stdin is a caller who meant to pipe a batch in
/// and did not. Waiting for one to be typed would look like a hang, so it is
/// a usage error instead. `is_terminal` is asked of the process by the caller,
/// so that what this does with the answer can be tested without one.
fn batch_at(operand: &Path, is_terminal: bool) -> xlsplice::Result<Batch> {
    Batch::parse(&document(operand, is_terminal)?)
}

/// The text of the batch, from wherever the operand says it is.
fn document(operand: &Path, is_terminal: bool) -> xlsplice::Result<String> {
    if operand != Path::new(STDIN) {
        return std::fs::read_to_string(operand).map_err(|err| {
            Error::unreadable(format!(
                "cannot read the batch at {}: {err}. Give the path of a JSON \
                 array of operations, or {STDIN} to read one from stdin.",
                operand.display()
            ))
        });
    }
    if is_terminal {
        return Err(Error::usage(format!(
            "{STDIN} reads the batch from stdin, and stdin is a terminal. Pipe \
             a JSON array of operations in, or give the path of a file holding \
             one."
        )));
    }
    let mut json = String::new();
    std::io::stdin()
        .read_to_string(&mut json)
        .map_err(|err| Error::unreadable(format!("cannot read the batch from stdin: {err}")))?;
    Ok(json)
}

/// Whether stdin is a terminal, asked of the process once.
fn stdin_is_terminal() -> bool {
    std::io::stdin().is_terminal()
}

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
/// the order they were given, with an operation that names none standing as
/// what it does, so that the list is still as long as the batch.
fn targets(batch: &Batch) -> String {
    batch
        .operations
        .iter()
        .map(|operation| operation.target().unwrap_or_else(|| operation.kind()))
        .collect::<Vec<&str>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    use xlsplice::ErrorCode;

    /// Whether stdin is a terminal is asked of the process, which a test has
    /// no way to make one of; what is done with the answer is asked here.
    #[test]
    fn a_dash_with_a_terminal_on_stdin_is_a_usage_error() {
        let err = batch_at(Path::new(STDIN), true).expect_err("nothing was piped in");

        assert_eq!(err.code(), ErrorCode::Usage);
        assert!(err.message().contains("stdin is a terminal"), "{err}");
    }

    /// The terminal is only ever a question about the dash: an operand that
    /// is a path is read whatever stdin happens to be.
    #[test]
    fn a_path_operand_is_read_whatever_stdin_is() {
        let err = batch_at(Path::new("no-such-batch.json"), true).expect_err("no such file");

        assert_eq!(err.code(), ErrorCode::Unreadable);
        assert!(err.message().contains("no-such-batch.json"), "{err}");
    }
}
