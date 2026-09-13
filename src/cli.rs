//! The command line: clap's definitions, and the one question that must be
//! answered before clap runs.

use std::path::PathBuf;

use clap::builder::{PossibleValue, PossibleValuesParser, TypedValueParser};
use clap::{Parser, Subcommand};

use xlsplice::batch::{Destination, WriteType};

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

/// Where a writing verb's result lands: the two flags every one of them
/// takes.
///
/// Declared once and flattened into each writing verb, so that `set`,
/// `clear`, `apply`, `props` and `calc` cannot drift apart in how they spell
/// the same two questions. Not global, because a read verb has no result to
/// land anywhere.
#[derive(Debug, clap::Args)]
pub struct Landing {
    /// Write the result here instead, leaving FILE untouched. An existing
    /// file at this path is replaced.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Do everything but put the result anywhere, and report what would
    /// have changed.
    #[arg(long)]
    pub dry_run: bool,
}

impl Landing {
    /// Where the result goes: the path `--out` names, or the package itself.
    pub fn destination(&self) -> Destination {
        Destination::from(self.out.clone())
    }
}

/// What a writing verb may do to a formula in the cell it was pointed at.
///
/// Declared once and flattened into each cell-writing verb, as [`Landing`] is.
/// `apply` does not take it: a batch says it per operation, so that one
/// operation licensing a replacement cannot license another's.
#[derive(Debug, clap::Args)]
pub struct Formulas {
    /// Replace a formula in the target cell, dropping it and taking its calc
    /// chain entry with it. Without this a cell holding a formula is refused,
    /// so that a mis-addressed write cannot silently destroy one. A shared
    /// formula's master is refused even with this, because overwriting it
    /// orphans the rest of its range.
    #[arg(long)]
    pub replace_formula: bool,
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

    /// Read one or more cells, each named by an address or a defined name.
    Get {
        /// The package to read.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// The cells to read: `Sheet!A1`, a workbook-scoped defined name, or
        /// `Sheet!Name` for one scoped to a sheet. A name resolves to its
        /// anchor. One result comes back per target, in the order given.
        #[arg(value_name = "TARGET", required = true)]
        targets: Vec<String>,
    },

    /// Write a value into one cell, named by an address or a defined name.
    ///
    /// The cell must already be there, and must not hold a formula. Every
    /// part of the package outside the cell is copied byte for byte.
    Set {
        /// The package to write. Written in place unless `--out` is given.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// The cell to write: `Sheet!A1`, a workbook-scoped defined name, or
        /// `Sheet!Name` for one scoped to a sheet. A name resolves to its
        /// anchor.
        #[arg(value_name = "TARGET")]
        target: String,

        /// The value, read according to `--type`.
        #[arg(value_name = "VALUE")]
        value: String,

        /// How to read VALUE and store it: a number, text written as an
        /// inline string, or a boolean.
        #[arg(long = "type", value_name = "TYPE", value_parser = write_type())]
        kind: WriteType,

        #[command(flatten)]
        formulas: Formulas,

        #[command(flatten)]
        landing: Landing,
    },

    /// Empty one cell, named by an address or a defined name.
    ///
    /// The cell keeps its element and its style and loses its value, its type
    /// and any inline string, which is how Excel leaves a cell whose contents
    /// were deleted. A cell holding a formula is refused.
    Clear {
        /// The package to write. Written in place unless `--out` is given.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// The cell to empty: `Sheet!A1`, a workbook-scoped defined name, or
        /// `Sheet!Name` for one scoped to a sheet. A name resolves to its
        /// anchor.
        #[arg(value_name = "TARGET")]
        target: String,

        #[command(flatten)]
        formulas: Formulas,

        #[command(flatten)]
        landing: Landing,
    },

    /// Report whether the workbook recalculates fully when it is opened, or
    /// say that it should.
    ///
    /// With no flag this reads and writes nothing. With
    /// `--full-calc-on-load` it sets the flag, so that Excel works the
    /// workbook's values out again on the way in rather than trusting what
    /// the cache says.
    Calc {
        /// The package. Read unless the flag is given; then written in place
        /// unless `--out` is.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Flag the workbook to recalculate fully when it is next opened.
        #[arg(long)]
        full_calc_on_load: bool,

        #[command(flatten)]
        landing: Landing,
    },

    /// Read, write or take out the package's custom document properties.
    ///
    /// These are the named, typed values a package carries about itself,
    /// alongside the author and the title Excel fills in. A property is
    /// whatever a caller wants to stamp on a package: which template it came
    /// from, when it was filled in, which run produced it.
    Props {
        #[command(subcommand)]
        action: PropsAction,
    },

    /// Apply a batch of operations to a package, all of them or none.
    ///
    /// The batch is a JSON array of operations. It is validated whole before
    /// a byte is written and applied whole afterwards, so a failure anywhere
    /// leaves the package as it was, and the failure names the operation it
    /// came from by its place in the array.
    Apply {
        /// The package to write. Written in place unless `--out` is given.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// The batch: a path to a JSON array of operations, or `-` to read
        /// one from stdin.
        #[arg(value_name = "BATCH")]
        batch: PathBuf,

        #[command(flatten)]
        landing: Landing,
    },

    /// Print the version of xlsplice.
    Version,

    /// Crash, so the contract tests can reach the panic hook. Every other
    /// code in the frozen table is reachable from a real verb, so panicking is
    /// all this does. Debug builds only, hidden, and no part of the published
    /// contract.
    #[cfg(debug_assertions)]
    #[command(hide = true)]
    Selftest {
        /// Panic, to exercise the panic hook. Required, because there is
        /// nothing else here to ask for.
        #[arg(long, required = true)]
        panic: bool,
    },
}

/// What `props` does.
#[derive(Debug, Subcommand)]
pub enum PropsAction {
    /// List every custom document property with its type and its value, in
    /// the order the package holds them.
    Get {
        /// The package to read.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// Give a custom document property a value.
    ///
    /// A property of that name is written over, keeping the identifier it
    /// had; one that is not there is added. A package holding no custom
    /// properties at all gets the part they live in, declared as Excel
    /// declares it.
    Set {
        /// The package to write. Written in place unless `--out` is given.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// The property, by name. Matched exactly: two names differing in
        /// case are two properties.
        #[arg(value_name = "NAME")]
        name: String,

        /// The value, read according to `--type`.
        #[arg(value_name = "VALUE")]
        value: String,

        /// How to read VALUE and store it. A date is stored as a moment in
        /// UTC rather than as the serial a cell would hold.
        #[arg(long = "type", value_name = "TYPE", value_parser = write_type())]
        kind: WriteType,

        #[command(flatten)]
        landing: Landing,
    },

    /// Take a custom document property out. A property that is not there is
    /// not found.
    Unset {
        /// The package to write. Written in place unless `--out` is given.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// The property, by name.
        #[arg(value_name = "NAME")]
        name: String,

        #[command(flatten)]
        landing: Landing,
    },
}

/// What `--type` accepts: the write types the library offers, under the names
/// and descriptions it gives them, so a type outside them is clap's to refuse
/// and the help lists them without this module keeping its own copy of them.
fn write_type() -> impl TypedValueParser<Value = WriteType> {
    PossibleValuesParser::new(
        WriteType::ALL.map(|write_type| {
            PossibleValue::new(write_type.as_str()).help(write_type.description())
        }),
    )
    .map(|name| WriteType::named(&name).expect("clap offers only the names it was given"))
}

impl Command {
    /// The verb's name, for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Sheets { .. } => "sheets",
            Command::Names { .. } => "names",
            Command::Get { .. } => "get",
            Command::Set { .. } => "set",
            Command::Clear { .. } => "clear",
            Command::Calc { .. } => "calc",
            Command::Props { action } => match action {
                PropsAction::Get { .. } => "props get",
                PropsAction::Set { .. } => "props set",
                PropsAction::Unset { .. } => "props unset",
            },
            Command::Apply { .. } => "apply",
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
