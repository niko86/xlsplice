//! The binary: parse the command line, call the library, write what it says.
//!
//! Nothing here knows how a package is spliced, and nothing here decides what
//! the machine contract looks like: a verb comes back with an [`Answer`] or an
//! [`Error`], the library renders it, and this is what puts the two streams
//! where they go and exits.

mod cli;
mod out;
mod write;

use std::path::Path;
use std::process::ExitCode;

use clap::Parser;

use xlsplice::answer::{self, Answer};
use xlsplice::batch::{Batch, Operation};
use xlsplice::cells;
use xlsplice::error::Error;
use xlsplice::package::Package;
use xlsplice::render::OutputMode;
use xlsplice::workbook::Workbook;

use crate::cli::{Cli, Command};
use crate::out::{Out, Verbosity};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // clap may never finish parsing, so the argv scan is what decides the
    // shape of a usage error.
    out::install_panic_hook(out::mode(cli::json_requested(&argv)));

    match Cli::try_parse_from(&argv) {
        Ok(parsed) => {
            let out = Out::new(
                out::mode(parsed.global.json),
                Verbosity::from_flags(parsed.global.quiet, parsed.global.verbose),
            );
            run(parsed.command, &out)
        }
        // clap never finished, so the argv scan is what decides the shape.
        Err(err) => parse_failure(err, out::mode(cli::json_requested(&argv))),
    }
}

fn run(command: Command, out: &Out) -> ExitCode {
    out.trace(&format!(
        "xlsplice {}: running {}",
        env!("CARGO_PKG_VERSION"),
        command.name()
    ));

    out.emit(match command {
        Command::Sheets { file } => sheets(&file, out),
        Command::Names { file } => names(&file, out),
        Command::Get { file, targets } => get(&file, &targets, out),
        Command::Set {
            file,
            target,
            value,
            kind,
            formulas,
            landing,
        } => write::run(
            &file,
            &Batch::of(Operation::Set {
                target,
                write_type: kind,
                value,
                replace_formula: formulas.replace_formula,
            }),
            &landing,
            out,
        ),
        Command::Clear {
            file,
            target,
            formulas,
            landing,
        } => write::run(
            &file,
            &Batch::of(Operation::Clear {
                target,
                replace_formula: formulas.replace_formula,
            }),
            &landing,
            out,
        ),
        Command::Apply {
            file,
            batch,
            landing,
        } => write::apply(&file, &batch, &landing, out),
        Command::Version => answer::version(),
        #[cfg(debug_assertions)]
        Command::Selftest { .. } => panic!("selftest was asked to panic"),
    })
}

/// Open a package and read its workbook: what every read verb starts with.
/// The package comes back too, because a verb that reads cells goes on to
/// read more of its parts.
fn open(path: &Path, out: &Out) -> xlsplice::Result<(Package, Workbook)> {
    out.trace(&format!("opening {}", path.display()));
    let mut package = Package::open(path)?;
    out.trace(&format!(
        "{} parts in the package",
        package.part_paths().len()
    ));
    let workbook = Workbook::read(&mut package)?;
    out.trace(&format!("workbook part: {}", workbook.part()));
    Ok((package, workbook))
}

/// List the package's sheets.
fn sheets(path: &Path, out: &Out) -> xlsplice::Result<Answer> {
    let (_, workbook) = open(path, out)?;
    answer::sheets(&workbook)
}

/// List the package's defined names.
fn names(path: &Path, out: &Out) -> xlsplice::Result<Answer> {
    let (_, workbook) = open(path, out)?;
    answer::names(&workbook)
}

/// Read the cells `targets` name.
fn get(path: &Path, targets: &[String], out: &Out) -> xlsplice::Result<Answer> {
    let (mut package, workbook) = open(path, out)?;
    out.trace(&format!("reading {} target(s)", targets.len()));
    answer::cells(&cells::read(&mut package, &workbook, targets)?)
}

/// Render what clap gave back instead of a command.
///
/// `--help` and `--version` are requests, not failures: they print clap's text
/// on stdout and exit 0 even under `--json`, which is why `version` exists as
/// a verb. Everything else is a usage error, and goes down the same path as
/// any other failure so that one rule decides the shape of all of them.
fn parse_failure(err: clap::Error, mode: OutputMode) -> ExitCode {
    if !err.use_stderr() {
        let _ = err.print();
        return ExitCode::from(xlsplice::error::EXIT_SUCCESS);
    }
    Out::new(mode, Verbosity::Normal).emit(Err(Error::usage(usage_message(&err))))
}

/// clap's rendered error, minus its `error: ` prefix: the code in the envelope
/// already says it failed, and everything after the prefix, the offending
/// argument and the usage line, is what tells a caller how to correct itself.
fn usage_message(err: &clap::Error) -> String {
    let rendered = err.render().to_string();
    let text = rendered.trim();
    text.strip_prefix("error: ")
        .unwrap_or(text)
        .trim()
        .to_owned()
}
