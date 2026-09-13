# Operations own their reading, and the package memoises a part's text

The batch resolved every operation to a cell before anything else ran, parsed
each touched part as a worksheet, and rebuilt the container from the parts it
already held. So no operation could touch a part holding no cell, and none
could create or remove one: `Parts { added, removed }` was declared and
permanently empty, and #10 (remove `calcChain.xml`), #11 (a flag in
`xl/workbook.xml`, no cell) and #12 (create a part) were impossible rather than
awkward.

So an operation is handed the package and answers with the **part edits** it
wants: a splice of a part's bytes, the creation of a part, or its removal, each
named by part path. The batch runs over parts rather than over resolved cells,
merging the edits that land on each, and resolves a cell only for an operation
that names one. An operation reads whatever parts it needs, and `Package`
memoises the text of a part it has read, so an operation reading a part another
operation has already read costs nothing.

A part two operations both name is therefore parsed twice: once by each of
them. That is accepted, on the size argument ADR-0002 already makes — a part is
held whole in memory and a worksheet is a few hundred kilobytes — and it is
recorded here so that a later review does not re-suggest a shared parse. What
would have to be shared is not the text, which is, but the tree: roxmltree
borrows the text it parsed, so handing a parsed document from one operation to
the next means a lifetime in the operation's signature, in exchange for the
parse of a part that is already in memory.

## Considered options

- Keep resolving to cells first and special-case the operations that have none:
  rejected, it makes the two kinds of operation different shapes, and every
  later operation has to be classified as one or the other.
- Give the batch a parsed-document cache beside the text: rejected for now, it
  ties an operation's signature to the lifetime of a borrowed tree to save a
  parse whose cost the sizes make negligible. If a corpus package ever makes
  the parse measurable, this is the change to make, and nothing about the
  part-edit vocabulary has to move for it.
- Have the batch read parts and hand each operation the text it asked for:
  rejected, the batch would have to know what each operation reads before the
  operation runs, which is the coupling this removes.

## Consequences

- `PartEdit::Create` and `PartEdit::Remove` exist and `Package::rebuild`
  honours both: a created part is appended at the zip epoch of 1980-01-01, so a
  batch run twice produces the same bytes, and a removed part is left out.
  Nothing produces either yet; #12 and #10 are the first callers.
- An operation may name no cell, so a report's `address` is null for one that
  does not, which is an additive change to the envelope.
- The edits on one part must agree about what is being done to it: splices
  merge, removals merge, and two creations merge only if they carry the same
  bytes. Anything else is `internal`, like `splice::apply`'s overlap guard, and
  for the same reason: it is a fault in whatever built the batch.
- Resolution moved out of the read verb's module into `src/target.rs`, which
  both paths use, so `src/cells.rs` exports nothing to the write path.
- The memo holds the text of every part read for as long as the package is
  open, where before each part's text was dropped when the caller was done
  with it. A batch over three sheets therefore holds three sheets, the
  workbook part and two relationship parts at once; a read of a shared-string
  cell also holds the string table. That is the same order as the one part
  ADR-0002 already accepts holding whole, and it is bounded by the package.
- Each operation is asked in turn, so a batch with two faults reports the
  first operation's rather than the first fault of a particular kind. Before,
  every value was read before any target was looked up, so a mistyped value
  in a later operation beat a bad target in an earlier one. Nothing is
  written either way.
