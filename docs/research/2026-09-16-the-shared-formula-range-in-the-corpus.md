# Is a shared formula's range ever exact? What the corpus says

Research note, 2026-09-16, for issue #46 (overwriting a shared-formula child leaves the
master's range naming it).

Primary source only: the corpus itself, read where it lives. Every count below comes from
one read-only pass over the vendor templates in the directory `XLSPLICE_CORPUS` names.
Nothing was extracted, nothing was copied, and no template is named here: the numbers are
aggregates over the whole set, which is what the question needs.

## Method

1,794 files in the tree, of which 218 are Excel's own lock files (`~$…`) and hold no
package; the remaining 1,576 packages, across 245 templates, were read in full. Only
worksheet parts were looked at. A **master** is an `<f t="shared">` carrying a `ref`
attribute; a **child** is one carrying `si` and no `ref`, which is how `worksheet.rs`
itself tells them apart. A **group**'s members are its master plus every cell in that part
carrying the same `si`. A group **over-covers** where its range covers a cell that is not
a member.

Two Excel versions wrote every package in the set, and nothing else did: `docProps/app.xml`
says `Microsoft Excel / 16.0300` for 67,282 of the groups and `14.0300` for the other
9,150. So every shape below is a shape Excel wrote, and reads back, in templates in daily
production use.

## Summary

1. **The range is not exact in the vendor material, and Excel is what made it inexact.**
   1,070 of 76,432 groups cover a cell that is not a member, across 78 of the 245
   templates. The invariant #46 proposed to restore has never held here.
2. **Most of the holes cannot be expressed as a range at all.** 777 of the 1,070 are
   interior: the far end of the range is a member, and a cell inside it is not. Only 293
   touch the far end, which is the one case a narrowed range could describe.
3. **A master is not always the corner of its own range.** 36 groups put the master
   somewhere other than the top-left, so no arithmetic over the range may assume it.
   1,128 groups are two-dimensional, where taking one cell out leaves a shape no range
   can spell.
4. **The range does not even mean "these cells take the master's formula."** Of the cells
   in a hole, 1,281 carry their own formula and 2,823 are children of a *different* group.
   Membership is carried by `si` and by nothing else, exactly as `CONTEXT.md` says.
5. **The shape #46 describes is already in the corpus.** 78 cells inside a range hold a
   value and nothing else — a cell that was a child and is now a plain value cell, which
   is precisely what xlsplice leaves behind after a licensed write over a child. Excel
   wrote those files.
6. **There is a second way in, with no formula in sight.** 834 cells inside a range have
   no cell element at all. A write to one of those is an ordinary insertion: no formula to
   refuse, no flag to carry, nothing for a rule about formula replacement to attach to. A
   narrowing that fired only on formula replacement would therefore be incomplete on its
   own terms.
7. **A group with no children is Excel's convention, not a defect.** 63 groups over a
   range wider than one cell have no children at all, in 18 templates. Demoting such a
   master to a plain formula would move xlsplice's output away from what Excel writes.

## What this decides

The range is advisory and always was. It is read in exactly two places in the tool — the
`range` field `get` reports (`src/answer.rs`) and the wording of the message that refuses
a write to a master (`src/batch.rs`) — and nothing anywhere decides anything from it, so
an over-covering range cannot mis-route a splice or produce a false refusal.

So #46 is closed by writing the fact down rather than by narrowing anything. Narrowing
would spend bytes ADR-0001 exists to preserve, would reach a cell the caller never named
and is refused the right to name, and would still leave the majority of real cases
untouched. The one thing worth changing is how the tool talks about the field: a caller
must read a cell's **group** to know what is in a group, never the master's range.

## Caveats

One vendor's templates, two Excel versions, one product family. The scan trusts `si` to be
unique within a part, which is what the schema says and what every package here does. A
different corpus could hold an exact range everywhere; it would not make the range
dependable, because these packages are the ones xlsplice is pointed at.
