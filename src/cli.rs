//! The command line: clap's definitions, and the one question that must be
//! answered before clap runs.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[cfg(debug_assertions)]
use xlsplice::error::ErrorCode;

/// Surgical edits to Excel packages.
#[derive(Debug, Parser)]
#[command(name = "xlsplice", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub global: GlobalArgs,
}

/// Flags every verb accepts.
#[derive(Debug, clap::Args)]
pub struct GlobalArgs {
    /// Write one JSON envelope on stdout instead of human-readable text. Does
    /// not apply to `--help` or `--version`, which stay text; use the
    /// `version` verb for a version in the envelope.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress progress diagnostics. Affects stderr only; errors still print.
    #[arg(long, short, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Write extra diagnostics. Affects stderr only.
    #[arg(long, short, global = true)]
    pub verbose: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the package's sheets with their state, in workbook order.
    Sheets {
        /// The package to read.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// List the package's defined names with their scope, what each refers
    /// to, and the cell each resolves to.
    Names {
        /// The package to read.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// Print the version of xlsplice.
    Version,

    /// Produce a chosen failure, so the contract tests can reach every exit
    /// code before the verbs that raise them exist. Debug builds only, hidden,
    /// and no part of the published contract.
    #[cfg(debug_assertions)]
    #[command(hide = true)]
    Selftest {
        /// The error code to fail with, named as it appears in the envelope.
        #[arg(long, value_name = "CODE", value_parser = code_named, conflicts_with = "panic")]
        fail: Option<ErrorCode>,

        /// Panic, to exercise the panic hook.
        #[arg(long)]
        panic: bool,
    },
}

/// Look a code up by the name it carries in the envelope, so the stub has no
/// table of its own to drift from the library's. Hyphens are accepted for the
/// one code whose name has an underscore.
#[cfg(debug_assertions)]
fn code_named(name: &str) -> Result<ErrorCode, String> {
    ErrorCode::ALL
        .into_iter()
        .find(|code| code.as_str() == name || code.as_str().replace('_', "-") == name)
        .ok_or_else(|| {
            format!(
                "expected one of: {}",
                ErrorCode::ALL.map(ErrorCode::as_str).join(", ")
            )
        })
}

impl Command {
    /// The verb's name, for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Sheets { .. } => "sheets",
            Command::Names { .. } => "names",
            Command::Version => "version",
            #[cfg(debug_assertions)]
            Command::Selftest { .. } => "selftest",
        }
    }
}

/// Whether `--json` appears in `argv` as a flag.
///
/// clap cannot answer this, because a usage error means clap never finished
/// parsing, and a panic may happen before it starts; where clap does finish,
/// its own answer is the one used. Everything after `--` is an operand, not a
/// flag, so the scan stops there, and `--json=...` counts, because that is how
/// clap lexes a long flag even when it goes on to reject the value.
pub fn json_requested<I, S>(argv: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    argv.into_iter()
        .skip(1)
        .take_while(|arg| arg.as_ref() != "--")
        .any(|arg| arg.as_ref() == "--json" || arg.as_ref().starts_with("--json="))
}
