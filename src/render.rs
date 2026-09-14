//! What a command puts on its two streams, and what it exits with.
//!
//! One function decides all of it, over the one thing a verb produces: an
//! [`Answer`] or an [`Error`]. Whether stdout carries an envelope or text,
//! whether rows are a table or tab-separated fields, what an answer with
//! nothing in it says, and which exit code a failure maps to are all decided
//! here, so that no verb decides any of them and none of it needs a process to
//! observe.
//!
//! What is left to the process is the process: probing whether stdout is a
//! terminal, writing the two strings, and exiting.

use crate::answer::{Answer, Shape};
use crate::envelope::{Envelope, JsonStyle};
use crate::error::{Error, Result};

/// What stdout carries, and in what shape. `--json` decides which arm; stdout
/// itself decides the style within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable text: an aligned table on a terminal, tab-separated
    /// fields in a pipe.
    Text(TextStyle),
    /// One JSON envelope, pretty on a terminal and compact in a pipe.
    Json(JsonStyle),
}

impl OutputMode {
    /// The mode for a run that did or did not ask for `--json`, on a stdout
    /// that is or is not a terminal. The probe belongs to the process, so the
    /// caller makes it once and passes the answer.
    pub fn new(json: bool, is_terminal: bool) -> Self {
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

/// Everything a command has to show for itself: the bytes for each stream,
/// ready to write as they are, and the code to exit with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The data channel. Under `--json` it is exactly one envelope; in text it
    /// is the answer, or nothing at all when the answer has nothing to say.
    pub stdout: String,
    /// The diagnostic channel: a failure in text mode, and nothing otherwise.
    pub stderr: String,
    /// The frozen exit code for the outcome.
    pub exit: u8,
}

/// Render what a verb came back with.
///
/// The two streams stay apart: under `--json` the envelope is the whole
/// response and stderr is silent, and in text mode a failure says nothing on
/// stdout, so a caller reading either stream reads one thing only.
///
/// A failure exits with its code from the frozen table; a success exits with
/// the one its answer carries, which is 0 for every verb but `diff` under
/// `--exit-code`.
pub fn render(outcome: Result<Answer>, mode: OutputMode) -> Rendered {
    match outcome {
        Ok(answer) => Rendered {
            stdout: match mode {
                OutputMode::Json(style) => {
                    terminated(&Envelope::success(answer.payload()).render(style))
                }
                OutputMode::Text(style) => terminated(&lay_out(style, answer.shape())),
            },
            stderr: String::new(),
            exit: answer.success_code(),
        },
        Err(error) => Rendered {
            stdout: match mode {
                OutputMode::Json(style) => terminated(&Envelope::failure(&error).render(style)),
                OutputMode::Text(_) => String::new(),
            },
            stderr: match mode {
                OutputMode::Json(_) => String::new(),
                OutputMode::Text(_) => diagnostic(&error),
            },
            exit: error.exit_code(),
        },
    }
}

/// What a command puts on its two streams when it panics, and what it exits
/// with.
///
/// A crash is a failure like any other as far as the contract goes: exit 1,
/// and under `--json` an `internal` envelope on stdout, because a caller
/// parsing stdout must be able to parse every outcome. It differs in one way,
/// and this is the one place that says so: the diagnostic goes to stderr in
/// *every* mode, where an ordinary failure under `--json` says nothing there.
/// A crash is a bug in xlsplice rather than something the caller did, and it
/// stays debuggable by a person watching the terminal.
///
/// `at` is where the panic was raised, as the file and line a panic hook has;
/// `what` is what it said. Nothing here is a process: the hook writes the two
/// strings and exits, and this can be asked what it would write without one.
pub fn crash(at: Option<(&str, u32)>, what: &str, mode: OutputMode) -> Rendered {
    let where_ = match at {
        Some((file, line)) => format!(" at {file}:{line}"),
        None => String::new(),
    };
    let error = Error::internal(format!(
        "internal error{where_}: {what}. This is a bug in xlsplice; \
         please report it with the command you ran."
    ));
    let on_stderr = diagnostic(&error);
    Rendered {
        stderr: on_stderr,
        ..render(Err(error), mode)
    }
}

/// What a failure says on stderr. A crash reports here too, whatever the mode,
/// so the prefix a reader looks for is written in one place.
fn diagnostic(error: &Error) -> String {
    format!("error: {error}\n")
}

/// What a stream carries for `text`: the text and its terminator, or nothing
/// at all. Nothing to say is said with nothing: a verb that found no rows must
/// not put a blank line down the pipe.
fn terminated(text: &str) -> String {
    match text.is_empty() {
        true => String::new(),
        false => format!("{text}\n"),
    }
}

/// Lay an answer out for a person.
///
/// Widths are counted in characters. That is exact for the Latin names these
/// tables carry and approximate for the rest, which costs nothing: alignment
/// is decoration, and no consumer reads the aligned form.
fn lay_out(style: TextStyle, shape: &Shape) -> String {
    let (headers, rows) = match shape {
        Shape::Line(line) => return line.clone(),
        Shape::Rows { headers, rows } => (headers, rows),
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    const HEADERS: [&str; 2] = ["NAME", "STATE"];

    fn rows() -> Vec<Vec<String>> {
        vec![
            vec!["Data".to_owned(), "visible".to_owned()],
            vec!["Parameters".to_owned(), "veryHidden".to_owned()],
        ]
    }

    /// A difference under `--exit-code` is a successful answer that exits 1.
    /// Both halves of that matter: the code tells a shell script what it
    /// asked to be told, and the envelope tells a program reading stdout that
    /// the command did what it was asked. The internal failure that shares
    /// the number is `ok: false`, which is how they are told apart.
    #[test]
    fn a_success_exits_with_the_code_its_answer_carries() {
        let answer = Answer::rows(serde_json::json!({"identical": false}), &HEADERS, rows())
            .expect("a payload and rows")
            .exiting(crate::error::EXIT_DIFFERENT);

        let rendered = render(Ok(answer), OutputMode::Json(JsonStyle::Compact));

        assert_eq!(rendered.exit, 1);
        assert!(
            rendered.stdout.contains(r#""ok":true"#),
            "{}",
            rendered.stdout
        );
        assert_eq!(rendered.stderr, "", "a success says nothing on stderr");
    }

    #[test]
    fn an_answer_that_was_given_no_code_exits_zero() {
        let answer = Answer::rows(serde_json::json!({"identical": true}), &HEADERS, rows())
            .expect("an answer");

        for mode in [
            OutputMode::Json(JsonStyle::Compact),
            OutputMode::Text(TextStyle::Tsv),
        ] {
            assert_eq!(render(Ok(answer.clone()), mode).exit, 0, "{mode:?}");
        }
    }

    /// The flag says nothing about failures. A package that cannot be read
    /// exits 5 whatever a success would have exited with, because what went
    /// wrong is what a caller has to hear first.
    /// The crash rule, both halves of it: the envelope so a program parsing
    /// stdout can parse every outcome, and the diagnostic on stderr *as well*,
    /// which is the one place the two channels deliberately both speak.
    #[test]
    fn a_crash_under_json_carries_the_envelope_and_still_reports_on_stderr() {
        let rendered = crash(
            Some(("src/worksheet.rs", 42)),
            "a row that is not there",
            OutputMode::Json(JsonStyle::Compact),
        );

        assert_eq!(rendered.exit, ErrorCode::Internal.exit_code());
        assert!(
            rendered.stdout.contains(r#""ok":false"#),
            "{}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains(r#""code":"internal""#),
            "{}",
            rendered.stdout
        );
        assert!(
            rendered.stderr.starts_with("error: "),
            "a crash stays debuggable on stderr even under --json: {}",
            rendered.stderr
        );
    }

    #[test]
    fn a_crash_in_text_mode_says_nothing_on_stdout() {
        let rendered = crash(None, "panicked", OutputMode::Text(TextStyle::Tsv));

        assert_eq!(rendered.exit, ErrorCode::Internal.exit_code());
        assert_eq!(rendered.stdout, "", "the data channel carries no failure");
        assert!(
            rendered.stderr.starts_with("error: "),
            "{}",
            rendered.stderr
        );
    }

    /// Where it happened is the first thing a report wants, and a panic
    /// without a location is still a crash rather than a blank one.
    #[test]
    fn a_crash_names_where_it_was_raised_when_it_knows() {
        let known = crash(
            Some(("src/splice.rs", 7)),
            "overlapping splices",
            OutputMode::Text(TextStyle::Tsv),
        );
        let unknown = crash(
            None,
            "overlapping splices",
            OutputMode::Text(TextStyle::Tsv),
        );

        assert!(known.stderr.contains("src/splice.rs:7"), "{}", known.stderr);
        assert!(known.stderr.contains("overlapping splices"));
        assert!(!unknown.stderr.contains(" at "), "{}", unknown.stderr);
        assert!(unknown.stderr.contains("overlapping splices"));
        for rendered in [&known, &unknown] {
            assert!(
                rendered.stderr.contains("This is a bug in xlsplice"),
                "a crash says whose fault it is: {}",
                rendered.stderr
            );
        }
    }

    #[test]
    fn a_failure_still_exits_with_its_own_code() {
        let rendered = render(
            Err(Error::unreadable("not a package")),
            OutputMode::Json(JsonStyle::Compact),
        );

        assert_eq!(rendered.exit, ErrorCode::Unreadable.exit_code());
    }

    /// An answer over `headers` and `rows`, with a payload that plays no part
    /// in what these tests are about.
    fn answer(headers: &'static [&'static str], rows: Vec<Vec<String>>) -> Answer {
        Answer::rows(serde_json::json!({"tested": true}), headers, rows)
            .expect("the test's rows must fit its headers")
    }

    /// What a person sees on stdout for those rows in that style.
    fn text(style: TextStyle, headers: &'static [&'static str], rows: Vec<Vec<String>>) -> String {
        let rendered = render(Ok(answer(headers, rows)), OutputMode::Text(style));

        assert_eq!(rendered.stderr, "", "success says nothing on stderr");
        assert_eq!(rendered.exit, 0);
        rendered.stdout
    }

    #[test]
    fn a_table_pads_every_column_to_its_widest_field_under_a_header() {
        assert_eq!(
            text(TextStyle::Table, &HEADERS, rows()),
            "NAME        STATE\n\
             Data        visible\n\
             Parameters  veryHidden\n"
        );
    }

    #[test]
    fn tab_separated_output_has_no_header_and_no_padding() {
        assert_eq!(
            text(TextStyle::Tsv, &HEADERS, rows()),
            "Data\tvisible\nParameters\tveryHidden\n"
        );
    }

    #[test]
    fn an_empty_field_still_holds_its_place_in_a_tab_separated_line() {
        const THREE: [&str; 3] = ["A", "B", "C"];
        let row = vec![vec![
            "Gone".to_owned(),
            String::new(),
            "ref_error".to_owned(),
        ]];

        assert_eq!(text(TextStyle::Tsv, &THREE, row), "Gone\t\tref_error\n");
    }

    #[test]
    fn no_rows_leaves_the_header_alone_on_a_terminal_and_says_nothing_at_all_in_a_pipe() {
        assert_eq!(
            text(TextStyle::Table, &HEADERS, Vec::new()),
            "NAME  STATE\n"
        );
        assert_eq!(
            text(TextStyle::Tsv, &HEADERS, Vec::new()),
            "",
            "not even a newline: nothing to say is said with nothing"
        );
    }

    #[test]
    fn no_table_line_ends_in_a_space() {
        for line in text(TextStyle::Table, &HEADERS, rows()).lines() {
            assert_eq!(line, line.trim_end(), "{line:?}");
        }
    }

    #[test]
    fn an_answer_that_is_a_sentence_is_the_same_sentence_in_both_styles() {
        let sentence = || {
            Answer::line(serde_json::json!({"version": "0.1.0"}), "xlsplice 0.1.0")
                .expect("a line is an answer")
        };

        for style in [TextStyle::Table, TextStyle::Tsv] {
            let rendered = render(Ok(sentence()), OutputMode::Text(style));
            assert_eq!(rendered.stdout, "xlsplice 0.1.0\n", "{style:?}");
        }
    }

    #[test]
    fn the_envelope_is_the_whole_response_and_stderr_stays_clean() {
        let rendered = render(
            Ok(answer(&HEADERS, rows())),
            OutputMode::Json(JsonStyle::Compact),
        );

        assert_eq!(
            rendered.stdout,
            "{\"ok\":true,\"schema_version\":1,\"tested\":true}\n"
        );
        assert_eq!(rendered.stderr, "");
        assert_eq!(rendered.exit, 0);
    }

    #[test]
    fn a_failure_in_text_leaves_the_data_channel_empty_and_says_so_on_stderr() {
        let rendered = render(
            Err(Error::not_found("no sheet named 'Inputs'")),
            OutputMode::Text(TextStyle::Tsv),
        );

        assert_eq!(rendered.stdout, "");
        assert_eq!(rendered.stderr, "error: no sheet named 'Inputs'\n");
    }

    #[test]
    fn a_failure_under_json_is_the_envelope_and_stderr_stays_clean() {
        let rendered = render(
            Err(Error::not_found("no sheet named 'Inputs'")),
            OutputMode::Json(JsonStyle::Compact),
        );

        assert_eq!(
            rendered.stdout,
            "{\"ok\":false,\"schema_version\":1,\
             \"error\":{\"code\":\"not_found\",\"message\":\"no sheet named 'Inputs'\"}}\n"
        );
        assert_eq!(rendered.stderr, "");
    }

    /// The frozen table, from the outcome a caller hands over to the code the
    /// process exits with, in both modes.
    #[test]
    fn every_error_code_maps_to_its_frozen_exit_code_in_either_mode() {
        for code in ErrorCode::ALL {
            for mode in [
                OutputMode::Text(TextStyle::Tsv),
                OutputMode::Json(JsonStyle::Compact),
            ] {
                let rendered = render(Err(Error::new(code, "no")), mode);
                assert_eq!(rendered.exit, code.exit_code(), "{code} in {mode:?}");
            }
        }
    }

    #[test]
    fn an_answer_exits_zero_whatever_the_mode() {
        for mode in [
            OutputMode::Text(TextStyle::Table),
            OutputMode::Json(JsonStyle::Pretty),
        ] {
            assert_eq!(render(Ok(answer(&HEADERS, rows())), mode).exit, 0);
        }
    }

    #[test]
    fn a_terminal_gets_a_table_and_a_pipe_gets_tab_separated_lines() {
        assert_eq!(TextStyle::for_terminal(true), TextStyle::Table);
        assert_eq!(TextStyle::for_terminal(false), TextStyle::Tsv);
    }

    #[test]
    fn json_is_asked_for_and_the_terminal_decides_the_style_within_it() {
        assert_eq!(
            OutputMode::new(true, false),
            OutputMode::Json(JsonStyle::Compact)
        );
        assert_eq!(
            OutputMode::new(true, true),
            OutputMode::Json(JsonStyle::Pretty)
        );
        assert_eq!(
            OutputMode::new(false, false),
            OutputMode::Text(TextStyle::Tsv)
        );
        assert_eq!(
            OutputMode::new(false, true),
            OutputMode::Text(TextStyle::Table)
        );
    }
}
