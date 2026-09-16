---
name: xlsplice
description: Read and write cells, custom document properties and the calculate-on-load flag in Excel packages (.xlsx, .xlsm) without re-serialising them. Use when filling in a template, stamping a package, or checking which parts a write touched.
---

# xlsplice

A command-line tool for surgical edits to Excel packages. Every part it is not
pointed at comes out byte-identical, so a template's formatting, formulas,
charts and VBA survive a write untouched. There is no formula engine: a
formula's cached value is what the package holds until Excel recalculates.

Every example below is written with `--` before the operands, because a
defined name may contain characters a shell or a parser would take for
something else, and a package path may start with `-`.

## The two output shapes

Without `--json`, a verb writes tab-separated rows to stdout, one per thing it
answered about, with no header. With `--json`, it writes exactly one JSON
document and nothing else. **`--json` is the stable interface**; the rows are
not, and may be laid out differently in a later version.

```
xlsplice help json
xlsplice help exit-codes
xlsplice version --json
```

## Naming a cell

A target is a cell address or a defined name:

- `Inputs!A1` — a cell on the sheet called `Inputs`. The sheet name is matched
  without regard to case; it comes back in the package's own spelling.
- `MergedInput` — a workbook-scoped defined name.
- `Notes!LocalNote` — a defined name scoped to one sheet.

A defined name resolves to its **anchor**: the top-left cell of the first area
it refers to. A name that refers to no cell is not found.

## Reading

```
xlsplice sheets -- book.xlsx
xlsplice names -- book.xlsx
xlsplice get -- book.xlsx Inputs!A1 MergedInput
xlsplice props get -- book.xlsx
xlsplice calc -- book.xlsx
xlsplice diff -- book.xlsx other.xlsx
```

`get` answers one row per target, in the order given. A cell the sheet does
not hold reads as `empty` rather than as a failure.

`diff` compares two packages part by part and exits 0 whether or not they
differ; `--exit-code` makes a difference exit 1, as diff(1) does.

## Writing

```
xlsplice set --type number -- book.xlsx Inputs!A1 42
xlsplice set --type text -- book.xlsx Inputs!A2 hello
xlsplice set --type bool -- book.xlsx Inputs!A3 true
xlsplice set --type date -- book.xlsx Inputs!A4 2026-09-13
xlsplice clear -- book.xlsx Inputs!A1
xlsplice props set --type text -- book.xlsx Stamp.Text hydrated
xlsplice props unset -- book.xlsx Stamp.Flag
xlsplice calc --full-calc-on-load -- book.xlsx
```

Every writing verb takes:

- `--out PATH` — write the result there and leave the package alone.
- `--dry-run` — do everything, write nothing, and report what would have
  changed.

A write to a cell the sheet does not hold puts the cell in, and the row it
sits in with it. A write to a cell holding a formula is refused unless
`--replace-formula` says otherwise; a shared formula's master is refused even
then, because overwriting it orphans the rest of its range.

A shared formula holds the cells that carry its group, which `get` reports as
`group`. The master's `range` is advisory and is reported as stored: it may name
cells that are in no group, both in templates Excel wrote and after a licensed
write over a child. Read `group` to know what a shared formula holds, never
`range`.

A write of the value already there changes nothing, reports `"changed": false`
and leaves the file untouched, so re-running a hydration is safe.

Set the calculate-on-load flag after a hydration. The cached values a package
carries are the ones Excel last worked out, and nothing here recalculates
them.

## A batch

`apply` runs a list of operations over one package, all of them or none. It is
validated whole before a byte is written, so a failure anywhere leaves the
package as it was.

```
xlsplice apply --json -- book.xlsx batch.json
```

`batch.json` is a JSON array. Every value is given as text under the type that
says how to read it, so the batch carries what you wrote rather than what JSON
made of it:

```json
[
  {"op": "set", "target": "Inputs!A1", "type": "number", "value": "42"},
  {"op": "set", "target": "Inputs!A2", "type": "text", "value": "hello"},
  {"op": "clear", "target": "Inputs!A3"},
  {"op": "props.set", "name": "Run.At", "type": "date", "value": "2026-09-13"},
  {"op": "props.unset", "name": "Draft"},
  {"op": "calc", "full_calc_on_load": true}
]
```

A `set` or a `clear` also takes `"replace_formula": true`.

Pass `-` instead of a path to read the batch from stdin.

A batch is a set of edits over the package as it was read, not a sequence over
a document that changes under it. One cell cannot be named twice, and neither
can one document property: the batch is refused whole rather than one write
silently winning.

## Exit codes

| Exit | `error.code` | Meaning                                            |
|------|--------------|----------------------------------------------------|
| 0    | —            | Success.                                           |
| 1    | `internal`   | An unexpected failure, including a caught panic.   |
| 2    | `usage`      | The command line or the batch was wrong.           |
| 3    | `not_found`  | A sheet, name, cell, property or part was not found. |
| 4    | `refused`    | A guard said no; some are licensed by a flag.      |
| 5    | `unreadable` | Not a package, or the package cannot be read.      |

Any non-zero code guarantees no package was written. `diff --exit-code` exits
1 for a difference, which is not a failure. Codes may be added, never removed
or renumbered: treat one you do not recognise as a failure and read
`error.message`.

## The envelope

```json
{"ok":true,"schema_version":1,"cells":[{"target":"Inputs!A1","value":42, "...": null}]}
{"ok":false,"schema_version":1,"error":{"code":"not_found","message":"..."}}
```

Read `ok`, not the presence of a field. Changes are additive: ignore a field
or an `error.code` you do not recognise, and do not depend on field order. A
field that does not apply is `null` rather than absent.
