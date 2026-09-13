//! The process end of the machine contract: the two streams, and the exit.
//!
//! What goes on each stream and what the process exits with are the library's
//! to decide, in [`xlsplice::render`]. What is left here is what only a
//! process can do: ask whether stdout is a terminal, write the bytes, exit
//! with the code, and keep the diagnostic channel at the volume the caller
//! asked for. Neither stream is ever coloured.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use xlsplice::answer::Answer;
use xlsplice::error::Error;
use xlsplice::render::{self, OutputMode, Rendered, render};

/// The output mode for this run, given whether `--json` was asked for.
/// Whether stdout is a terminal is asked here, once, and not rediscovered at
/// each write.
pub fn mode(json: bool) -> OutputMode {
    OutputMode::new(json, std::io::stdout().is_terminal())
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

/// Where a command's answer goes, and how loud the running commentary is.
pub struct Out {
    mode: OutputMode,
    verbosity: Verbosity,
}

impl Out {
    /// Build the output channel for this run.
    pub fn new(mode: OutputMode, verbosity: Verbosity) -> Self {
        Out { mode, verbosity }
    }

    /// Write what a verb came back with, and give back the code to exit with.
    pub fn emit(&self, outcome: xlsplice::Result<Answer>) -> ExitCode {
        write(render(outcome, self.mode))
    }

    /// The same, but exiting `on_success` rather than 0 where it succeeded.
    ///
    /// One verb wants this. `diff --exit-code` says in its exit code whether
    /// the two packages differ, which is diff(1)'s convention; a failure still
    /// exits with its own code from the frozen table, because what went wrong
    /// is what a caller has to hear first.
    pub fn emit_exiting(&self, outcome: xlsplice::Result<Answer>, on_success: u8) -> ExitCode {
        let mut rendered = render(outcome, self.mode);
        if rendered.exit == xlsplice::error::EXIT_SUCCESS {
            rendered.exit = on_success;
        }
        write(rendered)
    }

    /// A trace of what the command is doing, on stderr, only when asked.
    pub fn trace(&self, message: &str) {
        if self.verbosity == Verbosity::Verbose {
            eprintln!("{message}");
        }
    }
}

/// Put the two streams where they go and give back the exit code.
fn write(rendered: Rendered) -> ExitCode {
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
    ExitCode::from(rendered.exit)
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
        let exit = error.exit_code();

        eprint!("{}", render::diagnostic(&error));
        if let OutputMode::Json(_) = mode {
            print!("{}", render(Err(error), mode).stdout);
        }
        let _ = std::io::stdout().flush();
        std::process::exit(i32::from(exit));
    }));
}
