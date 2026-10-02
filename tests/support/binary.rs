//! Spawning the binary: argv in, two streams and an exit code out.
//!
//! What only a process can show belongs here — what clap does with an argument
//! no verb ever sees, what `--` does to the operand after it, what a verb
//! reading stdin does when stdin is a pipe, and what the exit code actually
//! is. Everything else is cheaper and clearer at the library seam in
//! [`super::library`].

// Every suite compiles the whole of this and uses the part of it that suits
// what it is asking about, so what one suite does not reach is not dead.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};

/// Run the binary with `args` and both streams captured, so stdout is a pipe
/// rather than a terminal.
///
/// `output` gives the child an empty pipe on stdin, which is what a verb
/// reading stdin sees when nothing was piped in: not a terminal, and nothing
/// there.
pub fn run(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .args(args)
        .output()
        .expect("the binary under test must be runnable")
}

/// The same, from inside `dir`, for a test about how an operand is spelled
/// rather than about what it names.
pub fn run_in(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary under test must be runnable")
}

/// The same, with `input` piped to the child's stdin.
pub fn run_with_stdin(args: &[&str], input: &str) -> std::process::Output {
    use std::process::Stdio;

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_xlsplice"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary under test must be runnable");
    child
        .stdin
        .take()
        .expect("the child was given a pipe on stdin")
        .write_all(input.as_bytes())
        .expect("the child must take what is piped to it");
    child
        .wait_with_output()
        .expect("the child must finish and be waited for")
}

/// The crash probe, built and ready to run: an example binary that installs
/// the real panic hook and then panics, so that a test can watch a real
/// process do what only a real process does.
///
/// It is an example rather than a second verb or a second binary because
/// neither of those can be had without a cost #38 already refused: a verb
/// would be back on the published surface, and a `[[bin]]` would be installed
/// by `cargo install` and would have to be kept out of the release archive by
/// hand. An example is built by `cargo test`, shipped by nothing, and
/// installed by nothing.
///
/// The build is asked for here rather than assumed. A full `cargo test` builds
/// examples and `cargo test --test contract` does not, so a test that took the
/// binary on trust would pass under one invocation and fail under the other.
/// Asking costs a cargo no-op when it is already built.
///
/// It is asked once per test binary, not once per test. The crash tests run on
/// parallel threads, and when each ran its own build, one test could start the
/// probe while the other's build was relinking it — one suspect for the probe
/// that sat in the dynamic loader in #49. Once, the build is over before any
/// test starts the probe.
pub fn crash_probe() -> PathBuf {
    static PROBE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PROBE.get_or_init(build_crash_probe).clone()
}

fn build_crash_probe() -> PathBuf {
    let mut build = std::process::Command::new(env!("CARGO"));
    build
        .args(["build", "--quiet", "--example", "crash-probe"])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    // Whichever profile the suite itself was built with, so that the probe
    // lands beside the binary this test found rather than in the other one.
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    let status = build
        .status()
        .expect("cargo must be runnable to build the crash probe");
    assert!(status.success(), "the crash probe must build");

    let binary = Path::new(env!("CARGO_BIN_EXE_xlsplice"));
    let mut name = std::ffi::OsString::from("crash-probe");
    // `.exe` on Windows, nothing anywhere else, taken from the binary beside
    // it rather than spelled out per platform.
    if let Some(extension) = binary.extension() {
        name.push(".");
        name.push(extension);
    }
    binary
        .parent()
        .expect("the binary under test sits in a directory")
        .join("examples")
        .join(name)
}

/// Run the crash probe and give back what the process did.
///
/// The wait is bounded, because the probe once never got past the dynamic
/// loader and the suite sat until it was killed (#49). The probe's job is to
/// panic within milliseconds, so [`CRASH_BOUND`] is room for a loaded machine
/// rather than an estimate of how long it takes.
pub fn crash(args: &[&str]) -> std::process::Output {
    let mut probe = std::process::Command::new(crash_probe());
    probe.args(args);
    output_within(&mut probe, CRASH_BOUND)
}

/// How long the crash probe has to panic and exit before its test gives up.
pub const CRASH_BOUND: std::time::Duration = std::time::Duration::from_secs(30);

/// What `Command::output` does — stdin empty, stdout and stderr captured —
/// except that a child still running after `bound` fails the test, with its
/// pid and the state `ps` reports for it, rather than hanging it.
///
/// The child is killed once its state has been read, so a failed run leaves
/// nothing behind; its binary is left where it is, for whoever is chasing why.
pub fn output_within(
    command: &mut std::process::Command,
    bound: std::time::Duration,
) -> std::process::Output {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::Instant;

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the child must be runnable");

    // Both streams are drained while the child runs, as `output` drains them,
    // so a child that writes more than a pipe holds is not what stops it.
    fn drain(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stream
                .read_to_end(&mut bytes)
                .expect("a child's stream must be readable");
            bytes
        })
    }
    let stdout = drain(child.stdout.take().expect("stdout was piped"));
    let stderr = drain(child.stderr.take().expect("stderr was piped"));

    let deadline = Instant::now() + bound;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the child must be waitable") {
            break status;
        }
        if Instant::now() >= deadline {
            let pid = child.id();
            let state = process_state(pid);
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "the child was still running after {bound:?}: pid {pid}, state {state}, \
                 killed; program {:?}",
                command.get_program()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };

    std::process::Output {
        status,
        stdout: stdout.join().expect("the stdout reader must not panic"),
        stderr: stderr.join().expect("the stderr reader must not panic"),
    }
}

/// The state `ps` gives for `pid`, or why there is none: on Windows there is
/// no `ps`, and a test that has already timed out has nothing to gain from a
/// second failure.
fn process_state(pid: u32) -> String {
    match std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        Ok(out) => format!("unknown (ps exited {})", out.status),
        Err(error) => format!("unknown (ps: {error})"),
    }
}

pub fn stdout(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout must be UTF-8")
}

pub fn stderr(out: &std::process::Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr must be UTF-8")
}

pub fn exit_code(out: &std::process::Output) -> i32 {
    out.status
        .code()
        .expect("the binary must exit, not die on a signal")
}

pub fn json(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&stdout(out)).expect("stdout under --json must be one JSON document")
}
