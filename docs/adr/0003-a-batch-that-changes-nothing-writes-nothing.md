# A batch that changes nothing writes nothing

The spec promises that writing a value a cell already holds produces a package
identical to the input and reports nothing changed, so that re-running a
hydration is safe. ADR-0001 rebuilds the container from raw entries, which
drops Excel's local extra fields and rewrites some header flags: rebuilding a
package whose parts all still say what they said would therefore change the
file without changing anything in it. So a batch that splices no byte does not
write at all. In place, the package is left exactly as it was found; to `--out`,
the input's own bytes are copied across rather than a container rebuilt around
the same parts. Either way the result is the input byte for byte, and the
report says `changed: false`.

## Considered options

- Rebuild anyway and call the result unchanged: rejected, the bytes would
  differ from the input's and "byte-identical" would mean "identical except for
  the container", which is not what a caller comparing two files can observe.
- Make the rebuild byte-identical to the original: that is the own-writer
  upgrade path ADR-0001 already records, and it is a larger change than this
  promise needs.

## Consequences

- Two writes of the same value in a row leave one file, written once; the
  second run touches nothing, so a watcher sees no modification and a backup
  sees no new version.
- A spliced package and a copied one are different shapes of output from the
  same verb, so the write path decides between them in one place
  (`batch::land`) rather than at each caller.
- Idempotence is observable from outside with a byte comparison, which is what
  the byte-preservation suite asserts.
