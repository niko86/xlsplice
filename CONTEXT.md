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

**Target**:
The parts and nodes a command names for change. Everything outside the target is untouched.
_Avoid_: scope, selection

**Envelope**:
Everything in a worksheet part outside `<sheetData>`.
_Avoid_: header, wrapper

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
