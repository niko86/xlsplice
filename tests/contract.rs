//! Contract tests: everything here observes the built binary from outside,
//! through its argv, its two streams and its exit code, and nothing else.
//!
//! What is here is what needs a process to observe: what clap does before a
//! verb is reached, the argv scan that decides the shape of a usage error
//! before clap can, which stream a real run writes to at each volume, and the
//! panic hook. The shape of what a verb answers is not here: it is asserted
//! in process in `answers.rs`, and the layout and the exit codes with it, at
//! the library seam.
//!
//! Every code in the frozen table is still reached out of a real process.
//! Only 1 needs the stub below, and only for a crash: `set.rs` reaches it for
//! real, along with 2, 3 and 4; a usage error here reaches 2; `get.rs`
//! reaches 3 and 4; and any verb on a path that is not a package reaches 5,
//! in `get.rs` and `read_verbs.rs`.

mod support;

use support::{exit_code, json, run, stderr, stdout};

#[test]
fn an_unknown_flag_is_a_usage_error_in_text_on_stderr() {
    let out = run(&["version", "--nope"]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(
        stdout(&out),
        "",
        "a text-mode failure must leave stdout empty"
    );
    assert!(stderr(&out).contains("--nope"), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("Usage:"),
        "the usage line must be offered"
    );
}

#[test]
fn an_unknown_flag_with_json_is_a_usage_error_in_the_envelope() {
    let out = run(&["version", "--nope", "--json"]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(
        stderr(&out),
        "",
        "under --json the envelope is the whole response"
    );
    let body = json(&out);
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["schema_version"], serde_json::json!(1));
    assert_eq!(body["error"]["code"], serde_json::json!("usage"));
    let message = body["error"]["message"]
        .as_str()
        .expect("a message is required");
    assert!(
        message.contains("--nope"),
        "the message must name the offending flag"
    );
    assert!(
        message.contains("Usage:"),
        "the message must say what to run instead"
    );
}

#[test]
fn a_missing_subcommand_is_a_usage_error() {
    assert_eq!(exit_code(&run(&[])), 2);
    assert_eq!(
        json(&run(&["--json"]))["error"]["code"],
        serde_json::json!("usage")
    );
}

/// clap lexes `--json=x` as the `--json` flag before rejecting the value, so
/// the scan that decides the shape of a usage error must see it the same way.
#[test]
fn a_value_glued_to_json_still_selects_the_envelope() {
    let out = run(&["version", "--json=true"]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(json(&out)["error"]["code"], serde_json::json!("usage"));
}

#[test]
fn json_after_the_double_dash_is_an_operand_not_a_flag() {
    let out = run(&["version", "--", "--json"]);

    assert_eq!(exit_code(&out), 2);
    assert_eq!(
        stdout(&out),
        "",
        "`--` demoted --json to an operand, so no envelope"
    );
    assert!(!stderr(&out).is_empty());
}

#[test]
fn help_and_version_flags_stay_text_and_exit_zero() {
    for args in [["--help"], ["--version"]] {
        let out = run(&args);
        assert_eq!(exit_code(&out), 0, "{args:?}");
        assert!(!stdout(&out).is_empty(), "{args:?} prints on stdout");
        assert_eq!(stderr(&out), "", "{args:?}");
    }
}

#[test]
fn the_selftest_stub_is_never_advertised() {
    assert!(!stdout(&run(&["--help"])).contains("selftest"));
}

/// The other half of this rule, pretty-printing on a terminal, is held at the
/// library seam by `JsonStyle::for_terminal`: a spawned process has no
/// terminal to give it.
#[test]
fn json_is_compact_when_stdout_is_a_pipe() {
    let body = stdout(&run(&["version", "--json"]));

    assert_eq!(
        body.trim_end().lines().count(),
        1,
        "compact JSON is one line"
    );
    assert!(
        !body.contains(": "),
        "compact JSON has no spaces after its colons"
    );
}

#[test]
fn verbose_adds_diagnostics_on_stderr_and_leaves_stdout_alone() {
    let plain = run(&["version"]);
    let loud = run(&["version", "--verbose"]);

    assert_eq!(exit_code(&loud), 0);
    assert_eq!(
        stdout(&loud),
        stdout(&plain),
        "stdout is not a diagnostic channel"
    );
    assert!(
        !stderr(&loud).is_empty(),
        "--verbose must say something on stderr"
    );
}

#[test]
fn verbose_does_not_disturb_the_envelope() {
    let out = run(&["version", "--json", "--verbose"]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(
        json(&out)["ok"],
        serde_json::json!(true),
        "stdout is still one document"
    );
    assert!(!stderr(&out).is_empty());
}

#[test]
fn quiet_leaves_stderr_silent_and_stdout_unchanged() {
    let plain = run(&["version"]);
    let hushed = run(&["version", "--quiet"]);

    assert_eq!(exit_code(&hushed), 0);
    assert_eq!(stdout(&hushed), stdout(&plain));
    assert_eq!(stderr(&hushed), "");
}

#[test]
fn quiet_never_silences_an_error() {
    let out = run(&["version", "--nope", "--quiet"]);

    assert_eq!(exit_code(&out), 2);
    assert!(
        stderr(&out).starts_with("error: "),
        "a failure must stay visible"
    );
}

#[test]
fn quiet_and_verbose_cannot_both_be_asked_for() {
    assert_eq!(exit_code(&run(&["version", "--quiet", "--verbose"])), 2);
}

/// The panic hook is the one thing left that no verb can reach, so a stub
/// reaches it. The stub is compiled only into a debug build; a release build
/// has none, and so skips these.
#[cfg(debug_assertions)]
mod through_the_stub {
    use super::*;

    #[test]
    fn a_panic_becomes_the_internal_envelope_and_exit_one() {
        let out = run(&["selftest", "--panic", "--json"]);

        assert_eq!(exit_code(&out), 1);
        let body = json(&out);
        assert_eq!(body["ok"], serde_json::json!(false));
        assert_eq!(body["error"]["code"], serde_json::json!("internal"));
        assert!(
            !stderr(&out).is_empty(),
            "a crash must stay debuggable on stderr even under --json"
        );
    }

    #[test]
    fn a_panic_without_json_reports_on_stderr_and_exits_one() {
        let out = run(&["selftest", "--panic"]);

        assert_eq!(exit_code(&out), 1);
        assert_eq!(stdout(&out), "");
        assert!(stderr(&out).contains("error: "), "stderr: {}", stderr(&out));
    }
}
