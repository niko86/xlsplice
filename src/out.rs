//! The two output streams, and the rule that keeps them separate.
//!
//! stdout is the data channel: under `--json` it carries exactly one envelope
//! and nothing else. stderr is the diagnostic channel: `--quiet` and
//! `--verbose` move its volume and never touch stdout. Neither stream is ever
//! coloured.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use serde::Serialize;

use xlsplice::envelope::{Envelope, JsonStyle};
use xlsplice::error::{EXIT_SUCCESS, Error};

/// What stdout carries, and in what shape. `--json` decides which arm; stdout
/// itself decides the style, so the choice is made once, here, and not
/// rediscovered at each write.
#[derive(Debug, Clone, Copy)]
pub enum OutputMode {
    /// Human-readable text.
    Text,
    /// One JSON envelope, pretty on a terminal and compact in a pipe.
    Json(JsonStyle),
}

impl OutputMode {
    /// The mode for this run, given whether `--json` was asked for.
    pub fn resolve(json: bool) -> Self {
        if json {
            OutputMode::Json(JsonStyle::for_terminal(std::io::stdout().is_terminal()))
        } else {
            OutputMode::Text
        }
    }
}

/// How much of the diagnostic channel a caller asked for. It moves stderr
/// only; stdout carries the same bytes at every level. Errors ignore it: a
/// failure stays visible even under `--quiet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// `--quiet`: nothing on stderr but errors.
    Quiet,
    /// The default.
    Normal,
    /// `--verbose`: traces of what the command is doing.
    Verbose,
}

impl Verbosity {
    /// The level the global flags ask for. clap keeps them mutually exclusive.
    pub fn from_flags(quiet: bool, verbose: bool) -> Self {
        match (quiet, verbose) {
            (true, _) => Verbosity::Quiet,
            (_, true) => Verbosity::Verbose,
            _ => Verbosity::Normal,
        }
    }
}

/// Where a command's result goes, and in what shape.
pub struct Out {
    mode: OutputMode,
    verbosity: Verbosity,
}

impl Out {
    /// Build the output channel for this run.
    pub fn new(mode: OutputMode, verbosity: Verbosity) -> Self {
        Out { mode, verbosity }
    }

    /// Report success: the envelope under `--json`, otherwise the text.
    pub fn success<T: Serialize>(&self, payload: T, text: &str) -> ExitCode {
        match self.mode {
            OutputMode::Json(style) => println!("{}", Envelope::success(payload).render(style)),
            OutputMode::Text => println!("{text}"),
        }
        ExitCode::from(EXIT_SUCCESS)
    }

    /// Report a failure and give back the exit code it maps to. Under `--json`
    /// the envelope is the whole response and stderr stays clean.
    pub fn failure(&self, error: &Error) -> ExitCode {
        match self.mode {
            OutputMode::Json(style) => println!("{}", Envelope::failure(error).render(style)),
            OutputMode::Text => eprintln!("error: {error}"),
        }
        ExitCode::from(error.exit_code())
    }

    /// A trace of what the command is doing, on stderr, only when asked.
    pub fn trace(&self, message: &str) {
        if self.verbosity == Verbosity::Verbose {
            eprintln!("{message}");
        }
    }
}

/// Route every panic through the envelope, so no failure mode is unparseable.
///
/// The hook exits the process itself rather than letting the unwind reach
/// `main`, which would exit 101 and skip the contract. Exiting here also skips
/// destructors, which is what the "non-zero means nothing was written"
/// guarantee wants: a half-finished temporary file is abandoned, not renamed.
///
/// A crash is exceptional, so the diagnostic goes to stderr in every mode,
/// where under `--json` stdout is otherwise reserved for the envelope. The
/// mode comes from the argv scan, because a panic can precede clap.
pub fn install_panic_hook(mode: OutputMode) {
    std::panic::set_hook(Box::new(move |info| {
        let at = info
            .location()
            .map(|at| format!(" at {}:{}", at.file(), at.line()))
            .unwrap_or_default();
        let what = info.payload_as_str().unwrap_or("panicked");
        let error = Error::internal(format!(
            "internal error{at}: {what}. This is a bug in xlsplice; \
             please report it with the command you ran."
        ));

        eprintln!("error: {error}");
        if let OutputMode::Json(_) = mode {
            Out::new(mode, Verbosity::Normal).failure(&error);
        }
        let _ = std::io::stdout().flush();
        std::process::exit(i32::from(error.exit_code()));
    }));
}
