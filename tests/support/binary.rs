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
use std::path::Path;

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
