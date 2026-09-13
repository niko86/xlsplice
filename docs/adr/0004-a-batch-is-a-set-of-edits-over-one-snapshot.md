# A batch is a set of edits over one snapshot, so one cell twice is refused

A batch names the same cell twice. Is that a first write and then a second, or
a caller contradicting itself? The answer follows from what a batch already is
here. ADR-0002 parses each part once, and every operation computes its edits
against that one text, so there is no moment at which the second operation
could see what the first did: the two are answers to one question, not steps in
a sequence. And a `set` asserts nothing about what it expects to find, so
xlsplice cannot tell a deliberate overwrite from a mis-addressed write. So the
batch is refused whole, with exit 2, during its own validation — on the cell
each operation resolved to, before any part is read — naming both operation
indices and the canonical address.

The research note of 2026-09-12,
`docs/research/2026-09-12-two-operations-on-one-target.md`, names the axis the
sources divide on: formats where an operation can assert prior state (RFC
6902's `test`, a diff hunk's context lines) define an order and allow a repeat;
formats that apply a batch to one snapshot refuse one. xlsplice is the second
kind by construction.

Exit 2 rather than 4: `refused` is the family where the package says no and a
flag may override it — the formula guard, the shared master, a name that is not
a reference. A repeated cell is wrong against every package and no flag makes
it right, and #8 already puts an unknown operation kind in `usage`. The sources
agree here: RFC 5789 §2.2 lists only 4xx, Kubernetes maps duplicate keys to
`Invalid`/422, DynamoDB rejects duplicate targets at HTTP 400, and PostgreSQL
raises a Class 21 cardinality violation kept deliberately distinct from Class
XX `internal_error`.

## Considered options

- Apply them in order, last write wins: rejected, it would mean re-reading and
  re-parsing the part between operations, which is the one-parse rule ADR-0002
  set aside, and it would make a mis-addressed write silent.
- Let the overlap guard in `splice::apply` catch it: rejected, and it never
  worked. It is an assertion about byte ranges, and two writes to a cell
  written `<c r="A1"></c>` produce two *empty* ranges at one byte, which do not
  overlap: verified 2026-09-12, the batch exited 0 having written a part
  carrying `t="inlineStr"` twice, which no longer parses as XML.
- Add a flag to license a repeat: rejected, there is nothing a second write
  could mean that one write of the final value does not say more plainly.

## Consequences

- The check is on the resolved part and cell, so one cell named by its address
  and again by a defined name anchoring there is one cell.
- It runs before any part is read, which is why an operation answers where it
  lands (`Operation::at`) separately from what it wants done
  (`Operation::edits`). Resolving reads nothing, so the batch pays no I/O to
  hold its operations up against each other.
- `splice::apply`'s overlap guard stays `internal` and its doc comment now says
  it is an assertion about splices rather than a check on targets, so it is not
  mistaken for this one again.
- Unreachable from the command line today, which builds a batch of one. #8
  makes it reachable, and two invocations of `set` on one cell remain two
  batches and both land.
