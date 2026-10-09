//! The harness held to what it promises: tests of `support` itself, where a
//! promise the suites lean on could break without any of them failing.
//!
//! Only one is here. The wait on the crash probe has a bound (#49), and a
//! bound that never fires looks exactly like one that works, because the hang
//! it guards against could not be made to happen on demand. So it is made to
//! fire here, on a child that is sure to outlive it.

mod support;

use support::binary::output_within;

/// `sleep` stands in for a probe that never leaves the loader: what matters
/// is that the wait ends, that what it ends with names the child well enough
/// to go and look at it, and that what can only be had from a live child was
/// kept before the child was killed.
#[cfg(unix)]
#[test]
fn a_child_that_outlives_its_bound_is_reported_by_pid_and_state_rather_than_waited_for() {
    let mut sleeper = std::process::Command::new("/bin/sleep");
    sleeper.arg("30");
    let started = std::time::Instant::now();

    let overrun = output_within(&mut sleeper, std::time::Duration::from_millis(100))
        .expect_err("a child that sleeps for 30s must outlive a 100ms bound");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the wait must end near its bound, not when the child does: {:?}",
        started.elapsed()
    );
    assert_ne!(overrun.pid, 0, "{overrun}");
    let state = overrun
        .state
        .as_ref()
        .expect("ps must say what the child was doing");
    assert!(
        state.starts_with('S'),
        "a sleeping child is reported as sleeping: {overrun}"
    );
    assert!(
        overrun.reaped,
        "a sleeping child dies when killed: {overrun}"
    );

    // What a killed child cannot give afterwards, kept before it was killed.
    let binary = overrun
        .evidence
        .binary
        .as_ref()
        .expect("the binary the child ran must be copied");
    assert_eq!(
        std::fs::read(binary).expect("the copy must be readable"),
        std::fs::read("/bin/sleep").expect("the original must be readable"),
        "the copy is the binary the child ran: {overrun}"
    );
    if cfg!(target_os = "macos") {
        let sample = overrun
            .evidence
            .sample
            .as_ref()
            .expect("on macOS the child must be sampled");
        let text = std::fs::read_to_string(sample).expect("the sample must be readable");
        assert!(
            text.contains("Call graph"),
            "the sample says where the child's threads were: {overrun}"
        );
    }

    let report = overrun.to_string();
    assert!(
        report.contains(&format!("pid {}", overrun.pid))
            && report.contains(state.as_str())
            && report.contains(&overrun.evidence.dir.display().to_string()),
        "the failure a test prints must carry the pid, the state and where the evidence is: \
         {report}"
    );

    // A real overrun's evidence is kept for whoever reads the failure; this
    // one has been read, and every run would otherwise leave another.
    std::fs::remove_dir_all(&overrun.evidence.dir).expect("the evidence must be removable");
}
