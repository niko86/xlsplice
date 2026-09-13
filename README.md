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

Every version tag publishes a GitHub Release carrying one archive per target:

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

## Origin

Grilled out of the findings of an OfficeCLI trial in `the-reference-implementation` on
2026-09-10 (upstream bugs iOfficeAI/OfficeCLI #389, #390, #391) and that repo's
zip-level hydration layer, which is the reference implementation.
