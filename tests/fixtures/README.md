# Fixtures

Three packages saved from Excel by hand for issue #4. A fixture is a
package **Excel saved**: never re-save one, never regenerate one with a
library, and never put vendor material in one. Their bytes are the
baseline every byte-preservation test compares against.

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

Saved with Excel 16.x on 2026-09-11.

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
```

## The oracle

`tests/oracle.rs` puts these in front of a real Excel and asks whether it
opens them without complaint. Those cases are ignored by default:

```
cargo test --test oracle -- --ignored --test-threads=1
```

One at a time, because there is one Excel and they take turns at it. See
`docs/adr/0006-the-oracle-hands-excel-the-file-and-reads-the-screen.md` for
what that costs and why.
