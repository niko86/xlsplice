//! Panic on purpose, so that `tests/contract.rs` can watch the real panic hook
//! run in a real process.
//!
//! It exists because of what #38 settled and #44 declined to undo. The hook
//! ends in [`std::process::exit`], so nothing can observe it from inside the
//! process that ran it; and `xlsplice` itself has no way to be asked to panic,
//! because the hidden verb that once did that was a command on the published
//! surface whose only purpose was to be crashed. So the thing that panics is
//! this instead: the same hook, the same `render::crash`, the same two streams
//! and the same exit code, in a process that nobody ships.
//!
//! **What it does not hold.** It is not `xlsplice`, so it says nothing about
//! `main.rs` installing the hook, and nothing about it being installed before
//! clap can fail. `the_binary_installs_the_panic_hook_before_it_parses` reads
//! the source for that, and the two together are the whole of the claim.
//!
//! An example rather than a second binary: `cargo build --release` does not
//! build it, `cargo install` does not install it, and the release archive is
//! assembled by naming its files, so there is no way for this to reach anyone.
//!
//! `--json` picks the mode, the way it does everywhere else.

use std::io::IsTerminal;

use xlsplice::render::OutputMode;

fn main() {
    let json = std::env::args().any(|argument| argument == "--json");
    // The same question `out::mode` asks, and the same answer: under the test
    // both streams are pipes, so this is the piped shape of either mode.
    xlsplice::crash::install(OutputMode::new(json, std::io::stdout().is_terminal()));

    panic!("the crash probe was asked to panic");
}
