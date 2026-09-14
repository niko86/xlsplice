//! The help topics: the parts of the contract too long to sit in a flag's
//! description.
//!
//! `--help` says what a verb takes. These say what everything that comes back
//! means, which is what an agent reading the tool for the first time needs and
//! what no single verb's help has room for.
//!
//! Both topics are built from the library rather than transcribed beside it.
//! The exit-code table comes from [`ErrorCode`] and the batch's operation
//! kinds from [`Kind::ALL`], so a topic cannot fall behind the thing it
//! describes: adding a code or a kind changes the topic in the same commit.

use crate::batch::{Kind, WriteType};
use crate::error::{EXIT_DIFFERENT, EXIT_SUCCESS, ErrorCode};

/// A subject the `help` verb can explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    /// The JSON envelope: what every `--json` answer carries, what may change
    /// about it, and the batch document `apply` reads.
    Json,
    /// The frozen exit-code table.
    ExitCodes,
}

impl Topic {
    /// Every topic, in the order they are offered.
    pub const ALL: [Topic; 2] = [Topic::Json, Topic::ExitCodes];

    /// The topic as it is asked for on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Topic::Json => "json",
            Topic::ExitCodes => "exit-codes",
        }
    }

    /// One line saying what the topic covers, for the verb's own help.
    pub fn description(self) -> &'static str {
        match self {
            Topic::Json => "The JSON envelope, what may change about it, and the batch document",
            Topic::ExitCodes => "The frozen exit-code table",
        }
    }

    /// The topic asked for by name, or `None` for a name there is no topic
    /// for.
    pub fn named(name: &str) -> Option<Topic> {
        Topic::ALL.into_iter().find(|topic| topic.as_str() == name)
    }

    /// The whole of what the topic says.
    pub fn text(self) -> String {
        match self {
            Topic::Json => json(),
            Topic::ExitCodes => exit_codes(),
        }
    }
}

/// The envelope, the policy that governs it, and the batch document.
fn json() -> String {
    let kinds = Kind::ALL.map(|kind| format!("  {kind}")).join("\n");
    let types = WriteType::ALL
        .map(|write_type| format!("  {:<7} {}", write_type.as_str(), write_type.description()))
        .join("\n");
    format!(
        "THE JSON ENVELOPE

Every command given --json writes exactly one JSON document to stdout and
nothing else. Nothing else is ever written to stdout under --json: diagnostics
go to stderr, and a failure goes into the envelope rather than beside it.

An envelope that succeeded:

  {{\"ok\":true,\"schema_version\":1, ...the verb's own fields... }}

An envelope that failed:

  {{\"ok\":false,\"schema_version\":1,\"error\":{{\"code\":\"not_found\",\"message\":\"...\"}}}}

  ok              true or false. Read this, not the presence of a field.
  schema_version  1. Bumped only if a field is removed or retyped.
  error           Present only on failure. Never present on success.

WHAT MAY CHANGE

The envelope is the stable interface; the human-readable table is not, and may
be laid out differently from one version to the next. Changes to the envelope
are additive, so a consumer must:

  - ignore a field it does not recognise;
  - ignore an error.code it does not recognise, and fall back on the exit code;
  - not depend on the order of an object's fields.

A field that does not apply is null rather than absent, so every member of a
list carries the keys the others carry.

THE BATCH DOCUMENT

`apply` reads a JSON array of operations, from a file or from stdin as `-`. It
is validated whole before a byte is written and applied whole afterwards, so a
failure anywhere leaves the package as it was, and the failure names the
operation it came from by its place in the array.

  [
    {{\"op\": \"set\", \"target\": \"Inputs!A1\", \"type\": \"number\", \"value\": \"42\"}},
    {{\"op\": \"clear\", \"target\": \"Inputs!B1\"}},
    {{\"op\": \"props.set\", \"name\": \"Run.At\", \"type\": \"date\", \"value\": \"2026-09-13\"}},
    {{\"op\": \"props.unset\", \"name\": \"Draft\"}},
    {{\"op\": \"calc\", \"full_calc_on_load\": true}}
  ]

The operation kinds:

{kinds}

Every value is given as text, under the type that says how to read it, so that
the batch carries what the caller wrote rather than what JSON made of it. The
types:

{types}

A `set` or a `clear` takes an optional \"replace_formula\": true, which licenses
dropping a formula the target cell holds. Without it a cell holding one is
refused, so a mis-addressed write cannot silently destroy a template's formula.

A batch is a set of edits over the package as it was read, not a sequence over
a document that changes under it. So one cell cannot be named twice, and
neither can one document property: two operations on one thing are two answers
to one question rather than a first write and a second, and the batch is
refused whole with exit {usage}.

A batch that would change nothing writes nothing at all, and reports every
operation as \"changed\": false.
",
        usage = ErrorCode::Usage.exit_code(),
    )
}

/// The frozen table, built from the codes themselves.
fn exit_codes() -> String {
    let rows = ErrorCode::ALL
        .map(|code| {
            format!(
                "  {:<5}{:<13}{}",
                code.exit_code(),
                code.as_str(),
                meaning(code)
            )
        })
        .join("\n");
    format!(
        "EXIT CODES

  {EXIT_SUCCESS:<5}{:<13}The command succeeded.
{rows}

The `error.code` in the JSON envelope is the name in the second column, so a
caller may read either.

Any non-zero code guarantees that no package was written. A command that was
going to write one writes to a temporary file and renames it, so a command
that was killed leaves the package it was pointed at exactly as it was.

`diff --exit-code` exits {EXIT_DIFFERENT} where the two packages differ, which is what
diff(1) does. That is not an error: the command succeeded and answered, and
the envelope says so with \"ok\": true. It shares its number with `internal`,
and nothing else about the two is alike.

The table is frozen. Codes may be added, never removed and never renumbered,
so a consumer must tolerate a code it does not recognise: treat any non-zero
code it does not know as a failure, and read `error.message` for what went
wrong.
",
        "",
    )
}

/// What each code means, in the one sentence the table has room for.
fn meaning(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Internal => "An unexpected failure, including a caught panic.",
        ErrorCode::Usage => "The command line or the batch was wrong.",
        ErrorCode::NotFound => "A sheet, defined name, cell, property or part was not found.",
        ErrorCode::Refused => "A guard said no; some are licensed by a flag.",
        ErrorCode::Unreadable => "Not a package, or the package cannot be read.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_is_asked_for_by_the_name_it_answers_to() {
        for topic in Topic::ALL {
            assert_eq!(Topic::named(topic.as_str()), Some(topic));
        }
        assert_eq!(Topic::named("envelope"), None);
    }

    /// The table is built from the codes, so this is the assertion that it
    /// was built from all of them and spells each one right.
    #[test]
    fn the_exit_code_table_carries_every_code_with_its_number() {
        let text = Topic::ExitCodes.text();

        for code in ErrorCode::ALL {
            let row = format!("{:<5}{}", code.exit_code(), code.as_str());
            assert!(text.contains(&row), "the table must carry '{row}': {text}");
        }
        assert!(text.contains("0    "), "success has a row of its own");
    }

    #[test]
    fn the_json_topic_carries_every_operation_kind_and_write_type() {
        let text = Topic::Json.text();

        for kind in Kind::ALL {
            assert!(
                text.contains(kind.as_str()),
                "the batch schema must name '{kind}'"
            );
        }
        for write_type in WriteType::ALL {
            assert!(
                text.contains(write_type.as_str()),
                "the batch schema must name '{}'",
                write_type.as_str()
            );
        }
    }

    #[test]
    fn a_topic_is_more_than_one_line_and_ends_in_one_newline() {
        for topic in Topic::ALL {
            let text = topic.text();
            assert!(text.lines().count() > 10, "{}", topic.as_str());
            assert!(text.ends_with('\n'), "{}", topic.as_str());
            assert!(!text.ends_with("\n\n"), "{}", topic.as_str());
        }
    }
}
