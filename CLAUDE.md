# xlsplice

## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues on `niko86/xlsplice`, driven through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default triage labels, unchanged: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` at the repo root plus `docs/adr/`. See `docs/agents/domain.md`.

## Constraints

- The test corpus is vendor material and lives outside this repository, in the directory named by `XLSPLICE_CORPUS`. Never commit a corpus file, and never copy one into the repository, not even under a gitignored path.
- The reference implementation lives outside this repository, in the directory named by `XLSPLICE_REFERENCE`. Parts of it are licence-sensitive: open nothing there that is not plainly its own source, and never run anything against a production system.
- This repository is public, so Actions minutes are free. CI is still one Linux `cargo test` job on push and pull request, because that is what the suite needs, not because of a billing limit; `release.yml` builds only when someone starts it by hand; the Excel oracle suite never runs in CI, because it drives a real Excel.
- Read `CONTEXT.md` for vocabulary and `docs/adr/` for decisions before designing anything. The design record of 2026-09-11 is in `docs/research/`.
