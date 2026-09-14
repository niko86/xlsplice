//! Calling the library in process: what a verb is given, and what it wrote.
//!
//! Most suites live here rather than at the binary seam. A verb is called
//! directly and what comes back is put through `render`, which is the one
//! thing that decides the two streams and the exit code — so the whole machine
//! contract is observable without a process, and a test reads what a caller
//! would have read.

// Every suite compiles the whole of this and uses the part of it that suits
// what it is asking about, so what one suite does not reach is not dead.
#![allow(dead_code)]

use xlsplice::answer::Answer;
use xlsplice::render::{OutputMode, Rendered, render};

/// The operations a test batch is made of.
///
/// Not verbs: the verbs live in `xlsplice::verb`, where the binary calls
/// them too. These build the [`Operation`] values a batch carries, which is
/// test data rather than a second copy of anything — and each is one place, so
/// a field added to an operation stops the build here until it is considered.
pub mod op {
    use xlsplice::batch::{Operation, WriteType};

    /// One `set` operation.
    pub fn writing(target: &str, write_type: WriteType, value: &str) -> Operation {
        Operation::Set {
            target: target.to_owned(),
            write_type,
            value: value.to_owned(),
            replace_formula: false,
        }
    }

    /// One `calc` operation.
    pub fn calculating(full_calc_on_load: bool) -> Operation {
        Operation::Calc { full_calc_on_load }
    }

    /// One `props.set` operation.
    pub fn stamping(name: &str, write_type: WriteType, value: &str) -> Operation {
        Operation::PropsSet {
            name: name.to_owned(),
            write_type,
            value: value.to_owned(),
        }
    }

    /// One `props.unset` operation.
    pub fn unstamping(name: &str) -> Operation {
        Operation::PropsUnset {
            name: name.to_owned(),
        }
    }

    /// One `clear` operation.
    pub fn clearing(target: &str) -> Operation {
        Operation::Clear {
            target: target.to_owned(),
            replace_formula: false,
        }
    }

    /// The same operation, licensed to replace a formula it finds — what
    /// `--replace-formula` asks for, and what a batch says per operation.
    pub fn replacing(operation: Operation) -> Operation {
        match operation {
            Operation::Set {
                target,
                write_type,
                value,
                ..
            } => Operation::Set {
                target,
                write_type,
                value,
                replace_formula: true,
            },
            Operation::Clear { target, .. } => Operation::Clear {
                target,
                replace_formula: true,
            },
            other => other,
        }
    }
}

/// The targets a read verb is pointed at, as it wants them.
pub fn targets(list: &[&str]) -> Vec<String> {
    list.iter().map(|target| (*target).to_owned()).collect()
}

/// What a verb writes down a pipe under `--json`: one envelope on stdout, a
/// silent stderr, and the exit code.
pub fn under_json(outcome: xlsplice::Result<Answer>) -> Rendered {
    render(outcome, OutputMode::new(true, false))
}

/// What a verb writes down a pipe without `--json`: tab-separated fields on
/// stdout, or a failure on stderr.
pub fn in_text(outcome: xlsplice::Result<Answer>) -> Rendered {
    render(outcome, OutputMode::new(false, false))
}

/// The JSON envelope a verb put on stdout, parsed: what [`json`] gives for a
/// verb that went round through a process.
pub fn envelope(rendered: &Rendered) -> serde_json::Value {
    serde_json::from_str(&rendered.stdout)
        .unwrap_or_else(|err| panic!("stdout under --json must be one JSON document: {err}"))
}
