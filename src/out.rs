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
    /// Human-readable text: an aligned table on a terminal, tab-separated
    /// fields in a pipe.
    Text(TextStyle),
    /// One JSON envelope, pretty on a terminal and compact in a pipe.
    Json(JsonStyle),
}

impl OutputMode {
    /// The mode for this run, given whether `--json` was asked for. Whether
    /// stdout is a terminal is asked once, here, and not rediscovered at each
    /// write.
    pub fn resolve(json: bool) -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        if json {
            OutputMode::Json(JsonStyle::for_terminal(is_terminal))
        } else {
            OutputMode::Text(TextStyle::for_terminal(is_terminal))
        }
    }
}

/// How rows are laid out for eyes or for `cut`.
///
/// A terminal gets an aligned table under a header. A pipe gets tab-separated
/// fields and no header, so a field is always the same field and nothing is
/// truncated or implied by layout. Human output is not the stable interface:
/// `--json` is, and a script should use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStyle {
    /// Aligned columns under a header.
    Table,
    /// Tab-separated fields, no header.
    Tsv,
}

impl TextStyle {
    /// A table only on a terminal; a pipe gets tab-separated lines.
    pub fn for_terminal(is_terminal: bool) -> Self {
        if is_terminal {
            TextStyle::Table
        } else {
            TextStyle::Tsv
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
            // Nothing to say is said with nothing: a verb that found no rows
            // must not put a blank line down the pipe.
            OutputMode::Text(_) if text.is_empty() => {}
            OutputMode::Text(_) => println!("{text}"),
        }
        ExitCode::from(EXIT_SUCCESS)
    }

    /// Report success as rows: the envelope under `--json`, otherwise the
    /// rows in whichever text style stdout asked for.
    pub fn rows<T: Serialize>(
        &self,
        payload: T,
        headers: &[&str],
        rows: &[Vec<String>],
    ) -> ExitCode {
        let text = match self.mode {
            OutputMode::Text(style) => render_rows(style, headers, rows),
            OutputMode::Json(_) => String::new(),
        };
        self.success(payload, &text)
    }

    /// Report a failure and give back the exit code it maps to. Under `--json`
    /// the envelope is the whole response and stderr stays clean.
    pub fn failure(&self, error: &Error) -> ExitCode {
        match self.mode {
            OutputMode::Json(style) => println!("{}", Envelope::failure(error).render(style)),
            OutputMode::Text(_) => eprintln!("error: {error}"),
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

/// Lay `rows` out in `style`.
///
/// Widths are counted in characters. That is exact for the Latin names these
/// tables carry and approximate for the rest, which costs nothing: alignment
/// is decoration, and no consumer reads the aligned form.
fn render_rows(style: TextStyle, headers: &[&str], rows: &[Vec<String>]) -> String {
    let lines: Vec<String> = match style {
        TextStyle::Tsv => rows.iter().map(|row| row.join("\t")).collect(),
        TextStyle::Table => {
            let mut widths: Vec<usize> = headers.iter().map(|head| head.chars().count()).collect();
            for row in rows {
                for (column, field) in row.iter().enumerate() {
                    let width = field.chars().count();
                    if let Some(current) = widths.get_mut(column)
                        && width > *current
                    {
                        *current = width;
                    }
                }
            }
            let header: Vec<String> = headers.iter().map(|head| (*head).to_owned()).collect();
            std::iter::once(&header)
                .chain(rows.iter())
                .map(|row| pad(row, &widths))
                .collect()
        }
    };
    lines.join("\n")
}

/// One line of a table: two spaces between columns, every field but the last
/// padded to its column's width, and no trailing space.
fn pad(row: &[String], widths: &[usize]) -> String {
    let mut line = String::new();
    for (column, field) in row.iter().enumerate() {
        if column > 0 {
            line.push_str("  ");
        }
        line.push_str(field);
        let width = widths.get(column).copied().unwrap_or(0);
        line.push_str(&" ".repeat(width.saturating_sub(field.chars().count())));
    }
    line.trim_end().to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Vec<String>> {
        vec![
            vec!["Data".to_owned(), "visible".to_owned()],
            vec!["Parameters".to_owned(), "veryHidden".to_owned()],
        ]
    }

    const HEADERS: [&str; 2] = ["NAME", "STATE"];

    #[test]
    fn a_table_pads_every_column_to_its_widest_field_under_a_header() {
        assert_eq!(
            render_rows(TextStyle::Table, &HEADERS, &rows()),
            "NAME        STATE\n\
             Data        visible\n\
             Parameters  veryHidden"
        );
    }

    #[test]
    fn tab_separated_output_has_no_header_and_no_padding() {
        assert_eq!(
            render_rows(TextStyle::Tsv, &HEADERS, &rows()),
            "Data\tvisible\nParameters\tveryHidden"
        );
    }

    #[test]
    fn an_empty_field_still_holds_its_place_in_a_tab_separated_line() {
        let row = vec![vec![
            "Gone".to_owned(),
            String::new(),
            "ref_error".to_owned(),
        ]];

        assert_eq!(
            render_rows(TextStyle::Tsv, &["A", "B", "C"], &row),
            "Gone\t\tref_error"
        );
    }

    #[test]
    fn no_rows_leaves_the_header_alone_on_a_terminal_and_says_nothing_in_a_pipe() {
        assert_eq!(render_rows(TextStyle::Table, &HEADERS, &[]), "NAME  STATE");
        assert_eq!(render_rows(TextStyle::Tsv, &HEADERS, &[]), "");
    }

    #[test]
    fn no_table_line_ends_in_a_space() {
        for line in render_rows(TextStyle::Table, &HEADERS, &rows()).lines() {
            assert_eq!(line, line.trim_end(), "{line:?}");
        }
    }

    #[test]
    fn a_terminal_gets_a_table_and_a_pipe_gets_tab_separated_lines() {
        assert_eq!(TextStyle::for_terminal(true), TextStyle::Table);
        assert_eq!(TextStyle::for_terminal(false), TextStyle::Tsv);
    }
}
