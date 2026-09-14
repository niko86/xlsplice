//! The binary: parse the command line, call the library, write what it says.
//!
//! Nothing here knows how a package is spliced, and nothing here decides what
//! the machine contract looks like: a verb comes back with an [`Answer`] or an
//! [`Error`], the library renders it, and this is what puts the two streams
//! where they go and exits.

mod cli;
mod out;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::Parser;

use xlsplice::answer;
use xlsplice::error::Error;
use xlsplice::render::OutputMode;
use xlsplice::verb::{self, Trace};

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
    // The trace goes to stderr, and only when it was asked for. Built once
    // here so that every verb below says what it is doing into the same
    // place.
    let sink = |message: &str| out.trace(message);
    let trace = Trace::To(&sink);

    out.emit(match command {
        Command::Sheets { file } => verb::sheets(&file, &trace),
        Command::Names { file } => verb::names(&file, &trace),
        Command::Get { file, targets } => verb::get(&file, &targets, &trace),
        Command::Diff {
            file,
            other,
            exit_code,
        } => verb::difference(&file, &other, exit_code, &trace),
        Command::Set {
            file,
            target,
            value,
            kind,
            formulas,
            landing,
        } => verb::set(
            &file,
            &target,
            kind,
            &value,
            formulas.replace_formula,
            &landing.destination(),
            landing.dry_run,
            &trace,
        ),
        Command::Clear {
            file,
            target,
            formulas,
            landing,
        } => verb::clear(
            &file,
            &target,
            formulas.replace_formula,
            &landing.destination(),
            landing.dry_run,
            &trace,
        ),
        Command::Calc {
            file,
            full_calc_on_load,
            landing,
        } => verb::calc(
            &file,
            full_calc_on_load,
            &landing.destination(),
            landing.dry_run,
            &trace,
        ),
        Command::Props { action } => match action {
            cli::PropsAction::Get { file } => verb::props(&file, &trace),
            cli::PropsAction::Set {
                file,
                name,
                value,
                kind,
                landing,
            } => verb::props_set(
                &file,
                &name,
                kind,
                &value,
                &landing.destination(),
                landing.dry_run,
                &trace,
            ),
            cli::PropsAction::Unset {
                file,
                name,
                landing,
            } => verb::props_unset(
                &file,
                &name,
                &landing.destination(),
                landing.dry_run,
                &trace,
            ),
        },
        Command::Apply {
            file,
            batch,
            landing,
        } => verb::apply(
            &file,
            &batch,
            std::io::stdin().is_terminal(),
            &landing.destination(),
            landing.dry_run,
            &trace,
        ),
        Command::Help { topic } => answer::topic(topic),
        Command::Version => answer::version(),
        #[cfg(debug_assertions)]
        Command::Selftest { .. } => panic!("selftest was asked to panic"),
    })
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
