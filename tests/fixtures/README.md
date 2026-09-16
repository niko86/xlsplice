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

## The one edit

Excel stamps the saving user into `docProps/core.xml` (`dc:creator`,
`cp:lastModifiedBy`) and the absolute path it saved to into
`xl/workbook.xml` (`x15ac:absPath`), and both ship with every release.
On 2026-09-15 those two parts were edited once in each fixture to take
them out. Nothing else was touched: every other entry is Excel's, copied
raw -- header, compressed bytes and central-directory record -- with only
its offset moved, so the flags, the creator system and the zeroed
timestamp that the writer's raw copy is tested against are still the ones
Excel wrote. The two edited parts were compressed afresh under the
original entry's own flags, version and timestamp. That edit is not a
re-save and does not move the baseline; a future fixture avoids the need
for it by clearing the author fields before saving, as `save-fixtures.sh`
says.

## Checksums

If one of these ever changes, a fixture was re-saved and the baseline
moved. That is a bug, not an update.

```
ba3fd96d84945433f77bc9cbdb06a4e57e5cf63925eac42939446dca86320bf3 plain.xlsx
ad889ab8ea3abd29855c255bd6e9f2156ed87f3f7cf2cf24f1b8ff158eedc767 macros.xlsm
d91f56ac18ac05cc5dd39bf6ff2e291665e13ef408640e66df936a37d36d27ea feature.xlsx
d9dabf4b978f4e6bf8e44d48b483294552a5151ede2678fe41fc7c5a86ce78e1 dated-row.xlsx
```

## The oracle

`tests/oracle.rs` puts these in front of a real Excel and asks whether it
opens them without complaint. Those cases are ignored by default, and the
way to run them is:

```
scripts/oracle.sh
```

One at a time on the Mac, because there is one Excel and it takes the
screen, so the cases have to take turns at it; on Windows each case gets its
own invisible one and the flag costs nothing. Run them through the script
rather than by hand: it exports what the suite reads, so a suite of skips
cannot pass itself off as a suite of verdicts. See
`docs/adr/0006-the-oracle-hands-excel-the-file-and-reads-the-screen.md` for
what that costs and why.
