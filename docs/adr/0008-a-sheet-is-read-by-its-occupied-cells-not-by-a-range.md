# A sheet is read by its occupied cells, through a verb of its own

A caller scanning a sheet for its layout needs every cell the sheet holds, and
until now could only list addresses to `get` and guess how far the sheet
went (#50). Two answers were on the table that this one turns down: range
targets in `get`, and a dimension in `sheets`. Instead, `cells` reads one or
more sheets and answers with each sheet's **occupied cells**, in part order,
and the **extent** they cover.

The deciding point is that `get` and `cells` ask different questions. `get`
answers about the cells a caller named: one row per target, in the order
given, and a cell the sheet does not hold is a row reading `empty`, because
the caller asked about it. `cells` answers about what a part holds, and a
cell that holds nothing is no row at all. Putting both behind one verb would
make what a row means depend on a flag or on the shape of an operand.

## Considered options

- **Range targets in `get`** (`AGS!A1:AF44`): rejected. A target is one cell
  on both paths, since reads and writes resolve through `src/target.rs`, so
  widening it raises `set AGS!A1:C3` too. It also leaves the caller guessing
  the extent, which was the complaint, and it pads with `empty` rows for every
  absent cell in the rectangle. `get` still refuses a range, and its message
  points at `cells`.
- **A whole-sheet flag on `get`** (`get --sheet AGS`): rejected. Under the
  flag a row would mean "what the part holds", and without it "what was asked
  about".
- **The dimension in `sheets`**: rejected. `<dimension ref>` is advisory in
  the same way a shared formula's range is: other writers leave it absent or
  stale. Computing an extent instead would turn a read of the workbook part
  into a parse of every worksheet just to list the sheets. The extent goes in
  `cells`, which has already parsed the sheet, and it is computed from the
  cells it reports, so it never disagrees with them.
- **Every `<c>` element, style-only ones included**: rejected as the default.
  A template is full of formatted, empty cells, and the caller asked to be
  spared them. A formula Excel never calculated stays in, because under
  ADR-0007 `raw: null` on a formula cell is information.

## Consequences

- A row in `cells` has `get`'s fields under `get`'s key names, without
  `target` and `name`, so one parser reads both verbs' cells.
- One unreadable cell fails the whole read, as it does in `get`: across
  hundreds of packages, a refused file a caller can see beats a hole it
  cannot.
- A sheet whose part is not a worksheet, such as a chartsheet, is refused by
  every verb, `cells` among them, rather than reported as unreadable or
  answered with no cells.
