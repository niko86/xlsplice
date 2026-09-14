//! The process end of the machine contract: the two streams, and the exit.
//!
//! What goes on each stream and what the process exits with are the library's
//! to decide, in [`xlsplice::render`]. What is left here is what only a
//! process can do: ask whether stdout is a terminal, write the bytes, exit
//! with the code, and keep the diagnostic channel at the volume the caller
//! asked for. Neither stream is ever coloured.
//!
//! The panic hook was here too until #44. It is the same kind of thing and
//! would sit happily beside these, but it has to be reachable from outside
//! this binary to be watched working, so it lives in [`xlsplice::crash`].

use std::io::IsTerminal;
use std::process::ExitCode;

use xlsplice::answer::Answer;
use xlsplice::render::{OutputMode, Rendered, render};

/// The output mode for this run, given whether `--json` was asked for.
/// Whether stdout is a terminal is asked here, once, and not rediscovered at
/// each write.
pub fn mode(json: bool) -> OutputMode {
    OutputMode::new(json, std::io::stdout().is_terminal())
}

/// How much of the diagnostic channel a caller asked for. It moves stderr
/// only; stdout carries the same bytes at either volume. Errors ignore it: a
/// failure stays visible even under `--quiet`.
///
/// Two volumes rather than three, because there are two: a run that was not
/// asked to be loud says nothing on stderr, so there is nothing `--quiet`
/// could take away. A third value between them would be a level the code
/// never reads and a promise the output does not keep. The flag itself stays
/// on the published surface and says in its help that it changes nothing;
/// giving the default something to suppress is what would earn it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// Nothing on stderr but errors: the default, and what `--quiet` asks
    /// for.
    Quiet,
    /// `--verbose`: traces of what the command is doing.
    Verbose,
}

impl Verbosity {
    /// The volume `--verbose` asks for.
    ///
    /// It is the only flag that moves this. `--quiet` asks for the default,
    /// so nothing here reads it — which is the point of the flag doing
    /// nothing, said in code rather than only in the help. That the two
    /// cannot be asked for at once is clap's, and
    /// `quiet_and_verbose_cannot_both_be_asked_for` holds the real binary to
    /// it.
    pub fn asked_for(verbose: bool) -> Self {
        match verbose {
            true => Verbosity::Verbose,
            false => Verbosity::Quiet,
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
    ///
    /// Every verb goes through here, `diff --exit-code` included: the code a
    /// success exits with is the answer's, and `render` reads it, so nothing
    /// in this process decides one.
    pub fn emit(&self, outcome: xlsplice::Result<Answer>) -> ExitCode {
        write(render(outcome, self.mode))
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
