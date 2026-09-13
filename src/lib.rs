//! Surgical edits to Excel packages.
//!
//! The library is where the work happens; the `xlsplice` binary in front of it
//! parses arguments, calls in here, and writes what comes back.
//!
//! Underneath everything is the contract every command speaks: the typed
//! [`Error`] whose stable code maps to a frozen exit code, and the JSON
//! [`Envelope`] that is the one document a `--json` invocation writes to
//! stdout. At the top of it, a verb comes back with an [`Answer`], and
//! [`render`] turns that into the bytes of the two streams and the code to
//! exit with, so the whole contract can be observed without a process. Between
//! the two sit the reading layers: a [`package`] of parts joined by
//! [`relationships`], the [`workbook`] model over one of them, the
//! [`worksheet`] and [`strings`] parts a cell's content is spread across, and
//! [`cells`], where an operand becomes an answer. Both paths point at a cell
//! through [`target`], which is where an operand becomes a part and a cell.
//! The write path runs the other way: a [`batch`] of operations answers with
//! the edits it wants made to each part, and the package is rebuilt around
//! them.
//!
//! See `CONTEXT.md` for the vocabulary, `docs/adr/` for the decisions.

pub mod answer;
pub mod atomic;
pub mod batch;
pub mod calc_chain;
pub mod calculation;
pub mod cells;
pub mod date;
pub mod declared;
pub mod envelope;
pub mod error;
pub mod package;
pub mod properties;
pub mod reference;
pub mod relationships;
pub mod render;
pub mod splice;
pub mod strings;
pub mod target;
pub mod workbook;
pub mod worksheet;
pub mod xml;

pub use answer::Answer;
pub use envelope::{Envelope, JsonStyle, SCHEMA_VERSION};
pub use error::{EXIT_SUCCESS, Error, ErrorCode, Result};
pub use render::{OutputMode, Rendered, TextStyle, render};
