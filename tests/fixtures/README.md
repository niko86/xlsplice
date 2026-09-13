# Fixtures

Four packages saved from Excel by hand: three for issue #4 and a fourth
for issue #28. A fixture is a package **Excel saved**: never re-save one,
never regenerate one with a library, and never put vendor material in
one. Their bytes are the baseline every byte-preservation test compares
against.

- `plain.xlsx` — one visible sheet: a few numbers, a text, a date
  formatted as a date, and a boolean.
- `macros.xlsm` — a real VBA project (`xl/vbaProject.bin`) holding one
  trivial macro, so the pass-through of the VBA part is exercised.
- `feature.xlsx` — a merged range with a workbook-scoped defined name on
  it; a sheet-scoped defined name; a plain formula and a shared formula
  filled across several cells; a hidden sheet and a very hidden sheet; a
  row with a custom row format and a column with a column style, both
  leaving their cells absent; and custom document properties of each of
  the four types Excel offers.
- `dated-row.xlsx` — one sheet, `Inputs`, whose row 7 carries a custom
  row format that is a **date** format, with `B7` left absent. A date
  written into `B7` renders as a date only by inheriting the style of
  its row, which is the half of inheritance no other fixture holds.

Saved with Excel 16.x — the first three on 2026-09-11, and
`dated-row.xlsx` on 2026-09-13.

Excel stamps the absolute path it saved to into `xl/workbook.xml`, as
`x15ac:absPath`. Taking it out would mean editing the file, which is the one
thing a fixture must not have had done to it, so it stays.

## Checksums

If one of these ever changes, a fixture was re-saved and the baseline
moved. That is a bug, not an update.

```
c81533de1755ccd027e4bf5abc37a94bd109b5170668d1e4a24ed6cf380e98f2 plain.xlsx
5d864236a1c3f4add7ad45f2f87f43f6b82e20b117865bc81aca39113b766d24 macros.xlsm
f4a0eac7179c7a18038938e28d8acf696697f4190a5bdd375913450f8f600480 feature.xlsx
198297690d843a94788886882d797a11641ea9878a1665fb42ff64aade933eb6 dated-row.xlsx
```

## The oracle

`tests/oracle.rs` puts these in front of a real Excel and asks whether it
opens them without complaint. Those cases are ignored by default, and the
way to run them is:

```
scripts/oracle.sh
```

One at a time, because there is one Excel and they take turns at it. Run
them that way rather than by hand: the script exports what the suite reads,
so a suite of skips cannot pass itself off as a suite of verdicts. See
`docs/adr/0006-the-oracle-hands-excel-the-file-and-reads-the-screen.md` for
what that costs and why.
