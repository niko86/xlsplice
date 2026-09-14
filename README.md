# xlsplice

A small command-line tool for surgical edits to Excel packages (`.xlsx`, `.xlsm`):
read and write cells by address or defined name, read and write custom document
properties, set the calculate-on-load flag, and report which parts of a package
differ. Every part it does not target stays byte-identical; there is no formula
engine and no whole-file re-serialisation.

```
xlsplice set --type number -- book.xlsx Inputs!A1 42
xlsplice props set --type date -- book.xlsx Run.At 2026-09-13
xlsplice calc --full-calc-on-load -- book.xlsx
xlsplice diff -- template.xlsx book.xlsx
```

`SKILL.md` is the whole of what an agent needs to drive it, and ships in every
release archive beside the binary. `xlsplice help json` and
`xlsplice help exit-codes` say the same things from the binary itself.

## Installing

### From a release

Each release carries one archive per target:

| Target                     | Archive                                              |
|----------------------------|------------------------------------------------------|
| macOS, Apple silicon       | `xlsplice-vX.Y.Z-aarch64-apple-darwin.tar.gz`        |
| Windows, x64               | `xlsplice-vX.Y.Z-x86_64-pc-windows-msvc.zip`         |
| Linux, x64                 | `xlsplice-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`    |

Each holds the binary, `SKILL.md` and the licence. Unpack it and put the binary
somewhere on `PATH`:

```
gh release download vX.Y.Z --repo niko86/xlsplice --pattern '*aarch64-apple-darwin*'
tar xzf xlsplice-vX.Y.Z-aarch64-apple-darwin.tar.gz
install xlsplice-vX.Y.Z-aarch64-apple-darwin/xlsplice ~/.local/bin/
```

macOS quarantines a binary downloaded through a browser; one fetched with `gh`
or `curl` is not quarantined. If Gatekeeper does object, clear it with
`xattr -d com.apple.quarantine ./xlsplice`.

### From source

```
cargo install --git https://github.com/niko86/xlsplice --locked
```

The fallback when there is no archive for the platform, and the way to install
from a branch. It needs a Rust toolchain; a release archive does not.

### Cutting one

`docs/releasing.md`. The binaries are built by hand on the machines themselves
rather than on runners, because Actions minutes are limited here and a macOS
runner spends them ten times over.

## Finding the binary from a caller

A caller that shells out — the Python hydration wrapper is the one this exists
for — looks for `xlsplice` on `PATH`, and **`XLSPLICE_BIN` overrides that**:
set it to the full path of the binary to use.

```
export XLSPLICE_BIN=/opt/xlsplice/bin/xlsplice
```

It is what lets a wrapper pin a version, or run against a build out of
`target/release`, without touching `PATH` for everything else on the machine.
Unset, the wrapper resolves `xlsplice` the way any other command is resolved.

## What it guarantees

- **Any non-zero exit means nothing was written.** A write goes to a temporary
  file and is renamed, so a command that was killed leaves the package it was
  pointed at exactly as it was.
- **A write that changes nothing writes nothing.** Re-running a hydration is
  safe and leaves the file's bytes alone.
- **`--json` is the stable interface.** The tab-separated human output is not,
  and may be laid out differently in a later version. Changes to the envelope
  are additive: ignore a field, or an `error.code`, you do not recognise.

`CONTEXT.md` carries the vocabulary and `docs/adr/` the decisions.

## Seeing it work

`scripts/demo.sh` hydrates a template start to finish with every command
shown: it reads the sheets and the input cells behind their defined names,
refuses a write over a formula, fills two cells and a date and stamps who ran
it in one batch, sets the recalculate flag, and then holds the result against
the template part by part. Nothing is written beside the package it reads —
the output goes to a temporary directory, and the path is printed for opening
in Excel.

```
scripts/demo.sh                      # the fixture committed here
scripts/demo.sh ~/templates/a.xlsm   # or a template of your own
```

## Testing

`cargo test` runs everywhere and needs nothing: the suites work over four
small fixtures committed to `tests/fixtures/`, and hold every write to the
byte-level guarantee with a comparator that reads both containers itself.

Two suites want more than a checkout, and both are off unless they are asked
for.

**The corpus.** Point `XLSPLICE_CORPUS` at a directory of real templates and
the same fixed operation set runs over every `.xlsx` and `.xlsm` under it. The
templates are vendor material: they are read, never written, and never enter
this repository. Each package is put through what it can take — a template
whose sheets start empty takes a row but no write over a cell — and the run
says how many cases each answered. Absent the variable, those cases skip and
the rest of the suite is unaffected.

```
XLSPLICE_CORPUS=~/templates cargo test --test corpus
```

**The oracle.** A real Excel, asked whether a package opens clean or demands a
repair. It drives the application through the screen, so it is ignored by
default and asked for by name, one case at a time:

```
scripts/oracle.sh                    # over the fixtures
scripts/oracle.sh ~/templates        # and over a corpus
```

The script is the way to run it. It exports what the suite reads — a variable
typed on a line of its own sets a shell variable the child process never sees,
which silently turns `require` off and makes a suite of skips look like a suite
of verdicts — checks the screen can be read before Excel is launched, and
passes `--nocapture` so the counts the suite prints are visible. Underneath it
is `cargo test --test oracle -- --ignored --test-threads=1`.

On a machine with no Excel each case skips and says why;
`XLSPLICE_ORACLE=require`, which the script exports, turns that skip into a
failure, for the machine the oracle is meant to run on. A case putting many
packages in front of Excel says how many of them Excel answered about, because
a skip is not a pass and a green run on its own does not tell you which it
was. The verdict is read off the screen, so it needs
Accessibility permission for the terminal the tests are started from — without
it every case skips saying so — and `XLSPLICE_ORACLE_TRACE=1` prints what Excel
was seen to do. See ADR-0006.

## Origin

Grilled out of the findings of an OfficeCLI trial in `the-reference-implementation` on
2026-09-10 (upstream bugs iOfficeAI/OfficeCLI #389, #390, #391) and that repo's
zip-level hydration layer, which is the reference implementation.
