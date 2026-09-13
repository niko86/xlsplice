# The oracle hands Excel the file through LaunchServices and reads its answer off the screen

The oracle has to do two things: get a package in front of a real Excel, and
find out what Excel made of it. Neither goes the way the scripting dictionary
suggests, and both were settled by trying it on this Mac on 2026-09-13.

**Excel is handed the file, never told to open it.** Excel is sandboxed, so it
may only read a file something has granted it. Told to `open` a path over an
Apple Event it puts up the sandbox's own file-open panel and waits for a human
to pick the file, which arrives at the caller as `AppleEvent timed out (-1712)`
with nothing said about why. `/usr/bin/open -a "Microsoft Excel" FILE` goes
through LaunchServices, which grants Excel the file the way a double click
does, and Excel opens it. So the package is always handed over that way, and
Apple Events are only ever used to ask questions afterwards.

**Excel is never launched by an Apple Event, and never opened into at no
windows.** Addressing an application that is not running launches it, and an
Excel that an Apple Event launched comes up with no window. From then on it
answers questions cheerfully and silently does nothing when told to open or
close a workbook — no error, no window, `count of workbooks` stuck at zero.
The same state is reached by opening a document into an Excel that is running
and showing no window, which is what an Excel that has just had its last
workbook closed is. Only a kill gets it back. So the oracle starts every run
from an Excel that is not running, lets the package itself be the launch, and
quits Excel afterwards rather than leaving one sitting at no windows. A wait
follows the quit: leaving the process table is not the same as LaunchServices
having finished, and an open asked for in that gap is one Excel never acts on.

**The verdict is read off the screen, because Excel does not say it.** A
package Excel had to repair opens as an ordinary workbook: `name of active
workbook` is the file's name and nothing in the scripting interface reports the
repair. What does report it is the screen — a modal dialog, *We found a problem
with some content in 'NAME'. Do you want us to try to recover as much as we
can?*, with Yes and No; and afterwards the document window's title, which Excel
suffixes with `Repaired`. Both are read through System Events, and the dialog
is answered `No` so that Excel recovers nothing, writes nothing and leaves
nothing on screen.

## Consequences

- Every oracle case is a cold Excel: a few seconds when the machine is warm,
  twenty or so when it is not. That is the cost of the only sequence that
  works, and it is why the suite is `#[ignore]`d and asked for by name.
- The oracle refuses an Excel with a workbook open, or with any window at all,
  rather than closing someone's work or taking their screen. An Excel showing
  nothing is nobody's, and is quit, or killed if it is already wedged.
- Reading the screen means the backend needs Accessibility permission for
  whatever runs the tests — the terminal the tests were started from, not the
  test binary. Without it System Events reports no windows for any
  application, so Excel opens the package, draws its window, and the watcher
  sees nothing for ninety seconds and calls it a timeout: every package looks
  broken and nothing says why. So the permission is asked about before Excel
  is launched, in the form that errors rather than the form that quietly
  answers none, and its absence is an `Unavailable` naming what to grant. A
  grant the list shows as enabled can still deny — an operating system update
  on 2026-09-13 left one looking on and refusing every request, through a
  reboot and a re-grant — so the probe is what says whether the oracle can
  answer, never the checkbox, and `UI elements enabled` is no better than the
  checkbox: on 2026-09-14 it answered `true` while every call denied. What is
  refused is narrow, which is worth knowing when diagnosing one: Apple Events
  are not blocked at all, and System Events will answer how many processes
  there are while refusing to say how many windows they have. What has gone
  stale is a row in the permission database, and the list edits that row
  rather than removing it; `tccutil reset Accessibility <bundle id>` removes
  it, so the next call asks again rather than reading what is there.
- A Windows backend (#15) answers the same question and will not answer it this
  way: nothing here about sandboxes, LaunchServices or System Events crosses
  over, which is why the interface is one operation returning clean, repair or
  unavailable and nothing more.
