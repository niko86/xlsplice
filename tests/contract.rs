//! Contract tests: everything here observes the built binary from outside,
//! through its argv, its two streams and its exit code, and nothing else.

mod support;

use support::{exit_code, json, run, stderr, stdout};

/// The version declared in `Cargo.toml`, read from the manifest rather than
/// from the binary's own constant, so a hardcoded version would be caught.
fn manifest_version() -> String {
    let manifest = include_str!("../Cargo.toml");
    manifest
        .lines()
        .take_while(|line| !line.starts_with("[dependencies]"))
        .find_map(|line| line.strip_prefix("version = "))
        .expect("Cargo.toml must declare a package version")
        .trim_matches('"')
        .to_owned()
}

#[test]
fn version_prints_the_crate_version_as_text() {
    let out = run(&["version"]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(stdout(&out), format!("xlsplice {}\n", manifest_version()));
    assert_eq!(stderr(&out), "");
}

#[test]
fn version_json_prints_the_envelope() {
    let out = run(&["version", "--json"]);

    assert_eq!(exit_code(&out), 0);
    assert_eq!(stderr(&out), "");
    let body = json(&out);
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["schema_version"], serde_json::json!(1));
    assert_eq!(body["version"], serde_json::json!(manifest_version()));
    assert_eq!(body.get("error"), None);
}

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

/// The rest of the table needs verbs that do not exist yet, so it is reached
/// through the `selftest` stub, which is compiled only into a debug build.
/// A release build has no stub, and so skips these.
#[cfg(debug_assertions)]
mod through_the_stub {
    use super::*;

    /// The frozen table, transcribed from the spec.
    const TABLE: [(&str, &str, i32); 5] = [
        ("internal", "internal", 1),
        ("usage", "usage", 2),
        ("not-found", "not_found", 3),
        ("refused", "refused", 4),
        ("unreadable", "unreadable", 5),
    ];

    #[test]
    fn a_command_that_succeeds_exits_zero() {
        let out = run(&["selftest"]);

        assert_eq!(exit_code(&out), 0);
        assert_eq!(stderr(&out), "");
    }

    #[test]
    fn every_code_in_the_table_has_a_case_that_produces_it_with_the_envelope() {
        for (kind, code, expected_exit) in TABLE {
            let out = run(&["selftest", "--fail", kind, "--json"]);

            assert_eq!(exit_code(&out), expected_exit, "{kind}");
            let body = json(&out);
            assert_eq!(body["ok"], serde_json::json!(false), "{kind}");
            assert_eq!(body["schema_version"], serde_json::json!(1), "{kind}");
            assert_eq!(body["error"]["code"], serde_json::json!(code), "{kind}");
            assert!(
                !body["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .is_empty(),
                "{kind} must carry a message"
            );
        }
    }

    #[test]
    fn every_code_in_the_table_reports_on_stderr_without_json() {
        for (kind, _, expected_exit) in TABLE {
            let out = run(&["selftest", "--fail", kind]);

            assert_eq!(exit_code(&out), expected_exit, "{kind}");
            assert_eq!(stdout(&out), "", "{kind} must leave the data channel empty");
            assert!(
                stderr(&out).starts_with("error: "),
                "{kind}: {}",
                stderr(&out)
            );
        }
    }

    /// The stub names codes the way the envelope does, so a code the library
    /// does not publish is a usage error rather than a silent pass.
    #[test]
    fn the_stub_refuses_a_code_that_is_not_in_the_table() {
        assert_eq!(exit_code(&run(&["selftest", "--fail", "wat"])), 2);
    }

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
