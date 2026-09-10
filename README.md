# xlsplice

A small command-line tool for surgical edits to Excel packages (`.xlsx`, `.xlsm`):
read and write cells by address or defined name, read and write custom document
properties, set the calculate-on-load flag, and report which parts of a package
differ. Every part it does not target stays byte-identical; there is no formula
engine and no whole-file re-serialisation.

Design is not settled yet. It is being grilled from the findings of an
OfficeCLI trial in `the-reference-implementation` on 2026-09-10 (upstream bugs
iOfficeAI/OfficeCLI #389, #390, #391) and that repo's zip-level hydration
layer, which is the reference implementation.
