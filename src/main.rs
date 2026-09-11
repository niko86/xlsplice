//! The binary: parse the command line, call the library, render the result.
//!
//! Nothing here knows how a package is spliced. Its whole job is the machine
//! contract: one envelope on stdout under `--json`, diagnostics on stderr, and
//! an exit code from the frozen table in [`xlsplice::error`].

mod cli;
mod out;
mod render;

use std::path::Path;
use std::process::ExitCode;

use clap::Parser;

use xlsplice::error::Error;
use xlsplice::package::Package;
use xlsplice::workbook::Workbook;

use crate::cli::{Cli, Command};
use crate::out::{Out, OutputMode, Verbosity};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // clap may never finish parsing, so the argv scan is what decides the
    // shape of a usage error.
    out::install_panic_hook(OutputMode::resolve(cli::json_requested(&argv)));

    match Cli::try_parse_from(&argv) {
        Ok(parsed) => {
            let out = Out::new(
                OutputMode::resolve(parsed.global.json),
                Verbosity::from_flags(parsed.global.quiet, parsed.global.verbose),
            );
            run(parsed.command, &out)
        }
        // clap never finished, so the argv scan is what decides the shape.
        Err(err) => parse_failure(err, OutputMode::resolve(cli::json_requested(&argv))),
    }
}

fn run(command: Command, out: &Out) -> ExitCode {
    out.trace(&format!(
        "xlsplice {}: running {}",
        env!("CARGO_PKG_VERSION"),
        command.name()
    ));

    match command {
        Command::Sheets { file } => match workbook_of(&file, out) {
            Ok(workbook) => out.rows(
                render::sheets(&workbook),
                &render::SHEET_HEADERS,
                &render::sheet_rows(&workbook),
            ),
            Err(err) => out.failure(&err),
        },

        Command::Names { file } => match workbook_of(&file, out) {
            Ok(workbook) => out.rows(
                render::names(&workbook),
                &render::NAME_HEADERS,
                &render::name_rows(&workbook),
            ),
            Err(err) => out.failure(&err),
        },

        Command::Version => {
            let version = env!("CARGO_PKG_VERSION");
            out.success(Version { version }, &format!("xlsplice {version}"))
        }

        #[cfg(debug_assertions)]
        Command::Selftest { fail, panic } => {
            if panic {
                panic!("selftest was asked to panic");
            }
            match fail {
                Some(code) => out.failure(&Error::new(
                    code,
                    format!("selftest stub for the {code} code"),
                )),
                None => out.success(Selftest { selftest: "ok" }, "ok"),
            }
        }
    }
}

/// Open a package and read its workbook: what both read verbs start with.
fn workbook_of(path: &Path, out: &Out) -> xlsplice::Result<Workbook> {
    out.trace(&format!("opening {}", path.display()));
    let mut package = Package::open(path)?;
    out.trace(&format!(
        "{} parts in the package",
        package.part_paths().len()
    ));
    Workbook::read(&mut package)
}

/// The payload of a `selftest` that was asked for nothing.
#[cfg(debug_assertions)]
#[derive(serde::Serialize)]
struct Selftest {
    selftest: &'static str,
}

/// The payload of `version --json`.
#[derive(serde::Serialize)]
struct Version {
    version: &'static str,
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
    Out::new(mode, Verbosity::Normal).failure(&Error::usage(usage_message(&err)))
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
