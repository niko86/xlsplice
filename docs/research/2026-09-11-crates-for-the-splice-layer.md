# Crates for the splice layer: zip copying, XML editing, value reading

Researched 2026-09-11 against crate sources in the cargo registry, docs.rs / crates.io
metadata, PKWARE APPNOTE 6.3.10 and ECMA-376 Part 2 (5th ed., Dec 2021). Runnable probes
were built in a throwaway cargo project; a fresh `.xlsx` was saved by Excel 16.112.4
(`osascript`) to observe the real package layout. Versions examined: `zip` 8.6.0 (and
9.0.0-pre3), `quick-xml` 0.42.0, `roxmltree` 0.21.1, `calamine` 0.36.1, `rawzip` 0.5.1,
`rc-zip` 5.4.1 + `rc-zip-sync` 4.4.2, `xmlparser` 0.13.6, `xot` 0.31.2, `a1` 1.0.2,
`formualizer-parse`/`-common` 3.1.1. Registry path abbreviated below as `$REG` =
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`.

## Summary

1. **Zip: use `zip` 8.6.0 (MIT, MSRV 1.88) to read; do not expect `raw_copy_file` to give a byte-identical file.** It copies the compressed payload, method, CRC, sizes, DOS mtime and entry comment verbatim, but rebuilds both headers: it drops local extra fields (Excel's 0xA220 growth-hint padding), drops general-purpose flag bits 1-2 (Excel writes 0x0006), and rewrites system (DOS→Unix), "version made by" (4.5→2.0) and external attributes (0→0o100644<<16). Measured: 9165-byte Excel file → 7333 bytes.
2. `ZipWriter::merge_archive` is far closer: bytes before the central directory are copied verbatim; only the central directory's flags field changes (0x0006→0x0000). Excel 16.112.4 opened both variants without a repair prompt. Whole-file fidelity needs ~100 lines of own writer over `zip`'s offsets (`header_start`, `data_start`, `central_header_start`); the crate does not even expose local extra fields.
3. `rawzip` 0.5.1 (MIT, MSRV 1.85) exposes raw compressed slices and parsed local headers with extra fields, and has a writer with `extra_field`, but no raw-copy entry point; `rc-zip` is read-only, decompressing; `zip-extract` is deprecated.
4. **XML: text splicing (strategy b) with byte ranges from `roxmltree` 0.21.1 (MIT OR Apache-2.0, MSRV 1.60) is the strategy the evidence supports**; the untouched bytes are preserved by construction. `roxmltree` ranges include the BOM offset and cover `<`…`>` of the end tag for elements, raw escaped text for text nodes, and value-without-quotes for attributes.
5. `quick-xml` 0.42.0 (MIT, MSRV 1.86) Reader→Writer round-trip is byte-identical on Excel-shaped input under the default `Config` (attribute order/quotes, `<c/>` vs `<c />` vs `<c></c>`, entities as `GeneralRef`, decl, comments, PIs, CDATA, `\r\n`) — except it silently drops a UTF-8 BOM. `trim_text` or `expand_empty_elements` break it; constructed nodes are normalised to `key="value"` with escaping. Its offsets (`buffer_position()`, contiguous, BOM excluded) also suffice for strategy (b).
6. **Read: `calamine` 0.36.1 (MIT, MSRV 1.88) covers visibility incl. `veryHidden`, defined names, typed values, formulas (shared expanded), `.xlsm`, missing `sharedStrings.xml`.** It does not expose defined-name scope (`localSheetId`), the style index, the `t` attribute, raw `<v>` text, a date-conversion switch, or any `docProps/*`; it pins `quick-xml` 0.41 and `zip` 8.6.
7. Because writes need the tool's own `workbook.xml`/sheet parser anyway, an own `quick-xml` reader covers the read operations with less surface; `calamine` is optional convenience, not a dependency the design needs.
8. Excel-produced layout (observed, 11 entries): `[Content_Types].xml` first, DOS system, no data descriptors, no zip64, deflate with flag 0x0006 ("super fast"), 1980-01-01 timestamps, 0xA220 growth-hint padding in the *local* header of five parts, no BOM, `\r\n` after the XML declaration, no inter-element whitespace. OPC Annex B requires local/central header consistency and rates third-party extra fields "pass through on editing".
9. A1 references: `a1` 1.0.2 (MIT) parses `'Sheet Name'!$A$1:$B$2` with `''` escapes and `$` anchors but pulls `rkyv`+`serde`; `formualizer-common` 3.1.1 (MIT OR Apache-2.0) has sheet-scoped refs with anchors. Both small enough to replace with ~60 own lines.
10. Licences of everything recommended: `zip` MIT, `quick-xml` MIT, `roxmltree` MIT OR Apache-2.0, `calamine` MIT, `rawzip` MIT.

## 1. Zip: copying entries without re-serialisation

### 1.1 The `zip` crate today

- crates.io: `max_stable_version` 8.6.0 (published 2026-04-25), `newest_version` 9.0.0-pre3 (2026-08-10), licence MIT, `rust_version` 1.88, repo `zip-rs/zip2` (crates.io API, fetched 2026-09-11). Confirmed in `$REG/zip-8.6.0/Cargo.toml:13-16,50` (`edition = "2024"`, `rust-version = "1.88"`, `license = "MIT"`).
- Current API names: `ZipWriter::raw_copy_file` (`src/write.rs:1691-1694`), `raw_copy_file_rename` (`:1623-1633`), `raw_copy_file_to_path` (`:1660-1666`), `raw_copy_file_touch` (`:1719-1742`, overrides mtime and unix mode), `merge_archive` (`:1563-1580`). Raw entries come from `ZipArchive::by_index_raw` (`src/read/zip_archive.rs:535`).
- 9.0.0-pre3 (pre-release, 2026-08-10) keeps the same shape: `raw_copy_file_rename` still starts from `file.options().into_full_options()` (`$REG/zip-9.0.0-pre3/src/write.rs:1540-1573`) and `ZipFile::options()` still carries only four fields (`src/read/zipfile.rs:309-333`). Its changelog lists "rewrite extra fields (#879)" and "change `extra_data` to `extra_fields` (#833)" plus several `[breaking]` removals in pre1 (zip2 `CHANGELOG.md` on `master`, fetched 2026-09-11, lines 10-70), so the 9.x API will move; nothing there changes the fidelity conclusions below.

### 1.2 What `raw_copy_file` preserves, from the source (8.6.0)

Path: `raw_copy_file` → `raw_copy_file_rename` → `raw_copy_file_rename_internal` (`write.rs:1635-1653`):

```rust
let raw_values = ZipRawValues { crc32: file.crc32(), compressed_size: file.compressed_size(), uncompressed_size: file.size() };
self.start_entry(name, options, Some(raw_values))?;
self.writing_to_file = true; self.writing_raw = true;
io::copy(&mut file.take_raw_reader()?, self)?;
```

`take_raw_reader` (`read.rs:777-779`) hands over the underlying `Take<&mut R>` positioned at the compressed data, so **payload bytes are copied verbatim** (no inflate/deflate). `options` comes from `ZipFile::options()` (`read.rs:1046-1073`), which builds a `SimpleFileOptions` from exactly: `large_file` (size > zip64 threshold), `compression_method`, `unix_permissions(self.unix_mode().unwrap_or(0o644) | S_IFREG)`, `last_modified_time` (DOS time, replaced by `DateTime::default_for_write` if invalid), then `normalize()` (`write.rs:497-503`). `into_full_options` (`write.rs:721-737`) sets `extended_options: ExtendedFileOptions::default()`, i.e. **no extra fields carried**. `raw_copy_file_rename` re-adds the entry comment (`write.rs:1628-1631`).

The local header is then built by `start_entry` (`write.rs:1169-1330`) and `ZipFileData::initialize_local_block` (`types.rs:405-478`):

| Field | Preserved? | Evidence |
|---|---|---|
| Compressed payload bytes | yes | `io::copy(take_raw_reader)`, `write.rs:1651` |
| Compression method | yes | `options().compression_method`, `read.rs:1050` |
| CRC-32, compressed/uncompressed sizes | yes | `ZipRawValues`, `write.rs:1641-1645` |
| Last-modified (DOS date/time, 2 s) | yes if valid | `read.rs:1052-1056`; written via `timepart()/datepart()`, `types.rs:637-641` |
| Entry comment | yes | `write.rs:1628-1631` |
| Entry name | yes (rename possible) | `write.rs:1692` |
| Entry order | as you call it | caller-driven loop |
| Unix permissions | yes when present, else 0o644 | `read.rs:1051`; DOS entries with external attrs 0 have `unix_mode() == None` (`types.rs:320-323`) |
| Local extra fields | **no** | never read: `extra_field` is filled from the central directory (`read.rs:524,554`); `find_data_start` reads only the lengths (`types.rs:235-260`); `into_full_options` discards extras |
| Central extra fields | **no** | same `into_full_options`; `ZipFile::extra_data()` (`read.rs:1026-1028`) returns them but `options()` ignores them |
| General-purpose flags | **recomputed** | `flags: 0` at `types.rs:441`; serialised from `ZipFileData::flags()` (`types.rs:592-608`) = utf8-bit (only if name non-ASCII) \| data-descriptor bit \| encrypted bit. Bits 1-2 (deflate option) lost; a utf8 flag on an ASCII name lost |
| Data descriptor (bit 3) | cleared on seekable writers | `using_data_descriptor = !self.seek_possible`, `write.rs:1298-1299` |
| "Version made by" | **recomputed** | `version_made_by = version_needed()` (`types.rs:477`, `write.rs:1300`) → 20 for deflate (`types.rs:351-386`) |
| Host system (upper byte of version made by) | **replaced** | `options.system` is `None`; falls to `cfg!(windows)` → `Dos` else `Unix` (`types.rs:424-431`) |
| External attributes | **replaced** | `permissions << 16` (`types.rs:422`) |
| Zip64 markers | regenerated from sizes | `Zip64ExtendedInformation::new_local(options.large_file)`, `write.rs:1189-1193`; central: `write.rs:2497-2502` |
| Archive comment | not copied | must be re-set with `set_comment`/`set_raw_comment` (`write.rs:1034,1046`); readable via `ZipArchive::comment()` (`read.rs:971`) |

The central directory is always re-serialised by `finalize` → `write_central_and_footer` (`write.rs:1866-1930`) → `write_central_directory_header` (`write.rs:2489-2530`) from `ZipFileData::block()` (`types.rs:656-720`), which recomputes `version_to_extract`, `flags()`, sets `internal_file_attributes: 0`, and strips alignment/zip64 blocks from any central extra (`strip_alignment_extra_field`, `write.rs:2533-2556`).

### 1.3 Byte-identical file, or only byte-identical payloads?

Only payloads (and, with `merge_archive`, everything before the central directory). Measured on the Excel-saved probe file with two probes:

```rust
// zz.rs (abridged): per-entry raw copy
let mut archive = ZipArchive::new(Cursor::new(orig.clone()))?;
let mut out = ZipWriter::new(Cursor::new(Vec::new()));
for i in 0..archive.len() { out.raw_copy_file(archive.by_index_raw(i)?)?; }
let out = out.finish()?.into_inner();
// then: compare orig[header_start..data_start] vs copy, and raw payloads, per entry
```

Output (all 11 entries behaved identically; one row shown):

```
orig len 9165, out len 7333, whole-file identical: false
entry                 hdr=  pay=  aExtra bExtra aMode bMode        aMadeBy bMadeBy
[Content_Types].xml   false true  0      0      None  Some(33188)  (4,5)   (2,0)
    local header a: [50,4b,03,04, 14,00, 06,00, 08,00, 00,00,21,00, ... 13,00, 08,02]
    local header b: [50,4b,03,04, 14,00, 00,00, 08,00, 00,00,21,00, ... 13,00, 00,00]
```

i.e. flags `06 00` → `00 00` and local extra length `08 02` (520) → `00 00`; payload bytes identical. The copy's central directory (Python `zipfile` dump) shows `create_system` 0→3, `create_version` 45→20, `flag_bits` 0x0006→0x0000, `external_attr` 0→0x81a40000. Note `aExtra 0`: the crate reported no extra data for the original even though its local header carries 520 bytes — the API cannot see local extra fields at all.

```rust
// mz.rs: whole-archive merge
let mut w = ZipWriter::new(Cursor::new(Vec::new()));
w.merge_archive(ZipArchive::new(Cursor::new(orig.clone()))?)?;
```

```
zip 8.6.0 merge_archive: orig 9165 bytes, out 9165 bytes, whole-file identical: false
bytes before central directory identical: True | cd sizes 702 702 | eocd identical: True
  [Content_Types].xml   flags: 0x6->0x0      (same for all 11 entries; every other field identical)
```

`merge_contents` copies `[0, dir_start)` with one `io::copy` and reuses the cloned `ZipFileData` (`read.rs:218-280`), so local headers keep their extra fields and flags; only `flags()` is recomputed for the central directory. This produces local/central flag disagreement, which OPC Annex B.2 says a producer "shall" avoid ("equal values in the appropriate fields of every File Header within the Central Directory and the corresponding Local File Header", ECMA-376-2 Annex B.2, p. 63) — Excel tolerated it (below), but it is a spec deviation.

Excel oracle (AppleScript from the handoff, opening each output as `.xlsx`, reading `C3` and `A1`, no repair sheet present afterwards): `raw_copy_file` output → `OK C3=84.0 A1=hello & <world>`; `merge_archive` output → same; original → same. So Excel 16.112.4 accepts both normalisations on this small file (one file, one Excel build: a data point, not a guarantee).

Design consequence: the crate's reader gives every offset needed to write a byte-faithful archive yourself — `header_start()`, `data_start()` (populated by `by_index_raw`), `central_header_start()` (`read.rs:1031-1043`), `compressed_size()` (`read.rs:986`), `ZipArchive::comment()`. For each untouched entry copy `[header_start, data_start + compressed_size)` verbatim (Excel writes no data descriptors; if bit 3 is set, add the 12/16-byte descriptor or refuse), then copy each source central-directory record verbatim with only the 4-byte local-header offset patched, and write a fresh end-of-central-directory record. That is the only route to "byte-identical except the targeted entries" at the container level; `zip` alone gives "payload-identical".

### 1.4 Alternatives with raw access

- **`rawzip` 0.5.1** (MIT, MSRV 1.85, published 2026-07-13; crates.io API). Zero-dependency reader/writer; the caller supplies the compressor. Reader: `ZipArchive::from_slice` / `from_file`, `entries()`, `get_entry(wayfinder)`; `ZipSliceEntry::data()` "Returns the raw, compressed data of the entry as a byte slice" (`$REG/rawzip-0.5.1/src/archive.rs:228-231`); `local_header()` parses the local header including `extra_fields()` and `flags()` (`archive.rs:330-345`); central entries expose `compression_method`, `crc32`, `last_modified`, `mode`, `extra_fields`, `comment`, `local_header_offset` (`archive.rs:1250-1431`). Writer: `ZipArchiveWriter::new_file(..)` builder with `compression_method`, `last_modified`, `unix_permissions`, `extra_field`, `crc32`, `comment`, then `start()` → `ZipEntryWriter` + `finish(DataDescriptorOutput)` (`src/writer.rs:314-522, 1071`). No documented "copy pre-compressed entry" API; whether `finish` accepts a caller-built descriptor for raw bytes is **unverified**. Best fit if the own-writer route in 1.3 wants a parsed view of local headers.
- **`rc-zip` 5.4.1** (MIT OR Apache-2.0, 2025-11-19) is "a sans-io library for **reading** zip files" (`$REG/rc-zip-5.4.1/src/lib.rs:3-13`); `Entry` exposes `name, method, comment, modified, header_offset, reader_version, flags, …` (`src/parse/archive.rs:63-140`). `rc-zip-sync` 4.4.2 (2025-11-27) adds `entries()`, `by_name()`, `reader()` (decompressing) and `bytes()` (`$REG/rc-zip-sync-4.4.2/src/read_zip.rs:154-200`); no raw-compressed accessor and no writer. Not a fit.
- **`zip-extract` 0.4.1** (2025-07-16): crates.io description is "Deprecated, use the zip crate instead."

### 1.5 How Excel lays out a package (observed + spec)

Probe: new workbook saved by Microsoft Excel 16.112.4 (`Info.plist CFBundleShortVersionString`; `docProps/app.xml` says `<Application>Microsoft Macintosh Excel</Application><AppVersion>16.0300</AppVersion>`), Python `zipfile` + raw local-header dump:

```
entry                       sys made need  cflag  lflag meth cextra lextra    extattr  dt
[Content_Types].xml           0   45   20 0x0006 0x0006    8      0    520 0x00000000 (1980,1,1,0,0,0)
_rels/.rels                   0   45   20 0x0006 0x0006    8      0    520 0x00000000
xl/workbook.xml               0   45   20 0x0006 0x0006    8      0      0 0x00000000
xl/_rels/workbook.xml.rels    0   45   20 0x0006 0x0006    8      0    264 0x00000000
xl/worksheets/sheet1.xml      0   45   20 0x0006 0x0006    8      0      0 0x00000000
xl/theme/theme1.xml, xl/styles.xml, xl/sharedStrings.xml, xl/calcChain.xml   (same, lextra 0)
docProps/core.xml             0   45   20 0x0006 0x0006    8      0    264 0x00000000
docProps/app.xml              0   45   20 0x0006 0x0006    8      0    264 0x00000000
local extra on the five parts: header ID 0xa220, sig 0xA028, PadVal 2 (520-byte) / 1 (264-byte), NUL padding
```

- Order: `[Content_Types].xml` first, then `_rels/.rels`, workbook, its rels, sheets, theme, styles, sharedStrings, calcChain, docProps. OPC only requires the name `[Content_Types].xml` (ECMA-376-2 §7.3.7); a crude text search of the Part 2 PDF found no clause requiring it to be the first item (**unverified** beyond that search).
- Flags 0x0006 with method 8 = "Super Fast (-es) compression option was used" (APPNOTE 6.3.10 §4.4.4, bits 2,1 = 1,1). Bit 3 clear: no data descriptors. Version made by 45 with host byte 0 = MS-DOS (APPNOTE §4.4.2.1-2; 4.5 = "File uses ZIP64 format extensions", §4.4.3.2 line 725) even though no zip64 structures are present; version needed 20.
- The local-only extra field is the Microsoft Open Packaging Growth Hint (APPNOTE §4.6.10, ID 0xa220, "Sig 0xA028, PadVal, Padding filled with NULL characters"). OPC defines it: "A part may have a growth hint … To allow in-place growth … Padding reserved in the ZIP Extra field in the local header that precedes the item" (ECMA-376-2 §6.2.4 and the §7.3 ZIP mapping table, referencing §7.3.8). Dropping it (as `raw_copy_file` does) removes an optional optimisation, not required data.
- OPC Annex B (normative), pp. 63-72: B.2 header consistency (above); Table B.1: data descriptor "Supported on consumption: Yes; production: Optional; pass through on editing: Optional" — so other producers may emit them; Table B.5: bit 3 "Yes/Yes/Yes", bit 4 "No / Bits set to 0 / Yes"; Deflate (2.0) "Yes/Yes/Yes"; extra-field table: 0x0001 zip64 "Yes/Yes/Optional", third-party IDs such as 0x5455 "No/No/Yes" (pass through on editing). Annex B also notes that "editing means in-place modification of individual records" and that a format may instead "re-write all parts and relationships on each save".
- All eleven XML parts start with `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n`, none has a BOM, and the sheet has no whitespace between elements: `<c r="C3"><f>B2*2</f><v>84</v></c>`, `<calcPr calcId="181029"/>`. Deflate level is not recoverable from the file (**unverified**).

## 2. XML: editing a part while leaving untouched bytes untouched

### 2.1 `quick-xml` 0.42.0 round-trip fidelity

Facts: crates.io 0.42.0 (2026-08-22), MIT, `rust-version = "1.86"` (`$REG/quick-xml-0.42.0/Cargo.toml:14,44`); `default = []` features, `encoding` optional (Cargo.toml `[features]`).

Reader `Config` (`src/reader/mod.rs:27-235`), defaults: `allow_dangling_amp` false; `allow_unmatched_ends` false; `check_comments` false; `check_end_names` **true**; `expand_empty_elements` **false** ("those tags are represented by an `Empty` event"); `trim_markup_names_in_closing_tags` **true** (strips whitespace in `</a >`); `trim_text_start`/`trim_text_end` **false**, each carrying a boxed warning that trimming "has known issues" (`mod.rs:174-235`). Writer `Config::add_space_before_slash_in_empty_elements` default false (`src/writer.rs:29-60,127`).

Writer path (`src/writer.rs:262-306`): `Start` → `<` + raw buf + `>`; `End` → `</` + name + `>`; `Empty` → `<` + raw buf + `/>`; `Text` → `e.as_bytes()` raw; `Comment`/`Decl`/`PI`/`DocType`/`GeneralRef`/`CData` → delimiters + raw content. Nothing is re-escaped on write. Escaping happens only when *constructing* events: `BytesText::new` escapes, `from_escaped` does not (`src/events/mod.rs:563-588`); `Attribute` from `(&str, &str)` escapes the value (`src/events/attributes.rs:363-365`); `push_attribute` emits `key="value"` with double quotes and a `// FIXME: need to escape attribute content` (`events/mod.rs:302-309`). Entity and character references are emitted as `Event::GeneralRef` and written back as `&name;` (`mod.rs:52`, `writer.rs:301`).

Probe (`qx.rs`, default config unless stated; input 510 bytes incl. a 3-byte BOM, mixing `"`/`'` quotes, double spaces in a start tag, `<c/>`, `<c />`, `<c></c>`, `&amp; &lt; &#10; &#x41;`, CDATA, comment, PI, `\r\n`):

```rust
let mut reader = Reader::from_reader(INPUT); cfg(reader.config_mut());
let mut writer = Writer::new(Cursor::new(Vec::new()));
loop { let before = reader.buffer_position(); let ev = reader.read_event_into(&mut buf)?; let after = reader.buffer_position();
       if matches!(ev, Event::Eof) { break } writer.write_event(ev)?; buf.clear(); }
```

```
   0..55   Decl(BytesDecl { content: "xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"" })
  55..57   Text("\r\n")
  57..236  Start("worksheet xmlns=\"…\" xmlns:r='…' mc:Ignorable=\"x14ac xr\"")
 236..246  Comment(" c ")      246..257 PI("pi data")
 268..293  Start("row r=\"1\"  spans='1:2' ")
 327..338  Empty("c r=\"B1\"")  338..350 Empty("c r=\"C1\" ")  350..360 Start("c r=\"D1\"") 360..364 End("c")
 416..419  Text(" a ")  419..424 GeneralRef("amp")  424..427 Text(" b ")  427..431 GeneralRef("lt")  432..437 GeneralRef("#10")  438..444 GeneralRef("#x41")
 458..473  CData("x<y")  479..482 Text("\n  ")  506..507 Text("\n")  507..507 Eof
[default config]              byte-identical to input: false; identical to input minus BOM: true
[trim_text(true)]             identical minus BOM: false  (first diff at 55: "\r\n" after the declaration dropped)
[expand_empty_elements=true]  identical minus BOM: false  (<c r="B1"/> → <c r="B1"></c>, <c r="C1" /> → <c r="C1" ></c>)
[trim_markup_names_in_closing_tags=false] identical minus BOM: true
```

Per item: attribute order, quote style, intra-tag whitespace — preserved (raw buf); `<c/>` / `<c />` / `<c></c>` — preserved (the trailing space survives inside the `Empty` buf); entities and numeric refs — preserved verbatim as `GeneralRef`, never unescaped; namespace prefixes — untouched (plain `Reader`, not `NsReader`); declaration, CDATA, comments, PIs, `\r\n` and text whitespace — preserved. **BOM: dropped.** The reader strips it before parsing (`src/reader/buffered_reader.rs:20-41`, `slice_reader.rs:253-264`, `mod.rs:302-304`) and emits no event for it. Excel parts carry no BOM (§1.5), so this bites only on foreign-produced packages.

### 2.2 `quick-xml` byte offsets

- `Reader::buffer_position()` "Gets the byte position in the input data just after the last emitted event (i.e. this is position where data of last event ends)"; with `trim_text_end` set it is "position before trim" (`src/reader/mod.rs:902-911`). `error_position()` points to the `<` of the offending markup (`mod.rs:913-926`). `read_to_end`/`read_to_end_into` return `Span = Range<u64>` (`mod.rs:574`; `slice_reader.rs:159`; `buffered_reader.rs:503`).
- The probe shows event spans are contiguous (`0..55, 55..57, 57..236, …`), so `[buffer_position before, buffer_position after)` is the exact byte range of each event, for `Start`/`Empty`/`End`/`Text`/refs alike — provided trimming is off. Offsets are **relative to the stream after BOM removal** (the declaration reported `0..55` while occupying file bytes 3..58); add the BOM length yourself when splicing into the original bytes.

### 2.3 `roxmltree` 0.21.1

Facts: 0.21.1 (2025-10-12), MIT OR Apache-2.0, `rust-version = "1.60"` (`$REG/roxmltree-0.21.1/Cargo.toml:14,34`); `default = ["std", "positions"]` (`Cargo.toml:37-42`); its only dependency is `memchr` — the tokenizer is inlined (`src/tokenizer.rs`; `lib.rs:36`), it no longer depends on `xmlparser`.

- `Document::parse(text: &'input str)` — "We do not support `&[u8]` or `Reader` because the input must be an already allocated UTF-8 string" (`src/parse.rs:415-441`). Whole part in memory, borrowed by the tree; `input_text()` returns it (`lib.rs:217`).
- `Node::range()` (`lib.rs:1450-1455`), `Attribute::range()` / `range_qname()` / `range_value()` (`lib.rs:589-630`; `position()` deprecated). Element ranges: start tag `<` to end of the end tag (`parse.rs:891` for `<e/>`, `parse.rs:903` sets `range.end` at the close tag); text nodes carry the raw token range (`parse.rs:746-747`).
- DTD: `ParsingOptions::allow_dtd` default false → `Error::DtdDetected` (`parse.rs:326-346, 94-97`). Encoding declaration: parsed and discarded (`tokenizer.rs:332-335`); non-UTF-8 bytes must be transcoded before calling. BOM: skipped by the tokenizer (`tokenizer.rs:298-301`) but positions stay relative to the original string.

Probe (`rx.rs`, same shape as above with a BOM; ranges printed as slices of the original `&str`):

```
Root     0..239   (whole input incl. BOM and declaration)
Element 60..238   "<row r=\"1\"  spans='1:2' ><c …</c>\n  </row>"
      attr "r"     range=65..70 -> "r=\"1\"";   range_value=68..69 -> "1"
      attr "spans" range=72..83 -> "spans='1:2'"; range_value=79..82 -> "1:2"
Element 85..119   "<c r=\"A1\" s=\"3\" t=\"s\"><v>0</v></c>"
Element 107..115  "<v>0</v>"        Text 110..111 "0"
Element 119..130  "<c r=\"B1\"/>"   Element 130..142 "<c r=\"C1\" />"
Element 170..220  "<t xml:space=\"preserve\"> a &amp; b &lt; &#10; </t>"
Text   194..216   " a &amp; b &lt; &#10; "     text() unescaped = " a & b < \n "
Text   229..232   "\n  "
DOCTYPE default: Some(DtdDetected)     input_text() starts with BOM: true
```

So `range()` gives exactly what a splice needs: the BOM counts (row starts at 60 = 3 + 55 + 2), an element's range spans both tags, an attribute's `range_value` excludes the quotes, and a text node's range is the raw escaped source while `text()` is the unescaped view.

### 2.4 Other XML crates

- **`xmlparser` 0.13.6** (MIT/Apache-2.0; last release 2023-09-30): "a low-level, pull-based, zero-allocation XML 1.0 parser … All tokens contain `StrSpan` structs which represent the position of the substring in the original document … not intended to be used directly" (`$REG/xmlparser-0.13.6/README.md`); skips a UTF-8 BOM (`src/lib.rs:353-356`); token spans documented per variant (`lib.rs:84-200`). Dormant for three years and superseded by roxmltree's inlined tokenizer; usable, but roxmltree gives the same spans plus a tree.
- **`xot` 0.31.2** (MIT; 2025-04-09): "parse XML into a tree, and serialize back to XML … Pretty-printing. Removal of non-significant whitespace"; no DTD (`$REG/xot-0.31.2/README.md`). No lossless or round-trip claim anywhere in its README or `lib.rs`; it is a mutable DOM that re-serialises. Not a fit for the guarantee.
- No other current crate found that claims byte-lossless XML round-tripping (crates.io search, 2026-09-11).

### 2.5 Which strategy the evidence favours

Text splicing (b). With (b) the guarantee "a targeted part differs only in the named nodes" holds by construction — output = `input[..start] ++ replacement ++ input[end..]` — and the test is a byte diff outside the spliced ranges; nothing about the parser's serialiser matters. Both locators are adequate: `roxmltree` gives explicit, BOM-inclusive ranges for elements, attributes and text (2.3); `quick-xml` gives contiguous end positions per event (2.2) and streams, which matters for very large sheets. Event round-trip (a) is demonstrably lossless *except the BOM* on Excel-shaped input under the default config (2.1), but it depends on (i) the `Config` staying exactly default, (ii) the Writer's raw emission surviving upgrades, and (iii) constructed events being the only normalised nodes; it is acceptable only behind the same byte-diff gate, at which point it is (b) with extra moving parts. Either way, new or edited nodes come out normalised (`key="value"`, escaped), which is what the write path wants.

## 3. Reading values: `calamine` 0.36.1 versus an own parser

Facts: 0.36.1 (2026-07-27), MIT, `rust-version = "1.88"` (`$REG/calamine-0.36.1/Cargo.toml:14,40`); depends on `quick-xml = "0.41"` and `zip = "8.6"` (default-features = false) — one minor behind the current `quick-xml`, so a project on 0.42 compiles two copies.

- **Sheet visibility**: `Reader::sheets_metadata() -> &[Sheet]` (`src/lib.rs:359`), `Sheet { name, typ: SheetType, visible: SheetVisible }` (`lib.rs:286-294`), `SheetVisible::{Visible, Hidden, VeryHidden}` (`lib.rs:275-283`), parsed from `<sheet state="…">` (`src/xlsx/mod.rs:459-476`; unknown state is an error). `sheet_names()` is "in workbook order" (`lib.rs:350`).
- **Defined names**: `defined_names() -> &[(String, String)]` (`lib.rs:364`). The xlsx reader stores only the `name` attribute and the element text (`xlsx/mod.rs:523-537`); `localSheetId`, `hidden`, `comment` are not read, so **scope is not available** and a sheet-scoped name is indistinguishable from a workbook-scoped one. The reference string is returned as unescaped text (e.g. `Sheet1!$A$1:$B$2`), unparsed.
- **Per cell**: `worksheet_range` returns `Range<Data>`; `worksheet_range_ref` returns `Range<DataRef>` (`lib.rs:332-335,475`). `Data` variants: `Int, Float, String, Bool, DateTime(ExcelDateTime), DateTimeIso, DurationIso, Error(CellErrorType), Empty` (`src/datatype.rs:36-56`); `DataRef` adds `SharedString(&str)` (`datatype.rs:340-361`). Mapping from the `t` attribute in `read_v` (`src/xlsx/cells_reader.rs:621-676`): `s` → shared string, `b` → `Bool`, `d` → `DateTimeIso(raw)`, `e` → `Error`, `str` → `String`, `n`/absent → `f64` via `fast_float2` then `format_excel_f64_ref` (`src/formats.rs:112-131`) which yields `DateTime` when the cell's style number-format is date/time-like, else `Float`; `inlineStr` → `String` (`cells_reader.rs:566-577`). `Cell<T>` carries only `pos` and `val` (`lib.rs:563-569`): the **style index `s` and the `t` attribute are read (`cells_reader.rs:270-271`) but not exposed**, and the raw `<v>` text is lost for numbers (`"1E2"`, `"-0"`, `"42.0"` all become `Float`). `Data::Int` is produced only for pivot-cache items in xlsx (`xlsx/mod.rs:3615-3625`), never for worksheet cells. **Date conversion cannot be switched off**; the serial is recoverable via `ExcelDateTime::as_f64()` (`datatype.rs:726`), and `has_1904_epoch()` exposes `date1904` (`xlsx/mod.rs:518-521,1141`).
- **Formula vs cached value**: separate — `worksheet_formula` returns `Range<String>` of formula text (`xlsx/mod.rs:2601-2620`), `worksheet_range` the cached values; `XlsxCellReader::next_cell_with_formula` gives both per cell, with shared formulas expanded from the anchor (`cells_reader.rs:142-152, 317-380`); `next_cell_with_formula_metadata` reports shared anchors/derived cells without expansion (`cells_reader.rs:389-399`).
- **Document properties**: none. No occurrence of `docProps`, `custom.xml`, `core.xml` or `app.xml` anywhere under `src/` (grep, 2026-09-11).
- **`.xlsm`**: `open_workbook_auto` routes `xlsx | xlsm | xlam` to `Xlsx` (`src/auto.rs:42`); `vba_project()` reads `xl/vbaProject.bin` (`xlsx/mod.rs:2578`). **Missing `sharedStrings.xml`**: `read_shared_strings` returns `Ok(())` when the part is absent (`xlsx/mod.rs:341-345`).
- **Not provided, so an own parser is needed anyway** (against the handoff's operation set): defined-name scope and the `?TC`-style names' metadata; cell `t` and `s` attributes and the literal `<v>` text (needed to report the *stored* type and to keep the style on write); `docProps/custom.xml` read/write; `calcPr`/`fullCalcOnLoad`; `calcChain.xml`; any byte offsets (calamine parses into values, never positions); anything about `[Content_Types].xml`/rels. Since the write path must parse `workbook.xml` and sheet XML with positions, the same `quick-xml`/`roxmltree` pass can answer the read operations; `calamine` would add a dependency and a second `quick-xml` for convenience only.

## 4. Anything else load-bearing

**A1 / range references.** `a1` 1.0.2 (MIT; 2026-05-28; successor of `a1_notation` 0.6.3, same author) parses `'Sheet Name'!A1:B2` with `''` quote escapes (`$REG/a1-1.0.2/src/a1/from_str.rs:4-52`), `$` anchors on rows/columns (`src/row/from_str.rs:12-13`), whole-column/row and multi-area ranges (docs.rs `a1` 1.0.2); dependencies `rkyv` and `serde` (`Cargo.toml:57-61`), which is heavy for a CLI that only needs parsing. `formualizer-common` 3.1.1 (MIT OR Apache-2.0; 2026-09-06) has `SheetCellRef { sheet: SheetLocator, coord: RelativeCoord }` with `row_abs`/`col_abs` and `try_from_a1(sheet, "A1")` (`$REG/formualizer-common-3.1.1/src/address.rs:330-360, 220-228`); its sibling `formualizer-parse` tokenises full formulas including quoted sheet qualifiers (`src/tokenizer.rs:693-720`), but no single-call parser for a bare `'Sheet'!$A$1:$B$2` string was found in `address.rs` (**unverified** in `parser.rs`). The grammar is small (quoted-or-bare sheet, `!`, `$?[A-Z]+$?[0-9]+`, optional `:` range); an own ~60-line parser avoids both dependency trees.

**Licences** (crates.io, 2026-09-11): `zip` 8.6.0 MIT · `quick-xml` 0.42.0 MIT · `roxmltree` 0.21.1 MIT OR Apache-2.0 · `calamine` 0.36.1 MIT · `rawzip` 0.5.1 MIT · `rc-zip` 5.4.1 MIT OR Apache-2.0 · `xmlparser` 0.13.6 MIT/Apache-2.0 · `xot` 0.31.2 MIT · `a1` 1.0.2 MIT · `formualizer-*` 3.1.1 MIT OR Apache-2.0.

**MSRVs**: `zip` 1.88, `calamine` 1.88, `quick-xml` 1.86, `rawzip` 1.85, `roxmltree` 1.60 (toolchain on this Mac is cargo 1.95.0, per the handoff).

## Sources

Crate sources (cargo registry, fetched 2026-09-11; `$REG` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`):
- `$REG/zip-8.6.0/`: `Cargo.toml`; `src/write.rs` (497-503, 721-737, 1034-1046, 1169-1330, 1563-1580, 1623-1742, 1866-1930, 2489-2556); `src/read.rs` (218-280, 524-567, 777-779, 971-1078); `src/read/zip_archive.rs:535`; `src/types.rs` (235-260, 320-345, 351-386, 405-478, 592-608, 621-720).
- `$REG/zip-9.0.0-pre3/src/write.rs:1540-1573`, `src/read/zipfile.rs:309-333`; zip2 `CHANGELOG.md` (master) https://raw.githubusercontent.com/zip-rs/zip2/master/CHANGELOG.md
- `$REG/quick-xml-0.42.0/`: `Cargo.toml`; `src/reader/mod.rs` (27-260, 293-304, 574, 902-926); `src/reader/buffered_reader.rs` (20-41, 411, 503); `src/reader/slice_reader.rs` (75, 159, 253-264); `src/writer.rs` (29-60, 127, 262-317); `src/events/mod.rs` (256-262, 302-309, 563-588); `src/events/attributes.rs:363-365`.
- `$REG/roxmltree-0.21.1/`: `Cargo.toml`; `README.md`; `src/lib.rs` (201-217, 579-630, 1450-1455); `src/parse.rs` (94-97, 325-346, 415-441, 718-750, 880-910); `src/tokenizer.rs` (298-301, 325-345).
- `$REG/calamine-0.36.1/`: `Cargo.toml`; `src/lib.rs` (225-294, 323-364, 475, 563-569); `src/xlsx/mod.rs` (341-345, 448-548, 1141, 2578, 2601-2620, 3596-3632); `src/xlsx/cells_reader.rs` (142-152, 256-300, 317-399, 554-676); `src/formats.rs` (8-12, 112-131); `src/datatype.rs` (36-56, 340-361, 690-726); `src/auto.rs:42`.
- `$REG/rawzip-0.5.1/`: `README.md`; `src/archive.rs` (86-261, 330-345, 548-683, 1211-1462); `src/writer.rs` (88-600, 789, 867, 1010-1178).
- `$REG/rc-zip-5.4.1/src/lib.rs:1-30`, `src/parse/archive.rs:63-140`; `$REG/rc-zip-sync-4.4.2/src/read_zip.rs:140-215`.
- `$REG/xmlparser-0.13.6/README.md`, `src/lib.rs:84-200, 345-365`; `$REG/xot-0.31.2/README.md`; `$REG/a1-1.0.2/src/a1/from_str.rs`, `src/row/from_str.rs`, `Cargo.toml`; `$REG/formualizer-common-3.1.1/src/address.rs`; `$REG/formualizer-parse-3.1.1/src/tokenizer.rs:686-730`.

Registry metadata (crates.io API, 2026-09-11): https://crates.io/api/v1/crates/{zip,quick-xml,roxmltree,calamine,xmlparser,rawzip,rc-zip,rc-zip-sync,xot,zip-extract,a1,a1_notation,formualizer-common,formualizer-parse}; docs.rs https://docs.rs/a1/1.0.2/a1/.

Specifications:
- PKWARE APPNOTE.TXT 6.3.10 (2022-11-01), https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT — §4.4.2 (version made by), §4.4.3 (4.5 = zip64), §4.4.4 (bits 1-2 for methods 8/9, lines 783-788), §4.6.1 table (0xa220), §4.6.10 (Growth Hint structure).
- ECMA-376 Part 2, 5th edition (December 2021), "Open Packaging Conventions", PDF inside https://ecma-international.org/wp-content/uploads/ECMA-376-2_5th_edition_december_2021.zip — §6.2.4 Growth hint; §7.3 ZIP mapping table and §7.3.7 Media Types stream item name; Annex B (normative) B.1-B.4, Tables B.1 and B.5, extra-field table (pp. 63-72). Text was extracted with a crude PDF-stream decoder (no poppler available); quotes may lose spacing.

Probes (all under the session scratchpad `/private/tmp/claude-501/-Users-niko86-sources-rust-xlsplice/bf95a682-593d-475a-b859-46760caa23d4/scratchpad/`): `src/bin/qx.rs` (quick-xml round trip + offsets), `src/bin/rx.rs` (roxmltree ranges), `src/bin/zz.rs` (zip `raw_copy_file`), `src/bin/mz.rs` (zip `merge_archive`), `zipdump.py` (header dump), `mk.applescript`/`mk3.applescript`/`oracle.applescript` (Excel 16.112.4 save and open oracle), `excel-probe.xlsx` and its two copies. Excel's sandbox required saving inside `~/Library/Containers/com.microsoft.Excel/Data/`; a `Grant File Access` sheet appeared for a `/private/tmp` path and was cancelled.
