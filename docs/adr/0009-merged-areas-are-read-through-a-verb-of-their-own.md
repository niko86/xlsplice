# Merged areas are read through a verb of their own

A caller reading values laid along a row needs to know which cells sit under a
**merged area**, because what each area shows is held in its **anchor** and
stepping a cell at a time reads the blanks beside it (#52). Until now it had
to open the package and parse `<mergeCells>` itself. `merged` reads one or
more sheets and answers with each sheet's merged areas, in part order, each
with its anchor as an address `get` accepts.

ADR-0008 put the extent in `cells` because `cells` had already parsed the
sheet, and the same argument would put the merged areas there. It is turned
down because the two ask about different halves of the part: `cells` about
the sheet data, `merged` about the worksheet envelope. A caller that reads by
`get` and defined names never calls `cells`, and one that wants only the
areas should not resolve shared strings for them, nor be refused them
because some cell elsewhere on the sheet is unreadable.

## Considered options

- **An `areas` field on `cells`**: rejected, for the reason above.
- **A `merged` field on each cell `get` answers**: left out. Every `get`
  would look through the envelope as well as at the cell, and no caller has
  asked for it. It is additive, so it can come later without a schema bump.
- **A streaming read that stops after `<mergeCells>`**: rejected.
  `<mergeCells>` follows `<sheetData>`, so the stop skips almost nothing, and
  it would be a second way of reading a worksheet beside the whole-part parse
  ADR-0005 accepts.
- **Areas as bare strings**: rejected. A list of strings cannot gain a field
  without retyping, which is a schema bump; a list of objects can.

## Consequences

- An area is reported as the part spells it, in upper case and without `$`.
  A reversed range and overlapping areas are reported, not refused: the verb
  says what the part declares and does not judge it. The anchor is worked
  out, the least column and the least row, so it is right for a reversed
  range too.
- A `ref` that cannot be read, or a `<mergeCell>` carrying none, fails the
  whole read as `unreadable`, as one unreadable cell fails `cells`.
- The `count` on `<mergeCells>` is ignored, being advisory in the way
  `<dimension>` is.
