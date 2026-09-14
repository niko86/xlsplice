//! The verbs: one function per command, each answering an [`Answer`] or an
//! [`Error`].
//!
//! They live here rather than in the binary because a caller that is not a
//! process wants them too. While they sat behind `main.rs`'s private modules
//! the test suites kept a hand-written twin of this file — and the twin
//! drifted, losing the licence to replace a formula that the real dispatch
//! passed, which was then patched back by a helper that existed only to undo
//! the loss. One copy cannot drift.
//!
//! What is *not* here is anything a process does: no argv, no streams, no
//! exit code, no clap. A verb takes what it needs, says what it is doing
//! through a [`Trace`], and answers. `main.rs` is what turns a command line
//! into one of these calls and what puts the answer where it goes.

use std::io::Read;
use std::path::Path;

use crate::answer::{self, Answer};
use crate::batch::{self, Batch, Destination, Operation, WriteType};
use crate::calculation;
use crate::cells;
use crate::diff::{self, Difference};
use crate::error::Error;
use crate::package::{FromFile, Package};
use crate::properties;
use crate::workbook::Workbook;

/// The operand that means stdin rather than a path.
const STDIN: &str = "-";

/// Where a verb says what it is doing, when anyone is listening.
///
/// `--verbose` is the only thing that listens, so the ordinary run and every
/// test pass [`Trace::Off`] and the lines cost nothing. It is a sink rather
/// than a flag because the tracing happens *inside* a verb, a dozen times
/// over: what part was opened, how many were in the package, what a batch is
/// pointed at, what it changed. Moved to the edges those lines would be lost.
pub enum Trace<'a> {
    /// Nobody is listening.
    Off,
    /// Say it to this.
    To(&'a dyn Fn(&str)),
}

impl Trace<'_> {
    /// Say what is happening, if anyone asked to hear it.
    pub fn say(&self, message: &str) {
        if let Trace::To(sink) = self {
            sink(message);
        }
    }
}

/// Open a package and read its workbook: what every read verb starts with.
///
/// The package comes back too, because a verb that reads cells goes on to
/// read more of its parts.
fn open(path: &Path, trace: &Trace) -> crate::Result<(Package<FromFile>, Workbook)> {
    trace.say(&format!("opening {}", path.display()));
    let mut package = Package::open(path)?;
    trace.say(&format!(
        "{} parts in the package",
        package.part_paths().len()
    ));
    let workbook = Workbook::read(&mut package)?;
    trace.say(&format!("workbook part: {}", workbook.part()));
    Ok((package, workbook))
}

/// `sheets`: the package's sheets, in the order the workbook holds them.
pub fn sheets(path: &Path, trace: &Trace) -> crate::Result<Answer> {
    let (_, workbook) = open(path, trace)?;
    answer::sheets(&workbook)
}

/// `names`: the package's defined names, with what each resolves to.
pub fn names(path: &Path, trace: &Trace) -> crate::Result<Answer> {
    let (_, workbook) = open(path, trace)?;
    answer::names(&workbook)
}

/// `get`: the cells `targets` name.
pub fn get(path: &Path, targets: &[String], trace: &Trace) -> crate::Result<Answer> {
    let (mut package, workbook) = open(path, trace)?;
    trace.say(&format!("reading {} target(s)", targets.len()));
    answer::cells(&cells::read(&mut package, &workbook, targets)?)
}

/// `diff`: which parts differ between two packages.
///
/// Reading two packages is the whole of it and neither is written to, so
/// nothing here goes near the write path. `in_the_exit_code` is
/// `--exit-code`, and it goes no further than the answer: the comparison is
/// the same either way, and what changes is the code a success exits with,
/// which the answer carries and `render` reads.
pub fn difference(
    a: &Path,
    b: &Path,
    in_the_exit_code: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    trace.say(&format!("comparing {} with {}", a.display(), b.display()));
    let found = compared(a, b)?;
    trace.say(&format!("{} part(s) between them", found.parts.len()));
    answer::difference(&found, in_the_exit_code)
}

/// Open both packages and compare them.
fn compared(a: &Path, b: &Path) -> crate::Result<Difference> {
    let mut before = Package::open(a)?;
    let mut after = Package::open(b)?;
    diff::compare(&mut before, &mut after)
}

/// `props get`: every custom document property the package holds.
///
/// A package holding none answers with an empty list rather than a failure:
/// having no custom properties is a thing a package is, not something wrong
/// with it.
pub fn props(file: &Path, trace: &Trace) -> crate::Result<Answer> {
    trace.say(&format!(
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
    trace.say(&format!("{} propert(ies) in {part}", found.len()));
    answer::properties(&found)
}

/// `calc`: report the calculation flag, or say what it should be.
///
/// Reading writes nothing, so `--out` and `--dry-run` have nothing to do
/// where the flag is not being set, and saying so is better than accepting
/// them and quietly ignoring them.
pub fn calc(
    file: &Path,
    full_calc_on_load: bool,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    if full_calc_on_load {
        let batch = Batch::of(Operation::Calc {
            full_calc_on_load: true,
        });
        return run(file, &batch, destination, dry_run, trace);
    }
    if *destination != Destination::InPlace || dry_run {
        return Err(Error::usage(
            "calc with no --full-calc-on-load reads the flag and writes nothing, so \
             --out and --dry-run have nothing to do. Add --full-calc-on-load to set it."
                .to_owned(),
        ));
    }
    trace.say(&format!(
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

/// `set`: write one value into one target.
///
/// As many arguments as the command line has operands and flags, because that
/// is what it is: `FILE TARGET VALUE --type --replace-formula --out
/// --dry-run`. Grouping them would put a second spelling of the command
/// between the command and the verb, which is the thing this module exists to
/// remove.
#[allow(clippy::too_many_arguments)]
pub fn set(
    file: &Path,
    target: &str,
    write_type: WriteType,
    value: &str,
    replace_formula: bool,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    let batch = Batch::of(Operation::Set {
        target: target.to_owned(),
        write_type,
        value: value.to_owned(),
        replace_formula,
    });
    run(file, &batch, destination, dry_run, trace)
}

/// `clear`: empty one target, keeping the style it renders under.
pub fn clear(
    file: &Path,
    target: &str,
    replace_formula: bool,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    let batch = Batch::of(Operation::Clear {
        target: target.to_owned(),
        replace_formula,
    });
    run(file, &batch, destination, dry_run, trace)
}

/// `props set`: stamp one typed custom document property.
pub fn props_set(
    file: &Path,
    name: &str,
    write_type: WriteType,
    value: &str,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    let batch = Batch::of(Operation::PropsSet {
        name: name.to_owned(),
        write_type,
        value: value.to_owned(),
    });
    run(file, &batch, destination, dry_run, trace)
}

/// `props unset`: withdraw one custom document property.
pub fn props_unset(
    file: &Path,
    name: &str,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    let batch = Batch::of(Operation::PropsUnset {
        name: name.to_owned(),
    });
    run(file, &batch, destination, dry_run, trace)
}

/// `apply`: read the batch the operand names, and run it.
///
/// Reading the batch is the only thing `apply` does that `set` does not; from
/// there it is the same path every writing verb takes. Whether stdin is a
/// terminal is asked of the process by the caller, so what this does with the
/// answer can be tested without one.
pub fn apply(
    file: &Path,
    operand: &Path,
    stdin_is_terminal: bool,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    trace.say(&format!(
        "reading the batch at {}",
        match operand == Path::new(STDIN) {
            true => "stdin".to_owned(),
            false => operand.display().to_string(),
        }
    ));
    let batch = batch_at(operand, stdin_is_terminal)?;
    run(file, &batch, destination, dry_run, trace)
}

/// The batch an operand names: a JSON array of operations in a file, or one
/// read from stdin where the operand is a single dash.
fn batch_at(operand: &Path, is_terminal: bool) -> crate::Result<Batch> {
    Batch::parse(&document(operand, is_terminal)?)
}

/// The text of the batch, from wherever the operand says it is.
///
/// A dash with a terminal on stdin is a caller who meant to pipe a batch in
/// and did not. Waiting for one to be typed would look like a hang, so it is
/// a usage error instead.
fn document(operand: &Path, is_terminal: bool) -> crate::Result<String> {
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

/// The one write path: what every writing verb does once it has said what it
/// asks of the package.
///
/// A writing verb's own work is building the operations; everything after
/// that — the destination, the dry-run flag, the run itself, the trace of
/// what it did and the answer it comes back with — is the same for all of
/// them, so it is here, once, rather than copied into each.
pub fn run(
    file: &Path,
    batch: &Batch,
    destination: &Destination,
    dry_run: bool,
    trace: &Trace,
) -> crate::Result<Answer> {
    trace.say(&format!(
        "writing {} in {}{}",
        targets(batch),
        destination.path(file).display(),
        if dry_run { " (dry run)" } else { "" }
    ));
    let report = batch::run(file, batch, destination, dry_run)?;
    trace.say(&format!(
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

    use crate::ErrorCode;

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

    /// Nobody is listening by default, and a verb that says something into
    /// [`Trace::Off`] is saying it to nothing at all.
    #[test]
    fn a_trace_nobody_asked_for_says_nothing() {
        let said = std::cell::RefCell::new(Vec::new());
        let sink = |message: &str| said.borrow_mut().push(message.to_owned());

        Trace::Off.say("into the void");
        assert!(said.borrow().is_empty());

        Trace::To(&sink).say("out loud");
        assert_eq!(said.borrow().as_slice(), ["out loud"]);
    }
}
