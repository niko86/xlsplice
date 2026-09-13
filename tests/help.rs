//! What the tool says about itself: every verb's example, the two help
//! topics, and the skill file.
//!
//! The point of these is that documentation cannot go stale quietly. Every
//! example the tool prints, and every example the skill file carries, is run
//! here exactly as it is written, in a directory holding the files the
//! examples name. An example that stops working stops the build.
//!
//! The verbs are enumerated from the tool's own help rather than listed here,
//! so a verb added without an example is caught by the same test.

mod support;

use std::path::Path;

use serde_json::json;
use support::{Workspace, exit_code, json, run, run_in, stderr, stdout};
use xlsplice::error::ErrorCode;
use xlsplice::help::Topic;

/// The names the examples use for the files they act on. A directory holding
/// these is what an example runs in.
const PACKAGE: &str = "book.xlsx";
const OTHER: &str = "other.xlsx";
const BATCH: &str = "batch.json";

/// A directory holding everything the examples name.
fn stage(label: &str) -> Workspace {
    let workspace = Workspace::new(label);
    let package = workspace.copy_of("feature.xlsx");
    let staged = workspace.dir().join(PACKAGE);
    std::fs::copy(&package, &staged).expect("a test must be able to stage its own package");
    std::fs::copy(&package, workspace.dir().join(OTHER))
        .expect("a test must be able to stage its own package");
    workspace.file(
        BATCH,
        br#"[{"op": "set", "target": "Inputs!A1", "type": "number", "value": "42"}]"#,
    );
    workspace
}

/// Every verb the tool offers, taken from its own help so that a verb added
/// without an example is caught here.
fn verbs() -> Vec<String> {
    let help = stdout(&run(&["--help"]));
    let commands = help
        .split_once("Commands:\n")
        .expect("the help lists commands")
        .1;
    commands
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// The example lines a block of help carries: every line that is an
/// invocation of the tool.
fn examples_in(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("xlsplice "))
        .map(|line| line.split_whitespace().skip(1).map(str::to_owned).collect())
        .collect()
}

/// Run an example in `dir`, and say what it exited with.
fn ran(dir: &Path, example: &[String]) -> std::process::Output {
    let args: Vec<&str> = example.iter().map(String::as_str).collect();
    run_in(dir, &args)
}

#[test]
fn every_verb_shows_an_example() {
    for verb in verbs() {
        let help = stdout(&run(&[&verb, "--help"]));

        assert!(
            !examples_in(&help).is_empty(),
            "`xlsplice {verb} --help` shows no example:\n{help}"
        );
    }
}

/// The one that matters: what the help prints is what a caller may type.
#[test]
fn every_verbs_example_runs_as_written() {
    for verb in verbs() {
        let help = stdout(&run(&[&verb, "--help"]));

        for example in examples_in(&help) {
            let workspace = stage(&format!("verb-{verb}"));

            let out = ran(workspace.dir(), &example);

            assert_eq!(
                exit_code(&out),
                0,
                "`xlsplice {}` from `{verb} --help` failed: {}",
                example.join(" "),
                stderr(&out)
            );
        }
    }
}

/// A verb with subcommands has an example under each of them too, because
/// that is where a caller reads what the subcommand takes.
#[test]
fn every_subcommand_shows_an_example_that_runs_as_written() {
    let actions = ["get", "set", "unset"];

    for action in actions {
        let help = stdout(&run(&["props", action, "--help"]));
        let examples = examples_in(&help);
        assert!(!examples.is_empty(), "`props {action} --help` shows none");

        for example in examples {
            let workspace = stage(&format!("props-{action}"));

            let out = ran(workspace.dir(), &example);

            assert_eq!(
                exit_code(&out),
                0,
                "`xlsplice {}` failed: {}",
                example.join(" "),
                stderr(&out)
            );
        }
    }
}

#[test]
fn both_help_topics_exist_and_say_something() {
    for topic in Topic::ALL {
        let out = run(&["help", topic.as_str()]);

        assert_eq!(exit_code(&out), 0, "{}: {}", topic.as_str(), stderr(&out));
        assert!(
            stdout(&out).lines().count() > 10,
            "{} is too short to be a topic",
            topic.as_str()
        );
    }
}

/// The topic is generated from the codes rather than transcribed beside them,
/// and this is what says so: every code the library has, with the number it
/// exits with, and no number the library does not use.
#[test]
fn the_exit_code_topic_matches_the_librarys_own_mapping() {
    let text = stdout(&run(&["help", "exit-codes"]));

    for code in ErrorCode::ALL {
        let row = text
            .lines()
            .find(|line| line.split_whitespace().nth(1) == Some(code.as_str()))
            .unwrap_or_else(|| panic!("the topic must carry a row for {}: {text}", code.as_str()));
        assert_eq!(
            row.split_whitespace().next(),
            Some(code.exit_code().to_string().as_str()),
            "the row for {} must lead with its exit code: {row}",
            code.as_str()
        );
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("  ") && line.trim().starts_with(char::is_numeric))
            .count(),
        ErrorCode::ALL.len() + 1,
        "one row per code, and one for success: {text}"
    );
}

/// The skill file's table is transcribed rather than generated, so it is held
/// against the same mapping.
#[test]
fn the_skill_files_exit_code_table_matches_the_librarys_own_mapping() {
    let skill = skill_file();

    for code in ErrorCode::ALL {
        let row = format!("| {}    | `{}`", code.exit_code(), code.as_str());
        assert!(skill.contains(&row), "SKILL.md's table must carry '{row}'");
    }
}

#[test]
fn a_topic_that_is_not_one_is_a_usage_error_naming_the_ones_there_are() {
    let out = run(&["help", "envelope", "--json"]);

    assert_eq!(exit_code(&out), 2);
    let message = json(&out)["error"]["message"]
        .as_str()
        .expect("a failed envelope carries a message")
        .to_owned();
    for topic in Topic::ALL {
        assert!(message.contains(topic.as_str()), "{message}");
    }
}

#[test]
fn a_topic_under_json_is_the_same_text_in_the_envelope() {
    let out = run(&["help", "json", "--json"]);

    assert_eq!(exit_code(&out), 0, "{}", stderr(&out));
    let body = json(&out);
    assert_eq!(body["topic"], json!("json"));
    assert_eq!(
        body["text"].as_str().expect("the topic's text"),
        Topic::Json.text(),
        "the envelope carries the topic whole, trailing newline and all"
    );
}

/// The skill file as the repository holds it.
fn skill_file() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("SKILL.md"))
        .expect("the repository must hold SKILL.md")
}

/// Every command the skill file shows, from its fenced blocks. A fenced block
/// marked `json` is a document rather than a command, so it is skipped.
fn skill_examples() -> Vec<Vec<String>> {
    let skill = skill_file();
    let mut examples = Vec::new();
    let mut inside = false;
    for line in skill.lines() {
        if let Some(language) = line.strip_prefix("```") {
            inside = language.trim().is_empty();
            continue;
        }
        if inside && line.starts_with("xlsplice ") {
            examples.push(line.split_whitespace().skip(1).map(str::to_owned).collect());
        }
    }
    examples
}

#[test]
fn the_skill_file_shows_an_example_of_every_verb() {
    let examples = skill_examples();
    let shown: Vec<&String> = examples
        .iter()
        .filter_map(|example| example.first())
        .collect();

    for verb in verbs() {
        assert!(
            shown.iter().any(|first| **first == verb),
            "SKILL.md shows no example of `{verb}`"
        );
    }
}

/// The other one that matters: what the skill file tells an agent to type is
/// what an agent may type.
#[test]
fn every_example_in_the_skill_file_runs_as_written() {
    let examples = skill_examples();
    assert!(
        examples.len() > 10,
        "SKILL.md should show more than {} examples",
        examples.len()
    );

    for example in examples {
        let workspace = stage("skill");

        let out = ran(workspace.dir(), &example);

        assert_eq!(
            exit_code(&out),
            0,
            "`xlsplice {}` from SKILL.md failed: {}",
            example.join(" "),
            stderr(&out)
        );
    }
}

/// The skill file shows `--` before its operands, because a defined name may
/// carry characters a parser would take for something else and a package path
/// may start with `-`.
#[test]
fn the_skill_files_examples_put_the_operands_behind_a_double_dash() {
    for example in skill_examples() {
        let names_a_file = example
            .iter()
            .any(|argument| argument.ends_with(".xlsx") || argument.ends_with(".json"));
        if !names_a_file {
            continue;
        }
        assert!(
            example.iter().any(|argument| argument == "--"),
            "`xlsplice {}` names a file and shows no `--`",
            example.join(" ")
        );
    }
}

/// Every verb honours the two diagnostic flags the same way: stderr only,
/// stdout untouched, and an error still printed however quiet it was asked
/// to be.
#[test]
fn every_verb_honours_quiet_and_verbose_on_stderr_alone() {
    for verb in verbs() {
        let workspace = stage(&format!("flags-{verb}"));
        let example = examples_in(&stdout(&run(&[&verb, "--help"])))
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{verb} shows an example"));

        // A fresh package for each run: a write of a value already there
        // reports itself unchanged, which would look like a flag moving
        // stdout when it is the second write doing it.
        let loudly = stage(&format!("loud-{verb}"));
        let hushedly = stage(&format!("hush-{verb}"));
        let plain = ran(workspace.dir(), &example);
        let loud = ran(
            loudly.dir(),
            &[example.clone(), vec!["--verbose".to_owned()]].concat(),
        );
        let hushed = ran(
            hushedly.dir(),
            &[example.clone(), vec!["--quiet".to_owned()]].concat(),
        );

        assert_eq!(
            stdout(&loud),
            stdout(&plain),
            "{verb}: --verbose moved stdout"
        );
        assert_eq!(
            stdout(&hushed),
            stdout(&plain),
            "{verb}: --quiet moved stdout"
        );
        assert_eq!(
            stderr(&hushed),
            "",
            "{verb}: --quiet left something on stderr"
        );
        assert!(
            !stderr(&loud).is_empty(),
            "{verb}: --verbose said nothing on stderr"
        );
        assert_eq!(exit_code(&loud), 0, "{verb}");
        assert_eq!(exit_code(&hushed), 0, "{verb}");
    }
}

#[test]
fn the_help_verb_lists_both_topics_with_what_each_covers() {
    let help = stdout(&run(&["help", "--help"]));

    for topic in Topic::ALL {
        assert!(help.contains(topic.as_str()), "{}: {help}", topic.as_str());
        assert!(
            help.contains(topic.description()),
            "{}: {help}",
            topic.as_str()
        );
    }
}

/// A directory of packages is what the examples act on, so the staging is
/// asserted too: a test that stages nothing would pass every example that
/// happens not to need a file.
#[test]
fn the_staged_directory_holds_what_the_examples_name() {
    let workspace = stage("staging");

    for name in [PACKAGE, OTHER, BATCH] {
        assert!(
            workspace.dir().join(name).exists(),
            "an example names {name}"
        );
    }
}
