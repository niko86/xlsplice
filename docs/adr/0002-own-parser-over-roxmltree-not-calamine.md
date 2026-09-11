# Read and locate with our own parser over roxmltree, not calamine

calamine is the standard Rust reader for Excel packages, but it cannot report a defined name's scope, a cell's style index, its stored type attribute, the raw value text, or any document property, and `get`, `names` and `props` must report exactly those. The write path needs a byte-range locator regardless. So every targeted part is parsed with roxmltree: reads walk the tree and report stored facts; writes take node byte ranges from the same tree and splice them, applied from the end of the part backwards so earlier ranges stay valid. Reference parsing for `Sheet!A1` and `refersTo` is our own, about sixty lines.

## Considered options

- calamine for reads: rejected for the missing facts above, and it pins its own quick-xml version.
- quick-xml streaming as the locator: its positions work and its round-trip was byte-identical on Excel-shaped input, but it silently drops a BOM and two common config options break the round-trip. roxmltree gives exact, BOM-inclusive ranges and tree navigation, which the insertion logic needs.
- The `a1` crate for references: rejected, it pulls in serialisation dependencies for sixty lines of parsing.

## Consequences

- One code path serves reads and writes.
- A targeted part is held whole in memory as a string. At the sizes seen, a few hundred kilobytes per sheet, that is nothing.
- v1 depends on `zip` and `roxmltree` for the package, plus a CLI and a JSON crate.
