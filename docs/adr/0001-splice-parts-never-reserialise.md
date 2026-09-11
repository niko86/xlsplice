# Splice parts; never re-serialise a package

The OfficeCLI trial of 2026-09-10 showed that loading a package and writing it back whole produces files Excel demands to repair: worksheet envelope elements reordered, `calcChain.xml` dropped, prefixes normalised, cached values rewritten by a foreign engine. Our packages must be accepted by Excel and by a vendor add-in, so we decided the write model is a splice. Every part outside the target is copied byte for byte with its compression method and part order preserved; the zip container is rebuilt from those raw entries and may differ in its own bytes. A targeted part changes only in the named nodes, and every byte outside them is identical. There is no formula engine, no renderer, no audit stamp, and no implicit side effect: a command changes nothing it was not asked to.

## Considered options

- Event-based XML round-trip: rejected, it normalises exactly the things the bisect proved Excel cares about.
- Whole-file byte identity: rejected, it needs a custom zip writer and nothing that opens the package can observe the difference.

## Consequences

- Edits are located by byte range and spliced as text, not rebuilt through an XML writer.
- Text is written as inline strings so `sharedStrings.xml` is never a target; orphaned shared strings and stale `count` attributes are accepted and covered by the oracle suite.
- The guarantee is testable in CI without Excel: untouched parts compare equal, and a targeted part compares equal outside the spliced ranges.
- The container is rebuilt with the `zip` crate's raw entry copy, which keeps each part's compressed bytes, method, CRC and timestamp but drops local extra fields (on Excel-saved files, growth-hint padding) and rewrites some header flags. Excel opens the result clean. Should a consumer ever need those fields, the upgrade path is an own writer over the crate's entry offsets, about a hundred lines; see `docs/research/2026-09-11-crates-for-the-splice-layer.md`.
