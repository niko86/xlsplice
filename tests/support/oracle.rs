//! The oracle: a real Excel deciding whether a package opens clean or demands
//! repair.
//!
//! Every other suite here asserts against xlsplice's own reading of a package,
//! which is the reading under test. Only Excel can say whether a package it
//! has never seen is one it will open without complaint, so this is the one
//! judge the tool does not supply.
//!
//! One operation, [`opened`], answers [`Verdict::Clean`], [`Verdict::Repair`]
//! or [`Verdict::Unavailable`]. Unavailable is not a failure: a machine with
//! no Excel, or an operating system with no backend here, cannot answer, and a
//! suite that cannot be run is not a suite that has failed. Setting
//! [`REQUIRE`] to `require` turns that round, for the one machine where the
//! oracle is meant to run and its silence would mean a broken harness rather
//! than an absent Excel.
//!
//! Nothing in here runs unless an ignored test is asked for by name, because
//! driving a GUI application is slow, needs a logged-in session, and takes the
//! screen away from whoever is using the machine. It also needs Accessibility
//! permission for whatever runs the tests, because the verdict is read off the
//! screen: without it System Events reports no windows for any application at
//! all and every package looks like a timeout. So the permission is asked
//! about before Excel is launched, and its absence is a skip that says what to
//! grant rather than a suite of timeouts. Setting `XLSPLICE_ORACLE_TRACE`
//! prints what Excel was seen to do, which is where to start when one does.
//!
//! ## Why Excel is driven the way it is
//!
//! Excel is sandboxed, so it may only read a file it has been handed access
//! to. Told to `open` a path over an Apple Event it puts up the sandbox's own
//! file-open panel and waits for a human, which arrives here as a timeout with
//! nothing said. Handed the file through LaunchServices, the way a double
//! click does it, it is granted the file and opens it. So the package is
//! always opened with `/usr/bin/open -a`, and Apple Events are only ever used
//! to ask questions afterwards.
//!
//! Excel is also never launched by an Apple Event, and never opened into at no
//! windows. An Excel that an Apple Event started comes up with no window, and
//! an Excel showing no window wedges when a document is opened into it: it
//! goes on answering questions cheerfully and silently does nothing it is
//! told, and only a kill gets it back. So every run starts from an Excel that
//! is not running, the package itself is the launch — which also avoids the
//! empty `Book1` a document-less launch creates — and Excel is quit afterwards
//! rather than left sitting at no windows.
//!
//! The repair prompt is a modal dialog rather than anything Excel's scripting
//! interface reports, so System Events reads it and answers it. The verdict is
//! also legible after the fact in the document window's title, which Excel
//! suffixes with `Repaired`, and both are believed.

#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

/// What Excel made of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Excel opened it and said nothing.
    Clean,
    /// Excel would not open it as it stands: it offered to recover what it
    /// could, or opened a repaired copy.
    Repair,
    /// No answer, and why. Not a failure unless [`REQUIRE`] says it is.
    Unavailable(String),
}

/// The variable that says an oracle must be there. Set it to `require` on a
/// machine where a missing Excel means the harness is broken rather than the
/// machine being the wrong one.
pub const REQUIRE: &str = "XLSPLICE_ORACLE";

/// Where a Mac keeps Excel. Availability is decided by this rather than by
/// launching anything, so a test on a machine without Excel costs nothing.
const EXCEL: &str = "/Applications/Microsoft Excel.app";

/// How long to wait for Excel to open the package. Generous, because a cold
/// Excel takes some twenty seconds to draw its first window on this machine
/// and a laden one takes longer.
const WAIT: u32 = 90;

/// How long to leave between an Excel going and the next one being asked for.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(2000);

/// Ask Excel to open `package`, and say what it made of it.
///
/// Excel is left with no workbooks open however this ends: the one that was
/// opened is closed without saving, and a repair prompt is answered `No` so
/// that nothing is recovered and nothing is left on screen.
///
/// An Excel that already has a workbook open belongs to whoever is using the
/// machine, so the oracle refuses rather than closing something it did not
/// open.
pub fn opened(package: &Path) -> Verdict {
    opened_by(Path::new(EXCEL), package)
}

/// The oracle, told where Excel is. Only [`opened`] and a test simulating a
/// machine without Excel have any business saying: there is one Excel on a
/// Mac and it is where [`EXCEL`] says.
pub fn opened_by(excel: &Path, package: &Path) -> Verdict {
    if !cfg!(target_os = "macos") {
        return Verdict::Unavailable("there is no oracle backend for this platform".to_owned());
    }
    if !excel.exists() {
        return Verdict::Unavailable(format!("Excel is not installed: no {}", excel.display()));
    }
    if let Some(refusal) = reads_the_screen() {
        return Verdict::Unavailable(refusal);
    }
    if let Some(refusal) = readied() {
        return Verdict::Unavailable(refusal);
    }
    match launched(package) {
        Err(why) => Verdict::Unavailable(why),
        Ok(()) => {
            let verdict = watched(package);
            tidied();
            verdict
        }
    }
}

/// Ask the oracle, and say what a test should do about no answer.
///
/// `None` means skip. The reason is printed either way, because a suite that
/// quietly skipped is one nobody notices has stopped running.
pub fn asked(package: &Path) -> Option<Verdict> {
    decided(opened(package), required())
}

/// What a test does about an answer, apart from the asking so that both
/// halves of the policy can be held to without an Excel to hand.
///
/// `None` means skip, and the reason is printed rather than swallowed: a
/// suite that quietly skipped is one nobody notices has stopped running.
pub fn decided(said: Verdict, required: bool) -> Option<Verdict> {
    match said {
        Verdict::Unavailable(why) => {
            assert!(
                !required,
                "{REQUIRE}=require, and the oracle could not answer: {why}"
            );
            eprintln!("skipping: the oracle could not answer: {why}");
            None
        }
        answered => Some(answered),
    }
}

/// Whether the environment says an oracle must be there.
fn required() -> bool {
    requires(std::env::var(REQUIRE).ok().as_deref())
}

/// The reading of [`REQUIRE`], apart from the environment so that a test can
/// say what each setting means without setting anything.
pub fn requires(set_to: Option<&str>) -> bool {
    set_to == Some("require")
}

/// Whether System Events will say what is on the screen, and why not when it
/// will not.
///
/// The verdict is read off the screen, so the backend needs Accessibility
/// permission for whatever runs the tests. Without it System Events does not
/// refuse the question so much as answer nothing to it: no windows, for any
/// application, with no error — so Excel opens the package, draws its window,
/// and the watcher sits there for ninety seconds seeing nothing and calls it a
/// timeout. Every package then looks broken, and nothing says the permission
/// is what is missing.
///
/// Asked in the form that errors rather than the form that quietly answers
/// none, it says so, and the run says so in one second instead of ninety.
/// Only that refusal is read as one: any other trouble with System Events is
/// left to the watcher, which has a better view of it.
fn reads_the_screen() -> Option<String> {
    let asked = run(
        r#"tell application "System Events" to return (count of windows of every process whose visible is true) as text"#,
    );
    let Err(why) = asked else { return None };
    (why.contains("assistive access") || why.contains("-25211")).then(|| {
        format!(
            "whatever runs the tests has no Accessibility permission, and the verdict \
             is read off the screen. Grant it in System Settings, Privacy & \
             Security, Accessibility: {why}"
        )
    })
}

/// Get Excel into the state the oracle needs, or say why it cannot be had.
///
/// The state it needs is Excel not running, so that the package itself is
/// what launches it. That is the one route into Excel that works: told to
/// open a document while it is showing no window, Excel wedges — it stops
/// answering and no window ever appears — and only a kill gets it back.
///
/// So a running Excel is looked at rather than used. One with a workbook open,
/// or with any window at all, belongs to whoever is at the machine and the
/// oracle refuses rather than closing their work or taking their screen. One
/// showing nothing is nobody's, and is asked to quit, or killed if it is
/// already wedged and past asking.
fn readied() -> Option<String> {
    if !running() {
        return None;
    }
    let showing = windows().unwrap_or(1);
    match workbooks() {
        Ok(open) if open > 0 => Some(format!(
            "Excel already has {open} workbook(s) open, and the oracle will not close them"
        )),
        Ok(_) if showing > 0 => Some(format!(
            "Excel is open in front of someone, showing {showing} window(s), and the oracle \
             will not take the screen from them"
        )),
        Ok(_) => closed_down(None),
        Err(why) if showing == 0 => closed_down(Some(why)),
        Err(why) => Some(format!(
            "Excel is running and will not answer, and is showing {showing} window(s) that \
             someone may be in the middle of: {why}"
        )),
    }
}

/// Take an Excel that is showing nothing away, so that the launch which
/// follows starts a fresh one. `wedged` carries why it stopped answering,
/// when it had stopped: one past asking is killed rather than asked.
///
/// Nothing is lost either way. An Excel with unsaved work has a window
/// holding it, and this only ever runs against one with no window at all.
fn closed_down(wedged: Option<String>) -> Option<String> {
    if wedged.is_none() {
        let _ = tell("quit saving no");
        if gone() {
            return None;
        }
    }
    let _ = Command::new("/usr/bin/pkill")
        .args(["-9", "-f", "MacOS/Microsoft Excel"])
        .status();
    if gone() {
        return None;
    }
    Some(match wedged {
        Some(why) => format!("Excel is wedged and would not be killed: {why}"),
        None => "Excel would neither quit nor be killed".to_owned(),
    })
}

/// Wait for Excel to leave the process table, and say whether it did.
///
/// A moment is left after it goes. Leaving the process table is not the same
/// as LaunchServices having finished with it, and an open asked for in that
/// gap is one Excel never acts on.
fn gone() -> bool {
    for _ in 0..60 {
        if !running() {
            std::thread::sleep(SETTLE);
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    false
}

/// How many workbooks Excel has open.
fn workbooks() -> Result<u32, String> {
    let said = tell("set n to (count of workbooks)\nreturn n as text")?;
    said.trim()
        .parse()
        .map_err(|_| format!("Excel gave no number of workbooks: {}", said.trim()))
}

/// How many windows Excel is showing, read through System Events because a
/// wedged Excel will not answer for itself. `None` when System Events cannot
/// say either, which is not the same as none.
fn windows() -> Option<u32> {
    let said = run(r#"tell application "System Events"
	if not (exists process "Microsoft Excel") then return "0"
	return (count of windows of process "Microsoft Excel") as text
end tell"#)
    .ok()?;
    said.trim().parse().ok()
}

/// Whether Excel is running, asked of the process table rather than of Excel,
/// because asking Excel would launch it.
fn running() -> bool {
    Command::new("/usr/bin/pgrep")
        .args(["-f", "MacOS/Microsoft Excel"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Hand the package to Excel through LaunchServices, which grants Excel's
/// sandbox the file and launches Excel if it is not already up.
fn launched(package: &Path) -> Result<(), String> {
    let out = Command::new("/usr/bin/open")
        .arg("-a")
        .arg("Microsoft Excel")
        .arg(package)
        .output()
        .map_err(|err| format!("could not run /usr/bin/open: {err}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "/usr/bin/open refused the package: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// Watch Excel until it says something: a repair prompt, a window, or nothing
/// at all for long enough that there is no answer to be had.
fn watched(package: &Path) -> Verdict {
    let name = package
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let outcome = run(&watcher(&name));
    if std::env::var("XLSPLICE_ORACLE_TRACE").is_ok() {
        eprintln!("oracle: watcher for {name} said {outcome:?}");
    }
    match outcome {
        Err(why) => Verdict::Unavailable(why),
        Ok(said) => read(said.trim()),
    }
}

/// What the watching script's one line of output means.
fn read(said: &str) -> Verdict {
    match said.split_once(' ') {
        Some(("repair", _)) | Some(("repaired", _)) => Verdict::Repair,
        Some(("clean", _)) => Verdict::Clean,
        Some(("unavailable", why)) => Verdict::Unavailable(why.to_owned()),
        _ => Verdict::Unavailable(format!(
            "the oracle script said something unexpected: {said}"
        )),
    }
}

/// Put Excel away, whatever happened. A dialog is answered before the
/// workbooks are closed, because a modal one stops Excel acting on anything it
/// is told, and Excel is then quit rather than left sitting at no windows,
/// which is the state the next run must not open into.
fn tidied() {
    if !running() {
        return;
    }
    let _ = run(DISMISS);
    let _ = tell("close every workbook saving no");
    closed_down(None);
}

/// The script that watches Excel open the package.
///
/// It polls twice a second rather than waiting on the open, because the answer
/// may arrive as a modal dialog, which is not something the open would return.
/// Three things end the wait: the repair prompt, which is answered `No` so
/// that Excel recovers nothing and leaves nothing behind; the sandbox's own
/// file-open panel, which means Excel was never granted the file and there is
/// no verdict to be had; and the document window, whose title Excel suffixes
/// with `Repaired` when it opened a recovered copy.
fn watcher(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    format!(
        r#"
on run
	set stem to "{stem}"
	set seen to ""
	repeat with i from 1 to {ticks}
		delay 0.5
		tell application "System Events"
			if exists process "Microsoft Excel" then
				tell process "Microsoft Excel"
					repeat with w in windows
						set winName to name of w
						set sub to (value of attribute "AXSubrole" of w) as text
						set seen to "[" & winName & "] " & sub
						if sub is "AXDialog" then
							set said to ""
							repeat with t in static texts of w
								set said to said & ((value of t) as text)
							end repeat
							if said contains "We found a problem with some content" then
								click button "No" of w
								return "repair Excel offered to recover the workbook"
							end if
							if winName is "Open" then
								key code 53
								return "unavailable Excel was not granted the file and asked for it"
							end if
						else
							if winName contains "Repaired" then
								return "repair Excel opened a repaired copy: " & winName
							end if
							if winName contains stem then
								return "clean " & winName
							end if
						end if
					end repeat
				end tell
			end if
		end tell
	end repeat
	if seen is "" then
		set seen to "no window at all"
	end if
	return "unavailable Excel neither opened {name} nor said why within {seconds} seconds; last seen: " & seen
end run
"#,
        ticks = WAIT * 2,
        seconds = WAIT,
    )
}

/// Answer whatever dialog Excel is showing, so that the next thing told to it
/// is acted on rather than queued behind a modal window.
const DISMISS: &str = r#"
on run
	tell application "System Events"
		if not (exists process "Microsoft Excel") then return "none"
		tell process "Microsoft Excel"
			repeat with w in windows
				if ((value of attribute "AXSubrole" of w) as text) is "AXDialog" then
					try
						click button "No" of w
					on error
						key code 53
					end try
				end if
			end repeat
		end tell
	end tell
	return "done"
end run
"#;

/// Ask Excel something, with a timeout, so that a modal dialog nobody noticed
/// stalls one call rather than the suite.
///
/// An Excel that is not running is not asked. Addressing an application that
/// is not up launches it, and an Excel an Apple Event launched is the wedged
/// one this file opens by describing: the guard is what keeps the oracle from
/// creating the state it has to recover from.
fn tell(body: &str) -> Result<String, String> {
    if !running() {
        return Err("Excel is not running".to_owned());
    }
    run(&format!(
        "with timeout of 30 seconds\ntell application \"Microsoft Excel\"\n{body}\nend tell\nend timeout\n"
    ))
}

/// Run an AppleScript and give back what it printed.
fn run(script: &str) -> Result<String, String> {
    let out = Command::new("/usr/bin/osascript")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("a piped stdin is there to be taken")
                .write_all(script.as_bytes())?;
            child.wait_with_output()
        })
        .map_err(|err| format!("could not run osascript: {err}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
}
