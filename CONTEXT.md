# xlsplice

A command-line tool for surgical edits to Excel packages. It exists because whole-package re-serialisation produced files that Excel demanded to repair; this tool changes only what it is told to and copies everything else byte for byte.

## Language

### Package

**Package**:
One `.xlsx` or `.xlsm` file: a zip container of parts.
_Avoid_: workbook, file, document

**Part**:
One zip entry in a package, identified by its path, such as `xl/worksheets/sheet1.xml`.
_Avoid_: entry, member, component

**Splice**:
An edit that replaces the bytes of named nodes inside one part and leaves every other byte of that part unchanged.
_Avoid_: patch, update, rewrite

**Part edit**:
One thing to be done to one part, named by its path: a _splice_ of its bytes,
the creation of a part the package does not hold, or the removal of one it
does. A splice is one species of part edit and the only one in use, so the two
are not synonyms: an _operation_ answers with part edits, and what a batch
applies is the edits on each part merged. Said in full wherever an
_operation_ is also in play, because an operation is not an edit.
_Avoid_: change, patch, mutation

**Target**:
What a command is pointed at. On the command line, one operand naming a cell,
by address or by defined name. Inside the tool, the parts and nodes a command
names for change; everything outside the target is untouched.
_Avoid_: scope, selection

**Relationship**:
What one part says another is to it, declared by id in the `_rels` part beside
the part that names it. A sheet's worksheet part is reached this way, and so
is the shared string table. Nothing but a relationship says which worksheet
part belongs to which sheet.
_Avoid_: link, pointer

**Envelope**:
Everything in a worksheet part outside `<sheetData>`. Say _worksheet envelope_
wherever the JSON envelope is also in play.
_Avoid_: header, wrapper

**Declaration**:
What a package says about a part besides holding it: its content type in
`[Content_Types].xml`, and the _relationship_ that reaches it. A part is
created and removed with both of them, because a package naming a part it does
not hold is what Excel offers to repair.
_Avoid_: registration, manifest entry

**Calc chain**:
The order Excel last calculated a workbook's formulas in, cached in
`xl/calcChain.xml`, one entry per formula cell. An entry names its sheet by
the number the workbook gives it, and one that names none is on the sheet the
entry before it named. Nothing depends on the chain being right, but an entry
for a cell that no longer holds a formula is an inconsistency, so a formula
replaced takes its entry with it.
_Avoid_: dependency graph, formula cache

**Full calc on load**:
Excel's standing instruction to work every formula out again on the way in
rather than trusting the _cached values_, carried as `fullCalcOnLoad` on the
workbook's calculation element. Set after a write whose consequences the
_calc chain_ and the caches no longer describe. Off is the absence of the
attribute, which is how Excel spells a workbook that does not ask for it.
_Avoid_: recalc flag, dirty flag

### Cells

**Address**:
A `Sheet!A1` reference to one cell.
_Avoid_: ref, coordinate, location

**Defined name**:
A workbook- or sheet-scoped name declared in the workbook part that refers to a cell or range.
_Avoid_: named range, label

**Cached value**:
The last result Excel stored in a formula cell.
_Avoid_: result, computed value

**Anchor**:
The top-left cell of the first area a defined name refers to; the one cell a name resolves to.
_Avoid_: first cell, origin

**Scope**:
Where a defined name can be seen from: the whole workbook, or one sheet.
Nothing to do with a command's _target_.
_Avoid_: level, visibility

**Sheet state**:
Whether a sheet's tab is shown: `visible`, `hidden` or `veryHidden`, spelled
as the package spells them. What is meant by a sheet's visibility.
_Avoid_: hidden flag, tab state

**Stored type**:
How a cell's value is stored, spelled as the cell's `t` attribute spells it:
`n`, `s`, `str`, `inlineStr`, `b`, `e` or `d`. A cell that is absent, or
present and holding no value, is `empty`.
_Avoid_: data type, cell type

**Shared string**:
Text a cell holds as an index into the package's one string table rather than
in the cell. An _inline string_ is the same text held in the cell itself. The
rich-text runs of either are one string, and phonetic text is no part of it.
_Avoid_: sst entry, interned string

**Shared formula**:
One formula filled across a range. Its _master_ carries the formula text and
the range; each _child_ carries only the _group_ the two have in common, and
takes its formula from the master. A master is never overwritten, because that
orphans its children.
_Avoid_: filled formula, formula group

**Style index**:
The number on a cell pointing into the package's formats. A cell declaring
none carries index 0, the default format.
_Avoid_: format id, xf

### Writing

**Operation**:
One thing a command does to a package: writing a cell, clearing one, setting a
property, setting the calculate-on-load flag. Named by what it does, not by the
verb that carried it.
_Avoid_: action, change, edit

**Batch**:
The list of operations one invocation asks of one package, validated whole
before anything is spliced and applied whole afterwards. A command line builds
a batch of one.
_Avoid_: transaction, job, plan

**Report**:
What came of a batch: a result per operation with whether it changed anything,
the parts changed, added and removed, where the result went, and whether it was
a dry run.
_Avoid_: summary, result, log

**Write type**:
What a write says a value is to become in the cell: `number`, `text`, `bool`
or `date`. Not a _stored type_, which is how the cell then spells it: a write
type of `text` is stored as `inlineStr`, and one of `number` declares no type
at all. A `date` is a `number` too, under whatever format the cell already
carries.
_Avoid_: value type, data type

**Date system**:
Which day a workbook counts its date _serials_ from, declared by `date1904` in
the workbook part: the 1900 system, where serial 1 is 1900-01-01, or the 1904
system, where serial 0 is 1904-01-01. The two are 1462 days apart. The 1900
system counts a 29th of February 1900 that never happened, kept for
compatibility with Lotus 1-2-3, so no serial below 61 names the day a calendar
would.
_Avoid_: epoch, base date

**Serial**:
The number a workbook stores a date as, whole days from the day its _date
system_ counts from, with a time of day as the fraction after the point.
_Avoid_: date value, timestamp

**Custom property**:
One of the named, typed values a package carries about itself, in
`docProps/custom.xml`, beside the author and title Excel fills in. A property
carries a name, matched exactly, and one child element naming its _variant
type_ and holding the value. Every property also carries an identifier, unique
within the part and counting from 2; one replaced keeps the identifier it had.
_Avoid_: metadata, tag, attribute

**Variant type**:
What a _custom property_ says its value is, spelled as the package spells it:
`lpwstr` for text, `i4` for a whole number, `r8` for one that is not, `bool`,
`filetime` for a moment. Not a _write type_, which is what a caller asks for:
a write type of `number` becomes `i4` or `r8` depending on the value, and one
of `date` becomes a `filetime` holding a moment in UTC rather than the _serial_
a cell would hold.
_Avoid_: property type, vt type

**Insertion**:
A cell or a row put into a worksheet because a write named one the part does
not hold. A template carries an element only for the cells something is
already in, so writing into one is ordinary rather than exceptional. A cell
goes into its row before the first cell of a greater column; a row goes into
the sheet data before the first row of a greater number. An inserted cell
takes the _style index_ Excel would show it under: its row's, where the row
declares a custom format, else the one a column definition covering it gives,
else none.
_Avoid_: creation, add, append

**Dry run**:
A batch done in full and put nowhere. It writes no file at all, and reports
what a real run would have changed.
_Avoid_: preview, simulation, check

**Difference**:
What two packages come to, compared _part_ by part: for each part, whether
both hold it with the same bytes, both hold it with different bytes, or only
one holds it at all; and, over all of them, whether the two are the same
package twice. Part-level only: nothing in a difference says what inside a
part moved. The container around the parts is no part of it, so two packages
whose entries sit in a different order, or carry different moments, still hold
the same parts.
_Avoid_: delta, changeset, comparison

### The contract

**Answer**:
What a verb comes back with: one result in both shapes at once, the payload the
JSON envelope carries and the same facts as rows under column headers. Every
row is as wide as the headers. Rendering an answer, or the error in its place,
is what produces the two streams and the exit code.
_Avoid_: response, output, result

**JSON envelope**:
The single JSON document a `--json` command writes to stdout: `ok`,
`schema_version`, the verb's payload, and, on failure, `error` with a stable
code. Distinct from the worksheet envelope.
_Avoid_: response, output, wrapper

**Exit code**:
The small integer a command exits with, one per error code, published and
frozen.
_Avoid_: status, return code, errorlevel

### Testing

**Oracle**:
A real Excel instance used by tests to say whether a package opens clean or demands repair.
_Avoid_: validator, checker

**Corpus**:
The real-world packages the test suites run over, kept outside the repository.
_Avoid_: fixtures, samples

**Fixture**:
A small synthetic package committed to the repository for tests that run anywhere.
_Avoid_: corpus, sample
