# What Excel on Windows does, asked once, so that the Windows oracle backend
# (#15) is written against answers rather than guesses.
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows-probe.ps1
#
# Run it from the root of a checkout, with Excel installed and closed. It opens
# two packages through COM with alerts suppressed: one Excel saved, and one
# deliberately broken the way the macOS oracle breaks one — the worksheet's
# `dimension` element moved to after `sheetData`, which is wrong at the schema
# and nowhere else. Then it reports everything that might serve as the signal
# for "Excel had to repair this", because that is the one thing the macOS
# backend reads off the screen and Windows must read some other way.
#
# The run of 2026-09-14 answered the first question: with alerts suppressed,
# `Workbooks.Open` throws on a package that would demand repair and opens one
# that would not. What is left is whether that throw can be told apart from
# every other reason an open fails, so each package is now read for the
# exception's type and HRESULT as well, and two failures that are not repairs
# are put through the same probe for comparison.
#
# It writes only inside its own temporary directory. Nothing is written to the
# repository, and no fixture is opened for writing.

$ErrorActionPreference = 'Continue'

function Say([string]$text) { Write-Host $text }
function Head([string]$text) { Write-Host ""; Write-Host "== $text" }

Head "The machine"
Say "PowerShell   : $($PSVersionTable.PSVersion)"
Say "OS           : $([System.Environment]::OSVersion.VersionString)"
Say "64-bit       : $([System.Environment]::Is64BitOperatingSystem)"

if (-not (Test-Path 'tests\fixtures\plain.xlsx')) {
    Say "Run this from the root of the checkout: tests\fixtures\plain.xlsx is not here."
    exit 1
}

# ---------------------------------------------------------------- the packages
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("xlsplice-probe-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null
$good = Join-Path $work 'good.xlsx'
$broken = Join-Path $work 'broken.xlsx'
Copy-Item 'tests\fixtures\plain.xlsx' $good
Copy-Item 'tests\fixtures\plain.xlsx' $broken

Head "Breaking one of them"
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::Open($broken, 'Update')
    $entry = $zip.Entries | Where-Object { $_.FullName -eq 'xl/worksheets/sheet1.xml' }
    $reader = New-Object System.IO.StreamReader($entry.Open())
    $xml = $reader.ReadToEnd()
    $reader.Close()

    if ($xml -notmatch '(<dimension[^>]*/>)') { throw "no dimension element to move" }
    $dimension = $Matches[1]
    $moved = $xml.Replace($dimension, '').Replace('</sheetData>', "</sheetData>$dimension")
    if ($moved -eq $xml) { throw "the dimension element did not move" }

    $stream = $entry.Open()
    $stream.SetLength(0)
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.Write($moved)
    $writer.Flush()
    $writer.Close()
    $zip.Dispose()
    Say "broken.xlsx: dimension moved to after sheetData"
}
catch {
    Say "could not break the package, so only the good one is worth reading below: $_"
}

# ------------------------------------------------------------------- the probe
Head "Starting Excel through COM"
try {
    $excel = New-Object -ComObject Excel.Application
}
catch {
    Say "no COM Excel: $_"
    Say "If Excel is the Microsoft Store build, it has no COM interface and the"
    Say "oracle cannot drive it. A desktop install is what #15 needs."
    exit 1
}
$excel.Visible = $false
$excel.DisplayAlerts = $false
$excel.AskToUpdateLinks = $false
Say "Version      : $($excel.Version)"
Say "Build        : $($excel.Build)"
Say "Workbooks now: $($excel.Workbooks.Count)"

function Probe([string]$label, [string]$path) {
    Head "Opening $label"
    $before = Get-ChildItem -Path $work -File | Select-Object -ExpandProperty Name
    $was = $excel.Workbooks.Count
    $opened = $null
    $failed = $null
    # A stopwatch, not `Measure-Command`. That runs its block in a child scope,
    # so the first version of this assigned `$script:opened` in there and read
    # `$opened` out here — two different variables inside a function. Every
    # reading below was skipped, nothing was ever closed, and both packages
    # answered "no workbook object came back" whatever Excel had done with
    # them. The run of 2026-09-14 was thrown away for it.
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try { $opened = $excel.Workbooks.Open($path) }
    catch { $failed = $_ }
    $watch.Stop()
    Say "seconds      : $([math]::Round($watch.Elapsed.TotalSeconds, 2))"
    # Before and after, because whether a workbook appeared at all is itself a
    # candidate signal, and a count on its own cannot say that.
    Say "Workbooks    : $was before, $($excel.Workbooks.Count) after"
    if ($failed) {
        # The message is the same generic one for every COM failure of `Open`,
        # so it cannot say what went wrong. The HRESULT might, and that is what
        # decides whether a backend can read a throw as "repair" or has to
        # narrow it first.
        Say "threw        : $($failed.Exception.Message)"
        Say "  type       : $($failed.Exception.GetType().FullName)"
        Say "  HResult    : 0x$('{0:X8}' -f $failed.Exception.HResult)"
        if ($failed.Exception.InnerException) {
            Say "  inner      : $($failed.Exception.InnerException.Message)"
            Say "  inner type : $($failed.Exception.InnerException.GetType().FullName)"
            Say "  inner H    : 0x$('{0:X8}' -f $failed.Exception.InnerException.HResult)"
        }
    }
    if ($opened) {
        Say "Name         : $($opened.Name)"
        Say "FullName     : $($opened.FullName)"
        Say "Saved        : $($opened.Saved)"
        Say "ReadOnly     : $($opened.ReadOnly)"
        Say "Sheets       : $($opened.Sheets.Count)"
        Say "A1           : $($opened.Sheets.Item(1).Range('A1').Value2)"
        try { $opened.Close($false) } catch { Say "close threw  : $_" }
    }
    else {
        Say "no workbook object came back"
    }
    # A repair leaves traces on Windows that it does not leave on a Mac: a log
    # beside the file, or a renamed recovery. Whatever appears here is a
    # candidate for the signal the backend reads.
    $after = Get-ChildItem -Path $work -File | Select-Object -ExpandProperty Name
    $new = $after | Where-Object { $before -notcontains $_ }
    if ($new) { Say "new files    : $($new -join ', ')" } else { Say "new files    : none" }
}

Probe "the package Excel saved" $good
if (Test-Path $broken) { Probe "the package broken on purpose" $broken }

# Two failures that are not repairs. A broken package throws, and so does a
# package that is not there and one that is not a package at all; if all three
# throw the same thing, a backend cannot read a bare throw as "Excel demanded
# repair" and must narrow it — otherwise a locked file or a bad path would be
# reported as a package Excel objected to, which is the oracle lying rather
# than skipping.
Probe "a file that is not there" (Join-Path $work 'no-such-file.xlsx')
$garbage = Join-Path $work 'not-a-package.xlsx'
Set-Content -Path $garbage -Value 'this is not a package' -Encoding Ascii
Probe "a file that is not a package at all" $garbage

# And the good one again, last of all. Whether a session survives a refusal is
# what decides the shape of the backend: one Excel for the whole suite, or one
# per package. Each open here costs hundredths of a second, so a session that
# survives puts 22 cases in a few seconds against the ten minutes the macOS
# backend takes; a session that does not means a launch and a quit per package,
# and most of that time back.
Probe "the package Excel saved, again, after three refusals" $good

Head "Anything Excel logged about a repair"
foreach ($where in @($work, [Environment]::GetFolderPath('MyDocuments'), $env:TEMP)) {
    $logs = Get-ChildItem -Path $where -Filter '*epair*' -File -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt (Get-Date).AddMinutes(-10) }
    foreach ($log in $logs) { Say "$($log.FullName)  ($($log.LastWriteTime))" }
}

Head "Leaving Excel as it was found"
try {
    $excel.Quit()
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($excel) | Out-Null
    Say "Excel quit"
}
catch { Say "Excel would not quit: $_" }
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue

Write-Host ""
Say "Paste everything above into issue #15."
