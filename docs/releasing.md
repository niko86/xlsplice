# Releasing

Cutting a release is building the binary on each operating system that gets
one, packing it with the skill file and the licence, and attaching the
archives to a GitHub Release.

**The builds are done by hand, on the machines themselves.** Actions minutes on
this private repository are limited and a macOS runner spends them ten times
over, so `release.yml` no longer runs on a tag: it is there for when a
platform's machine is not to hand, and it has to be started deliberately. What
follows is the ordinary path.

Nothing here is clever. It is written down so that a release cut six months
from now is the same shape as this one.

## What an archive holds

One directory named for the version and target, holding the binary, `SKILL.md`
and `LICENSE`:

| Target                | Built on            | Archive                                           |
|-----------------------|---------------------|---------------------------------------------------|
| `aarch64-apple-darwin`| the Mac             | `xlsplice-vX.Y.Z-aarch64-apple-darwin.tar.gz`     |
| `x86_64-pc-windows-msvc` | the Windows machine | `xlsplice-vX.Y.Z-x86_64-pc-windows-msvc.zip`   |
| `x86_64-unknown-linux-gnu` | a container or a runner | `xlsplice-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |

The names are what `README.md` tells a reader to download, and what a caller
looking for a binary expects. They are not decorative.

## Before anything is built

1. **The version in `Cargo.toml` is the version being released.** The tag is
   that version with a `v` in front: `version = "0.1.0"` is tagged `v0.1.0`.
   If the version is being changed, change it, run `cargo check` so
   `Cargo.lock` follows, and commit both together — every build below uses
   `--locked`, which fails if the two disagree.

2. **The suite is green, and so is the oracle.** `cargo test` everywhere, and
   `scripts/oracle.sh "$XLSPLICE_CORPUS"` on the Mac, because a release is the
   one moment the Excel verdict is worth having fresh.

3. **Tag it and push the tag.**

   ```
   git tag -a v0.1.0 -m "xlsplice v0.1.0"
   git push origin v0.1.0
   ```

   No workflow runs when you do this. The tag is what every machine below
   checks out, so that three binaries are three builds of one commit rather
   than of whatever each machine happened to have.

## On the Mac

```
git fetch --tags && git checkout v0.1.0
cargo build --release --locked

name=xlsplice-v0.1.0-aarch64-apple-darwin
mkdir -p "staging/$name"
cp target/release/xlsplice SKILL.md LICENSE "staging/$name/"
tar -C staging -czf "$name.tar.gz" "$name"

./staging/$name/xlsplice version    # says the version being released
```

## On the Windows machine

PowerShell, from the root of a checkout. A Rust toolchain is needed:
`winget install --id Rustlang.Rustup`, then a fresh terminal so `cargo` is on
`PATH`.

```powershell
git fetch --tags
git checkout v0.1.0
cargo build --release --locked

$name = "xlsplice-v0.1.0-x86_64-pc-windows-msvc"
New-Item -ItemType Directory -Force -Path "staging\$name" | Out-Null
Copy-Item target\release\xlsplice.exe "staging\$name\"
Copy-Item SKILL.md, LICENSE "staging\$name\"
Compress-Archive -Path "staging\$name" -DestinationPath "$name.zip" -Force

& "staging\$name\xlsplice.exe" version
```

Copy the `.zip` back to whichever machine is publishing — a share, `scp`, or
`gh release upload` from the Windows machine itself once the release exists.

## Linux

There is no Linux machine here, so there are three honest options.

- **Don't ship one.** If nothing consumes it, its absence costs nothing. Say
  so in the release notes rather than leaving a reader wondering.
- **A container on the Mac**, if Docker is running:

  ```
  docker run --rm -v "$PWD:/src" -w /src rust:latest \
    cargo build --release --locked --target x86_64-unknown-linux-gnu
  ```

  then pack it the same way as the Mac archive, with the Linux target's name.
- **The workflow.** A Linux runner bills at 1×, which is the cheapest minute
  Actions sells. See below.

## Publishing

With the archives on one machine:

```
gh release create v0.1.0 --title "v0.1.0" --generate-notes xlsplice-v0.1.0-*
```

`--generate-notes` writes the notes from the commits since the last release,
which is why the commit messages in this repository are written the way they
are.

To add an archive to a release that already exists — the Windows zip arriving
after the fact, say:

```
gh release upload v0.1.0 xlsplice-v0.1.0-x86_64-pc-windows-msvc.zip
```

## Afterwards

- Download one archive the way `README.md` tells a reader to, unpack it, and
  run the binary. A release nobody has installed is a release nobody has
  tested.
- On macOS, a binary fetched with `gh` is not quarantined and one fetched
  through a browser is. `README.md` says so; check that it is still true.

## The workflow, when a machine is not to hand

`.github/workflows/release.yml` builds the same archives on GitHub's runners.
It runs only when started by hand, from the Actions tab or:

```
gh workflow run release.yml -f tag=v0.1.0 -f platforms=linux
```

`platforms` is `linux` or `all`. It checks out the tag, refuses to build if
the tag and `Cargo.toml` disagree, and attaches what it built to the release —
creating it if it does not exist yet, uploading to it if it does. So it can be
used for the whole release, or for the one platform whose machine is elsewhere.

Minutes are billed at 1× on Linux, 2× on Windows and 10× on macOS, against a
limited allowance on a private repository. That multiplier is the whole reason
the builds above are done by hand.
