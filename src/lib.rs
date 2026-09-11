//! Surgical edits to Excel packages.
//!
//! The library is where the work happens; the `xlsplice` binary in front of it
//! parses arguments, calls in here, and renders the result.
//!
//! Underneath everything is the contract every command speaks: the typed
//! [`Error`] whose stable code maps to a frozen exit code, and the JSON
//! [`Envelope`] that is the one document a `--json` invocation writes to
//! stdout. Over it sit the reading layers: a [`package`] of parts joined by
//! [`relationships`], the [`workbook`] model over one of them, the
//! [`worksheet`] and [`strings`] parts a cell's content is spread across, and
//! [`cells`], where an operand becomes an answer.
//!
//! See `CONTEXT.md` for the vocabulary, `docs/adr/` for the decisions.

pub mod cells;
pub mod envelope;
pub mod error;
pub mod package;
pub mod reference;
pub mod relationships;
pub mod strings;
pub mod workbook;
pub mod worksheet;
pub mod xml;

pub use envelope::{Envelope, JsonStyle, SCHEMA_VERSION};
pub use error::{EXIT_SUCCESS, Error, ErrorCode, Result};
