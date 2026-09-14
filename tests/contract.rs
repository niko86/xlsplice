//! Contract tests: everything here observes the built binary from outside,
//! through its argv, its two streams and its exit code, and nothing else.
//!
//! What is here is what needs a process to observe: what clap does before a
//! verb is reached, the argv scan that decides the shape of a usage error
//! before clap can, which stream a real run writes to at each volume, and the
//! panic hook. The shape of what a verb answers is not here: it is asserted
//! in process, beside the verb that answers it, and the layout with it.
//!
//! Three of the frozen table's codes come out of a real process here: 0
//! wherever a run succeeds, 2 from a usage error, and 1 from the crash probe
//! below — which is a process but not `xlsplice`, for the reason its own
//! comment gives. The other three are raised by verbs that no longer spawn to
//! be watched, so what holds them is `render`'s own test over the whole table,
//! and the one line of `out.rs` that turns a rendered code into an exit
//! status is crossed here by the three that remain.

mod support;

use support::binary::{crash, exit_code, json, run, stderr, stdout};

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

/// `--quiet` asks for the default, which is silence: a run that was not asked
/// to be loud says nothing on stderr, so the flag has nothing left to take
/// away. What is asserted here is the default's silence, which is the property
/// `--quiet` names; the flag riding along says only that it is still accepted.
#[test]
fn the_default_is_silent_and_quiet_asks_for_the_default() {
    let plain = run(&["version"]);
    let hushed = run(&["version", "--quiet"]);

    assert_eq!(exit_code(&hushed), 0);
    assert_eq!(stderr(&plain), "", "the default says nothing on stderr");
    assert_eq!(stdout(&hushed), stdout(&plain));
    assert_eq!(stderr(&hushed), "");
}

/// And the help says so, rather than leaving a caller to find out by running
/// the tool twice and comparing. A published flag that does nothing is a
/// promise to keep accepting it, not a promise that it does something.
#[test]
fn the_help_says_quiet_does_nothing() {
    let help = stdout(&run(&["--help"]));

    let quiet = help
        .lines()
        .find(|line| line.contains("--quiet"))
        .unwrap_or_else(|| panic!("the help lists --quiet: {help}"));
    assert!(
        quiet.contains("does nothing"),
        "the help must say --quiet does nothing: {quiet}"
    );
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

/// A panic is the one outcome no verb can produce on purpose. What it *says*
/// is asserted at the library seam, where `render::crash` can be asked what it
/// would write without a process. What needs a process is the rest of the
/// hook — that the two strings reach the two streams and that the exit status
/// is 1 — and the three tests below are that, run against the crash probe.
///
/// **The probe is not `xlsplice`.** There was a hidden `selftest --panic` verb
/// here until #38: a command on the published surface whose only purpose was
/// to be crashed by these tests. It is not coming back, and that has a
/// consequence worth saying rather than leaving to be rediscovered — making
/// the real binary panic on demand *requires* a way to ask it to, so no test
/// can watch the published binary crash without putting the trapdoor back.
/// What is held here is one step short of that: the real hook, in a real
/// process, doing what only a process can be watched doing. That `main.rs`
/// installs it, and installs it before clap can fail, is what the last test
/// reads the source for — the seam between the two halves.
#[test]
fn a_crash_under_json_puts_the_envelope_on_stdout_and_still_reports_on_stderr() {
    let out = crash(&["--json"]);

    assert_eq!(
        exit_code(&out),
        1,
        "a crash exits 1: stderr: {}",
        stderr(&out)
    );

    let body = json(&out);
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["schema_version"], serde_json::json!(1));
    assert_eq!(body["error"]["code"], serde_json::json!("internal"));

    let message = body["error"]["message"]
        .as_str()
        .expect("a message is required");
    assert!(
        message.contains("crash probe"),
        "the panic's own words must survive into the envelope: {message}"
    );
    assert!(
        message.contains("crash-probe.rs"),
        "and so must where it was raised: {message}"
    );

    // The one way a crash differs from an ordinary failure, and the reason
    // this is asserted through a process rather than taken on trust: under
    // --json an ordinary failure says nothing on stderr, and a crash reports
    // there anyway, because it is a bug in xlsplice rather than something the
    // caller did.
    assert!(
        stderr(&out).starts_with("error: "),
        "a crash reports on stderr in every mode: {}",
        stderr(&out)
    );
}

#[test]
fn a_crash_in_text_mode_says_nothing_on_stdout() {
    let out = crash(&[]);

    assert_eq!(exit_code(&out), 1);
    assert_eq!(
        stdout(&out),
        "",
        "a text-mode failure must leave stdout empty"
    );
    assert!(
        stderr(&out).contains("internal error"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("bug in xlsplice"),
        "a crash must say it is ours to fix: {}",
        stderr(&out)
    );
}

#[test]
fn the_binary_installs_the_panic_hook_before_it_parses() {
    let main = std::fs::read_to_string("src/main.rs").expect("the binary's source is in the tree");
    let (before, after) = main
        .split_once("crash::install")
        .expect("main must install the panic hook");

    assert!(
        !before.contains("try_get_matches_from"),
        "the hook goes in before clap parses, because a panic can precede it"
    );
    assert!(
        after.contains("try_get_matches_from"),
        "and clap still parses after it"
    );
}
