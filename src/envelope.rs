//! The JSON envelope: the one document a `--json` invocation writes to stdout.
//!
//! Every `--json` response is an object whose first two keys are `ok` and
//! `schema_version`. On success the verb's payload follows, flattened into the
//! same object; on failure an `error` object holds the stable snake-case
//! [`ErrorCode`] and a message that names the fix. Nothing else reaches
//! stdout, and the exit code is unchanged by `--json`.
//!
//! The schema is versioned and changes only additively: fields and enum values
//! may be added, never removed or retyped, so a consumer must ignore fields
//! and error codes it does not recognise.

use serde::Serialize;

use crate::error::Error;

/// The schema version stamped into every envelope.
pub const SCHEMA_VERSION: u32 = 1;

/// How the envelope is rendered: compact for machines, pretty for eyes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonStyle {
    /// One line, no spaces. Used when stdout is not a terminal.
    Compact,
    /// Indented across several lines. Used when stdout is a terminal.
    Pretty,
}

impl JsonStyle {
    /// Pretty-print only when stdout is a terminal; a pipe gets compact JSON.
    pub fn for_terminal(is_terminal: bool) -> Self {
        if is_terminal {
            JsonStyle::Pretty
        } else {
            JsonStyle::Compact
        }
    }
}

/// The body of `error` in a failed envelope.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    /// The stable snake-case code; see [`ErrorCode`](crate::error::ErrorCode).
    pub code: &'static str,
    /// Human-readable text that names the fix.
    pub message: String,
}

/// One `--json` response.
#[derive(Debug, Serialize)]
pub struct Envelope<T: Serialize> {
    ok: bool,
    schema_version: u32,
    #[serde(flatten)]
    payload: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorBody>,
}

/// The payload of an envelope that carries nothing but the outcome.
#[derive(Debug, Serialize)]
pub struct NoPayload {}

impl<T: Serialize> Envelope<T> {
    /// An `ok: true` envelope carrying a verb's payload.
    pub fn success(payload: T) -> Self {
        Envelope {
            ok: true,
            schema_version: SCHEMA_VERSION,
            payload: Some(payload),
            error: None,
        }
    }

    /// Render the envelope as the single JSON document for stdout.
    ///
    /// # Panics
    ///
    /// If the payload does not serialise to a JSON object. The binary's panic
    /// hook turns that into an `internal` envelope, so even this failure mode
    /// stays parseable.
    pub fn render(&self, style: JsonStyle) -> String {
        let rendered = match style {
            JsonStyle::Compact => serde_json::to_string(self),
            JsonStyle::Pretty => serde_json::to_string_pretty(self),
        };
        rendered.expect("an envelope payload must serialise to a JSON object")
    }
}

impl Envelope<NoPayload> {
    /// An `ok: false` envelope carrying the error's code and message.
    pub fn failure(error: &Error) -> Self {
        Envelope {
            ok: false,
            schema_version: SCHEMA_VERSION,
            payload: None,
            error: Some(ErrorBody {
                code: error.code().as_str(),
                message: error.message().to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use serde::Serialize;

    #[derive(Serialize)]
    struct VersionPayload {
        version: &'static str,
    }

    #[test]
    fn a_success_envelope_leads_with_ok_and_the_schema_version() {
        let rendered =
            Envelope::success(VersionPayload { version: "0.1.0" }).render(JsonStyle::Compact);
        assert_eq!(
            rendered,
            r#"{"ok":true,"schema_version":1,"version":"0.1.0"}"#
        );
    }

    #[test]
    fn a_failure_envelope_carries_the_stable_code_and_the_message() {
        let err = Error::usage("unexpected argument '--nope'; run 'xlsplice --help'");
        let rendered = Envelope::failure(&err).render(JsonStyle::Compact);
        assert_eq!(
            rendered,
            r#"{"ok":false,"schema_version":1,"error":{"code":"usage","message":"unexpected argument '--nope'; run 'xlsplice --help'"}}"#
        );
    }

    #[test]
    fn a_success_envelope_has_no_error_key_and_a_failure_no_payload() {
        let ok = Envelope::success(VersionPayload { version: "0.1.0" }).render(JsonStyle::Compact);
        assert!(!ok.contains(r#""error""#));
        let bad = Envelope::failure(&Error::internal("boom")).render(JsonStyle::Compact);
        assert!(!bad.contains(r#""version""#));
    }

    #[test]
    fn compact_is_one_line_and_pretty_is_indented() {
        let envelope = Envelope::success(VersionPayload { version: "0.1.0" });
        assert!(!envelope.render(JsonStyle::Compact).contains('\n'));
        let pretty = envelope.render(JsonStyle::Pretty);
        assert!(pretty.contains("\n  \"ok\": true"));
    }

    #[test]
    fn json_is_pretty_on_a_terminal_and_compact_when_piped() {
        assert_eq!(JsonStyle::for_terminal(true), JsonStyle::Pretty);
        assert_eq!(JsonStyle::for_terminal(false), JsonStyle::Compact);
    }
}
