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

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

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
/// that sat in the dynamic loader in #49. Built once, the probe is finished
/// before any test starts it.
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
/// rather than an estimate of how long it takes. Nothing else here is bounded:
/// the binary itself has never hung, and the probe has.
pub fn crash(args: &[&str]) -> std::process::Output {
    let mut probe = std::process::Command::new(crash_probe());
    probe.args(args);
    output_within(&mut probe, CRASH_BOUND).unwrap_or_else(|overrun| panic!("{overrun}"))
}

/// How long the crash probe has to panic and exit before its test gives up.
const CRASH_BOUND: Duration = Duration::from_secs(30);

/// How long a killed child, or `ps`, gets to finish before it is given up on.
/// A process that cannot be killed must not hang the suite in the wait after
/// the kill, which is the one place the bound would otherwise not reach.
const GRACE: Duration = Duration::from_secs(5);

/// How often a bounded wait asks whether the child is done.
const POLL: Duration = Duration::from_millis(10);

/// A child still running when its bound ran out: the test fails with this
/// rather than hanging. Its binary is left where it is, for whoever is chasing
/// why.
#[derive(Debug)]
pub struct Overrun {
    pub program: std::ffi::OsString,
    pub bound: Duration,
    pub pid: u32,
    /// What `ps` said the child was doing when the bound ran out, or why
    /// there is no answer: on Windows there is no `ps`.
    pub state: Result<String, String>,
    /// Whether the kill took within [`GRACE`]. When it did not, the child is
    /// still there, and its pid is the way to it.
    pub reaped: bool,
}

impl std::fmt::Display for Overrun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match &self.state {
            Ok(state) => state.as_str(),
            Err(why) => why.as_str(),
        };
        let after = if self.reaped {
            "killed"
        } else {
            "killed, but still not gone"
        };
        write!(
            f,
            "{:?} was still running after {:?}: pid {}, state {state}, {after}",
            self.program, self.bound, self.pid
        )
    }
}

/// What `Command::output` does — stdin empty, stdout and stderr captured —
/// except that a child still running after `bound` is an [`Overrun`], with its
/// pid and the state `ps` reports for it, rather than a wait that never ends.
///
/// The bound is on the child, not on its streams: a grandchild that kept them
/// open after the child exited would still hold the wait. The crash probe
/// starts no processes, so nothing here does that.
pub fn output_within(
    command: &mut std::process::Command,
    bound: Duration,
) -> Result<std::process::Output, Overrun> {
    use std::process::Stdio;

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the child must be runnable");

    // Both streams are drained while the child runs, as `output` drains them,
    // so a child that writes more than a pipe holds is not what stops it.
    let stdout = drain(child.stdout.take().expect("stdout was piped"));
    let stderr = drain(child.stderr.take().expect("stderr was piped"));

    let Some(status) = wait_within(&mut child, bound) else {
        let pid = child.id();
        let state = process_state(pid);
        let _ = child.kill();
        let reaped = wait_within(&mut child, GRACE).is_some();
        // The readers are left behind: the kill closes their pipes when it
        // takes, and when it does not there is nothing to wait for them on.
        return Err(Overrun {
            program: command.get_program().to_owned(),
            bound,
            pid,
            state,
            reaped,
        });
    };

    Ok(std::process::Output {
        status,
        stdout: stdout.join().expect("the stdout reader must not panic"),
        stderr: stderr.join().expect("the stderr reader must not panic"),
    })
}

/// Read `stream` to its end on a thread of its own.
fn drain(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .expect("a child's stream must be readable");
        bytes
    })
}

/// Wait for `child` to exit, for no longer than `bound`.
fn wait_within(child: &mut std::process::Child, bound: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(status) = child.try_wait().expect("the child must be waitable") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL);
    }
}

/// The state `ps` gives for `pid`, or why there is none. `ps` is held to
/// [`GRACE`] like everything else on this path, so that a test that has
/// already timed out cannot hang in finding out why.
fn process_state(pid: u32) -> Result<String, String> {
    use std::process::Stdio;

    let mut ps = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("unknown (ps: {error})"))?;
    let Some(status) = wait_within(&mut ps, GRACE) else {
        let _ = ps.kill();
        return Err(format!("unknown (ps did not answer within {GRACE:?})"));
    };
    if !status.success() {
        return Err(format!("unknown (ps exited {status})"));
    }
    let mut out = String::new();
    ps.stdout
        .take()
        .expect("ps's stdout was piped")
        .read_to_string(&mut out)
        .map_err(|error| format!("unknown (reading ps: {error})"))?;
    Ok(out.trim().to_owned())
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
