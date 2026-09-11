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
- The reference implementation is `the reference implementation's directory`. Never open anything under its `a licence-sensitive decompile archive` (a licence-sensitive decompile archive), and never run anything against production the vendor system.
- GitHub Actions minutes on this private repository are limited. CI is one Linux `cargo test` job on push and pull request; release builds run only on a version tag; the Excel oracle suite never runs in CI.
- Read `CONTEXT.md` for vocabulary and `docs/adr/` for decisions before designing anything. The design record of 2026-09-11 is in `docs/handoffs/` and `docs/research/`.
