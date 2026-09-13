//! What the repository says about shipping: the release workflow's triggers
//! and contents, and what the README tells someone installing.
//!
//! None of this runs the workflow. What it holds is the two things about it
//! that are cheap to get wrong and expensive to notice: that it runs on a
//! version tag and on nothing else, because Actions minutes on this private
//! repository are limited and a macOS runner spends them ten times over; and
//! that every archive carries the skill file, because a release an agent
//! cannot read is half a release.

use std::path::{Path, PathBuf};

/// A file of the repository, by its path from the root.
fn repository_file(name: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("the repository must hold {name}: {err}"))
}

fn release_workflow() -> String {
    repository_file(".github/workflows/release.yml")
}

/// The targets the release is built for, as the ticket names them.
const TARGETS: [&str; 3] = [
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

#[test]
fn the_release_workflow_runs_on_a_version_tag() {
    let workflow = release_workflow();
    let triggers = workflow
        .split_once("\non:\n")
        .expect("the workflow declares its triggers")
        .1
        .split("\n\n")
        .next()
        .expect("the triggers are a block of their own")
        .to_owned();

    assert!(triggers.contains("tags:"), "{triggers}");
    assert!(triggers.contains("\"v*\""), "{triggers}");
}

/// The one that matters for the bill: nothing but a tag starts this.
#[test]
fn the_release_workflow_runs_on_nothing_else() {
    let workflow = release_workflow();
    let triggers = workflow
        .split_once("\non:\n")
        .expect("the workflow declares its triggers")
        .1
        .split("\n\n")
        .next()
        .expect("the triggers are a block of their own")
        .to_owned();

    for forbidden in [
        "pull_request",
        "schedule",
        "workflow_dispatch",
        "branches:",
        "release:",
    ] {
        assert!(
            !triggers.contains(forbidden),
            "the release workflow must not run on {forbidden}: {triggers}"
        );
    }
    assert_eq!(
        triggers.matches("push:").count(),
        1,
        "the tag push is the only trigger: {triggers}"
    );
}

/// `test.yml` is what guards a push and a pull request, and it is the only
/// workflow that may.
#[test]
fn the_test_workflow_is_the_only_one_a_push_or_a_pull_request_starts() {
    let test = repository_file(".github/workflows/test.yml");

    assert!(test.contains("pull_request"));
    assert!(test.contains("branches: [master]"));
    assert!(
        !test.contains("schedule"),
        "nothing here runs on a schedule"
    );
}

#[test]
fn the_release_workflow_builds_every_target_the_ticket_names() {
    let workflow = release_workflow();

    for target in TARGETS {
        assert!(
            workflow.contains(target),
            "the release must be built for {target}"
        );
    }
}

/// Every archive carries the skill file, because a release an agent cannot
/// read is half a release.
#[test]
fn every_archive_carries_the_binary_and_the_skill_file() {
    let workflow = release_workflow();

    assert!(
        workflow.contains("cp SKILL.md"),
        "the archives must carry SKILL.md"
    );
    assert!(
        workflow.contains("release/xlsplice.exe"),
        "the Windows archive must carry the binary"
    );
    assert!(
        workflow.contains("release/xlsplice\""),
        "the other archives must carry the binary"
    );
}

/// The archive names carry the tag and the target, which is what the README's
/// table promises someone downloading one.
#[test]
fn the_readmes_archive_names_are_the_ones_the_workflow_writes() {
    let readme = repository_file("README.md");
    let workflow = release_workflow();

    assert!(
        workflow.contains(r#"name="xlsplice-${GITHUB_REF_NAME}-${{ matrix.target }}""#),
        "the workflow names its archives after the tag and the target"
    );
    for target in TARGETS {
        let extension = match target.contains("windows") {
            true => "zip",
            false => "tar.gz",
        };
        let named = format!("xlsplice-vX.Y.Z-{target}.{extension}");
        assert!(
            readme.contains(&named),
            "the README must name the archive {named}"
        );
    }
}

#[test]
fn the_readme_covers_both_ways_of_installing() {
    let readme = repository_file("README.md");

    assert!(
        readme.contains("gh release download"),
        "the README must show installing from a release"
    );
    assert!(
        readme.contains("cargo install --git https://github.com/niko86/xlsplice"),
        "the README must show the cargo install fallback"
    );
}

/// The variable a caller sets to say which binary to run. Nothing in this
/// repository reads it — it is the wrapper's — so the README is the only place
/// it is written down, which is why it is asserted here.
#[test]
fn the_readme_names_the_variable_a_caller_overrides_the_binary_with() {
    let readme = repository_file("README.md");

    assert!(readme.contains("XLSPLICE_BIN"), "{readme}");
    assert!(
        readme.contains("export XLSPLICE_BIN="),
        "the README must show how to set it"
    );
}

/// A tag that disagrees with the manifest would publish archives named for a
/// version their binaries do not report, so the workflow checks before it
/// builds.
#[test]
fn the_workflow_holds_the_tag_to_the_manifests_version() {
    let workflow = release_workflow();

    assert!(
        workflow.contains("The tag and the manifest must agree"),
        "the workflow must check the tag against Cargo.toml"
    );
    assert!(workflow.contains("Cargo.toml"), "{workflow}");
}

/// One job publishes, so three builds do not race to create one release.
#[test]
fn one_job_publishes_the_release() {
    let workflow = release_workflow();

    assert_eq!(
        workflow.matches("gh release create").count(),
        1,
        "exactly one step creates the release"
    );
    assert!(
        workflow.contains("needs: build"),
        "publishing waits for every build"
    );
}
