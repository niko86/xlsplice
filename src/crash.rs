//! The panic hook: the one way out of a process that no verb can take.
//!
//! What a crash *says* is [`crate::render::crash`]'s, like every other
//! outcome. What is here is the hook that carries it out: take what the panic
//! knows, ask `render` what that looks like, put the two strings on the two
//! streams, and exit with the code. It decides nothing.
//!
//! It sits in the library rather than beside the other process-level things in
//! the binary's `out.rs`, for a reason that is about testing rather than
//! design. The hook ends in [`std::process::exit`], so the only way to watch
//! it work is from outside a process that has run it — and a second binary, an
//! example, and the binary itself are each their own crate root, so a module
//! private to one of them is reachable from none of the others. In the library
//! it is reachable from all of them, which is what lets `tests/contract.rs`
//! watch the real hook run without a trapdoor in `xlsplice`'s argv. #38 took
//! that trapdoor out and #44 declined to put it back.
//!
//! Anything embedding the library gets the same crash contract by calling
//! [`install`], which is a smaller surprise than it sounds: a library that
//! does not install it keeps Rust's own hook and is unaffected.

use std::io::Write;

use crate::render::{self, OutputMode};

/// Route every panic through the envelope, so no failure mode is unparseable.
///
/// The hook exits the process itself rather than letting the unwind reach
/// `main`, which would exit 101 and skip the contract. Exiting here also skips
/// destructors, which is what the "non-zero means nothing was written"
/// guarantee wants: a half-finished temporary file is abandoned, not renamed.
///
/// `mode` is settled before this is called and captured, because a panic can
/// precede the parse that would otherwise decide it.
pub fn install(mode: OutputMode) {
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|at| (at.file(), at.line()));
        let what = info.payload_as_str().unwrap_or("panicked");

        let rendered = render::crash(at, what, mode);
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
        let _ = std::io::stdout().flush();
        std::process::exit(i32::from(rendered.exit));
    }));
}
