# An empty value element means what the declared type says it means

A `<v>` that is present and holds nothing is written by more than one program
and means a different thing in each. This has now been decided twice in
opposite directions, so the reasoning is written down rather than left in a
commit message.

**Excel writes it for a formula that calculated to the empty string.** A
formula whose answer is `""` is stored as a `str` cell whose value element is
present and empty:

```xml
<c r="B1" t="str"><f>IF(ISNUMBER(A1),1,"")</f><v/></c>
```

This is not a rare shape. A read-only pass over the vendor corpus on
2026-09-24 — 1,583 packages, every one written by Microsoft Excel — found
1,149,670 cells holding a present but empty value element, and **every one of
them was `t="str"` with a formula**. Not one was untyped, and not one declared
any other type.

**openpyxl writes it for a formula it never calculated.** openpyxl does not
evaluate, so it emits the formula and an empty value element, and it never
declares a type on such a cell:

```xml
<c r="A1"><f>SUM(B1:B9)</f><v></v></c>
```

Round-tripping through openpyxl is more destructive still: loading a package
and saving it back strips the `t` attribute and discards the cached value on
every formula cell, so `<v>48</v>` comes back `<v></v>`. Nothing downstream
can recover what that erases.

**So the type decides.** The two spellings are the same XML — an empty element
— and the only thing that separates their meanings is the `t` attribute:

- `str` and `inlineStr` can hold the empty string as a value of their own, so
  a present but empty value element **is** a value there: `type` is the
  declared type, `raw` is `""` and `value` is `""`.
- No other type can. `""` is not a number, a boolean, an error code, a date or
  a shared-string index, so an empty value element declaring one of those
  stores nothing: `type` is `empty` and `raw` is `null`, exactly as a cell
  with no value element and a cell that does not exist do.

A caller can then tell Excel's answer from the absence of one: on a formula
cell, `raw: null` is a formula that was never calculated and `raw: ""` is one
that calculated to the empty string.

## What this replaces

`6917d9b` (2026-09-15) collapsed both spellings to "no value" because the
openpyxl shape was reaching `value_of` as a number holding `''` and being
refused as unreadable. That refusal is right for its own case and is kept: it
is what the second rule above preserves. What was wrong was applying one
answer to every type, which made Excel's million-cell case unreadable in a
different way — silently, as an absence rather than an error. Issue #47 found
it downstream, where a preflight warned on 123 cells of a normal return and
blamed the technician's Excel for it.

## Consequences

- `stored()` filters an empty raw only for the types that cannot hold it, so
  the decision lives in one line and the list of types is explicit.
- Nothing below `stored()` changes. `value_of` already answers the text types
  with the stored text, so `Some("")` becomes `Value::Text("")`, and the
  answer layer already renders that as `""` against `Value::Empty`'s `null`.
- A fixture for the Excel case **cannot be built with openpyxl**, which will
  not write `t="str"` at all. Hand-build the package, as the tests do.
- A shared-string cell with an empty value element holds an index that is not
  an index. It stays in the collapsing group and reads as empty rather than
  being refused. No such cell appeared anywhere in the corpus, so this is
  chosen for consistency with the other non-text types and can be revisited if
  a real one ever turns up.
