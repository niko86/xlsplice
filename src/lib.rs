//! Surgical edits to Excel packages.
//!
//! The library is where the work happens; the `xlsplice` binary in front of it
//! parses arguments, calls in here, and renders the result. What exists so far
//! is the contract every command speaks: the typed [`Error`] whose stable code
//! maps to a frozen exit code, and the JSON [`Envelope`] that is the one
//! document a `--json` invocation writes to stdout.
//!
//! See `CONTEXT.md` for the vocabulary, `docs/adr/` for the decisions.

pub mod envelope;
pub mod error;
pub mod package;
pub mod reference;
pub mod workbook;

pub use envelope::{Envelope, JsonStyle, SCHEMA_VERSION};
pub use error::{EXIT_SUCCESS, Error, ErrorCode, Result};
