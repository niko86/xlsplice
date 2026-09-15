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

/// Where Excel is, as a platform names it.
///
/// Not the same kind of thing on the two. A Mac has one Excel and it is an
/// application bundle at a known path, so absence is a question about the
/// filesystem. Windows has no path worth checking: what the backend reaches
/// for is a COM class the installer registered, and absence is that class not
/// answering — which is also how the Store build of Excel looks, having no COM
/// interface at all.
///
/// So this carries whatever the platform's backend asks for, and a test
/// simulating a machine without Excel names one that really is not there
/// rather than a flag that says to pretend. The real absence check then runs,
/// on both, which is the point: a simulated absence that skipped the check
/// would be testing the pretence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Where(String);

impl Where {
    /// Where this platform keeps Excel.
    pub fn installed() -> Self {
        Where(INSTALLED.to_owned())
    }

    /// Somewhere Excel is not. Only a test simulating a machine without one
    /// has any business asking for this.
    pub fn nowhere() -> Self {
        Where(NOWHERE.to_owned())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(target_os = "macos")]
const INSTALLED: &str = "/Applications/Microsoft Excel.app";
#[cfg(target_os = "macos")]
const NOWHERE: &str = "/Applications/No Such Excel.app";

/// The program id Excel registers. A desktop install answers to it; the Store
/// build does not, having no COM interface, and neither does a machine with no
/// Excel — which is why one reading covers both.
#[cfg(target_os = "windows")]
const INSTALLED: &str = "Excel.Application";
#[cfg(target_os = "windows")]
const NOWHERE: &str = "Excel.NoSuchApplication";

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const INSTALLED: &str = "nowhere this build knows to look";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const NOWHERE: &str = "nowhere this build knows to look";

/// The package the Windows backend opens first, to prove that Excel and the
/// session are working before it reads anything into the subject failing.
/// A fixture Excel saved, and one the suite already asserts opens clean.
#[cfg(target_os = "windows")]
const CONTROL: &str = "plain.xlsx";

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
    opened_by(&Where::installed(), package)
}

/// The oracle, told where Excel is. Only [`opened`] and a test simulating a
/// machine without Excel have any business saying.
///
/// One of these is compiled, and which one is the whole of the platform
/// split. Everything above it — the three answers, the skip-or-require
/// policy, and [`read`], which turns one line into a verdict — is shared, and
/// so is every case that asks.
#[cfg(target_os = "macos")]
pub fn opened_by(at: &Where, package: &Path) -> Verdict {
    let excel = Path::new(at.as_str());
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

/// The Windows backend: Excel through COM, and the verdict is whether a
/// workbook came back.
///
/// Almost nothing the Mac has to do applies. Excel is not launched by the
/// document and is never asked to show anything, so there is no window to
/// wait for, no dialog to dismiss, no screen to read and no Accessibility
/// permission to hold. With `DisplayAlerts` off Excel does not silently repair
/// a package and does not log having done so — it refuses: `Workbooks.Open`
/// throws, and no workbook appears. That is the whole signal, and four runs of
/// a probe on the lab machine on 2026-09-14 are what established it. The probe
/// was `scripts/windows-probe.ps1`; it was deleted once this backend was
/// signed off against a real Excel, and what it read is in the comments on #15.
///
/// **The control is load-bearing.** Every failure of `Open` carries the same
/// `0x800A03EC` and the same exception type, whether the package needs repair,
/// is not there, or is not a package at all. So a throw on its own does not
/// mean "Excel objected to this package"; it means "that open failed". A
/// backend reading a bare throw as [`Verdict::Repair`] would report a locked
/// file or a bad path as a package Excel refused — the oracle *lying* rather
/// than skipping, which is worse than having no oracle. So a package known to
/// be good is opened first, in the same session, and a verdict is only read
/// out of the subject once the control has proved that Excel and the session
/// are working. ADR-0006 records the Mac's version of this mistake from the
/// other side: there, a probe asked a broader question than the suite did.
///
/// Excel's own message is not read, though it would discriminate: Excel
/// describes the failures that are not repairs and says nothing about the one
/// that is. That decides by absence, so any failure Excel also declines to
/// describe would read as a repair, and the descriptions are localised, so it
/// would work here and not on an Excel in another language. It is carried in
/// the answer for diagnosis and decides nothing.
///
/// One Excel per package, started and quit each time. The probe showed a
/// session surviving three refusals intact, so one Excel could serve the whole
/// suite; that is an optimisation with state to own and a cleanup to get right,
/// and at hundredths of a second an open it is not needed yet.
///
/// One per package is also what lets the cases run alongside each other here.
/// Nothing is shared between them, none of them takes the screen, and the lab
/// machine ran all twenty-two on 2026-09-14 serialised and then again in
/// parallel, to the same verdicts. The Mac's `--test-threads=1` is a real
/// constraint there and habit here.
#[cfg(target_os = "windows")]
pub fn opened_by(at: &Where, package: &Path) -> Verdict {
    // Before Excel is troubled at all. A package that cannot be read is a
    // fault in the harness rather than a verdict about a package, and it is
    // exactly the confusion the control exists to prevent — caught here more
    // cheaply and said more plainly.
    if let Err(err) = std::fs::File::open(package) {
        return Verdict::Unavailable(format!(
            "the package cannot be read, so there is nothing to ask Excel about: {} ({err})",
            package.display()
        ));
    }
    // The control is copied out of the fixtures rather than opened where it
    // lives: a fixture's bytes are the baseline every byte-preservation test
    // compares against, and Excel writes an owner file beside a workbook it
    // opens. The workspace takes the copy away when this returns.
    let workspace = super::workspace::Workspace::new("oracle-control");
    let control = workspace.copy_of(CONTROL);

    match powershell(&asking(at, &control, package)) {
        Err(why) => Verdict::Unavailable(format!("could not ask Excel: {why}")),
        Ok(said) => read(said.trim()),
    }
}

/// No backend, and the suites say so rather than answering.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn opened_by(_at: &Where, _package: &Path) -> Verdict {
    Verdict::Unavailable("there is no oracle backend for this platform".to_owned())
}

/// The script that opens the control and then the package, and prints the one
/// line [`read`] turns into a verdict — the same line the Mac's watcher
/// prints, so one parser serves both.
#[cfg(target_os = "windows")]
fn asking(at: &Where, control: &Path, package: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
try {{
    $excel = New-Object -ComObject {program}
}} catch {{
    Write-Output "unavailable Excel is not installed, or is the Store build, which has no COM interface: $($_.Exception.Message)"
    exit
}}
$excel.Visible = $false
$excel.DisplayAlerts = $false
$excel.AskToUpdateLinks = $false
$said = ''
try {{
    $control = $null
    try {{ $control = $excel.Workbooks.Open('{control}') }} catch {{ }}
    if (-not $control) {{
        $said = 'unavailable the control package did not open, so it is Excel or this session that is wrong rather than the package under test'
    }} else {{
        $control.Close($false)
        $opened = $null
        $why = ''
        try {{ $opened = $excel.Workbooks.Open('{package}') }} catch {{ $why = $_.Exception.Message }}
        if ($opened) {{
            $said = 'clean ' + $opened.Name
            $opened.Close($false)
        }} else {{
            $said = 'repair ' + $why
        }}
    }}
}} finally {{
    try {{ $excel.Quit() }} catch {{ }}
    try {{ [System.Runtime.InteropServices.Marshal]::ReleaseComObject($excel) | Out-Null }} catch {{ }}
}}
Write-Output $said
"#,
        program = at.as_str(),
        control = single_quoted(control),
        package = single_quoted(package),
    )
}

/// A path as the body of a PowerShell single-quoted string, where the only
/// character with a meaning is the quote itself and it is escaped by doubling.
///
/// Single quotes rather than double, because a single-quoted string
/// interpolates nothing: a corpus template is called `PSD ISO Input
/// [v000012].xlsm`, and a package could as easily hold a `$`.
#[cfg(target_os = "windows")]
fn single_quoted(path: &Path) -> String {
    path.display().to_string().replace('\'', "''")
}

/// Run a PowerShell script and give back what it printed.
///
/// `-NoProfile` because a profile is the machine's and not the suite's, and
/// one that prints would be read as the answer.
#[cfg(target_os = "windows")]
fn powershell(script: &str) -> Result<String, String> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|err| format!("could not run powershell: {err}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
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
            if unheard(&why) {
                eprintln!("skipping: the oracle could not answer: {why}");
            }
            None
        }
        answered => Some(answered),
    }
}

/// Whether this is the first time a run has been told `why`.
///
/// One case may ask the oracle fifty times, and fifty identical paragraphs
/// about a missing permission bury the line that says how many packages Excel
/// actually saw. Each distinct reason is worth reading once; the same one
/// fifty times is not.
fn unheard(why: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static HEARD: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    HEARD
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("the set of reasons already printed")
        .insert(why.to_owned())
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
///
/// Asked, too, in the shape the watcher uses: one named process, its windows.
/// The first version of this asked about the windows of *every* visible
/// process, and on 2026-09-14 that shape was refused on a machine where every
/// shape the suite actually uses was allowed — one process in the enumeration
/// refusing is enough to fail the whole question. A probe that is stricter
/// than what it stands in for does not report a blocked suite, it blocks one.
fn reads_the_screen() -> Option<String> {
    let asked = run(r#"tell application "System Events"
	if not (exists process "Finder") then return "no Finder to ask about"
	return (count of windows of process "Finder") as text
end tell"#);
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
        // Behind whatever is in front, so a run of any length can be left
        // alone. The window is still drawn and still read: what `-g` withholds
        // is activation, not the screen. Nothing here may type, then — see the
        // watcher — because a keystroke goes to the front application, which
        // during a backgrounded run is somebody's work.
        .arg("-g")
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
    // Excel will not show a square bracket in a window title, because a
    // bracket is how a reference names a workbook: `X [v1].xlsm` is titled
    // `X (v1)`. So the name watched for is the one Excel will show rather than
    // the one on disk. Every corpus template is versioned this way, which is
    // how this was found: Excel opened the package, drew its window, and the
    // watcher looked straight past it for ninety seconds.
    let stem = stem.replace('[', "(").replace(']', ")");
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
								try
									click button "Cancel" of w
								end try
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
						try
							click button "Cancel" of w
						end try
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
