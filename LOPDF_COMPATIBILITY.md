# lopdf 0.42 compatibility investigation

## Branch and frozen baseline

- Branch: `compat/lopdf-042`
- Production baseline commit: `50701d84e2b2aa7441b84f41eca2e97136f2e592`
- Current version: `lopdf 0.36.0`
- Minimum secure target: `lopdf 0.42.0`
- Golden manifest: `tests/benchmarks/MT_AngelRaise_1080p.json` (must not change)
- Golden input: 61,002,045 bytes, SHA-256 `78ab94985e49bd7240432077cc0cc16ff3d558282ae5820c1b8134394150ab6d`
- Golden output: 11,835,505 bytes (80.59818322484107% reduction)
- Golden maximum mean render error: 5.817019354423868 / 255; validation PASS; 9/9 checks PASS
- Warm-cache baseline benchmark: 7.15 s wall, 9.49 s user, 0.23 s system, 839,254,016-byte maximum resident set size (single local sample; `/usr/bin/time -lp`).

The optimisation algorithm, compression settings, compositing, resizing, encoding, validation tolerances, and golden expectations are frozen for this investigation.

## Direct lopdf usage inventory

### Loading and parsing

- `src/parser.rs:1,21,46`: imports `Content`, `Dictionary`, `Document`, `Object`, `ObjectId`, `Stream`; converts `lopdf::Error`; loads all inspected input with `Document::load`.
- `src/exporter.rs:7,82,162`: loads input with `Document::load` before export.
- `src/rewriter.rs:3,20,37`: loads the input for stream replacement with `Document::load`.
- `src/raster_merge.rs:7,23,99`: loads the input before raster compositing.
- `src/flatten_pages.rs:10,27,203-206,253`: loads working, original, graphics-only, and temporary documents.
- `src/validator.rs:3,34,123-124`: loads both original and exported documents for structural validation.

This makes the parser security issue reachable for any PDF selected by the user. Validation cannot prevent it because validation also occurs after `Document::load`.

### Object access and page traversal

- `src/parser.rs:46-292`: traverses `Document::objects`, `get_pages`, page/resource dictionaries, references, arrays, names, integers/reals, streams, and decoded content operations; resolves referenced dictionaries with `get_dictionary`.
- `src/raster_merge.rs:99-690`: traverses pages, content streams, XObjects, resources and ExtGState dictionaries; resolves references and inspects blend/opacity objects.
- `src/flatten_pages.rs:203-734`: traverses page dictionaries, page boxes, resources, XObjects and nested form streams; uses `get_object`, `get_dictionary`, `Object::as_stream`, and an explicit visited-object set.
- `src/validator.rs:123-221`: compares pages, page boxes, rotations, annotations, selected semantic objects and bookkeeping streams across original/output documents.

### Object and stream mutation

- `src/rewriter.rs:37-79`: obtains mutable image streams, replaces encoded stream bytes, updates Width/Height/Filter/Length dictionary entries, then saves the document.
- `src/raster_merge.rs:99-497`: creates Flate image XObjects with `Stream::new`, rewrites page resources/content operations, adds/removes objects, and saves.
- `src/flatten_pages.rs:203-566`: creates JPEG image XObjects/content streams, rewrites page contents/resources, prunes graphics while preserving text/vector paths, and saves temporary/final documents.

### Image extraction and replacement

- `src/parser.rs:80-219`: identifies image XObjects, reads stream dictionaries/content/filter chains, records dimensions/colour/bits and placements.
- `src/rewriter.rs:48-74`: replaces selected image stream data and dimensions without changing placement geometry.
- `src/raster_merge.rs:388-474`: adds the merged raster image and corresponding `Do` operation.
- `src/flatten_pages.rs:338-415,510-566`: creates flattened images and recursively inspects form/image streams.

### Geometry, annotations, text, vectors, fonts and metadata

- Page boxes/rotation/geometry: `src/parser.rs:48-79,230-292`; `src/raster_merge.rs:107-216`; `src/flatten_pages.rs:215-337,567-734`; `src/validator.rs:123-194`.
- Annotations: preserved and compared through page dictionaries in `src/validator.rs:123-194`; fixture construction/assertions in `tests/export.rs:39,80-91` and `tests/flatten_pages.rs:23,61,134-145`.
- Text/vector operations: decoded/encoded with `lopdf::content::Content` and `Operation` in `src/parser.rs`, `src/raster_merge.rs`, and `src/flatten_pages.rs`; preservation is checked by `src/validator.rs` and the golden tests.
- Fonts: NoBS does not rewrite font programs directly. Font dictionaries/resources and text operators are preserved/traversed in `src/parser.rs`, `src/raster_merge.rs`, and `src/flatten_pages.rs`; synthetic fonts are created in test fixtures.
- Metadata: NoBS performs no dedicated metadata rewrite. Trailer/catalog/info objects are retained by loading, mutating and saving the same `Document`; semantic validation ignores only explicitly identified bookkeeping streams.
- Encryption: NoBS calls no encryption/decryption API directly. Behaviour is inherited from `Document::load` and save; encrypted inputs therefore require corpus coverage.

### Serialization and writing

- `src/exporter.rs`, `src/rewriter.rs`, `src/raster_merge.rs`, and `src/flatten_pages.rs` save mutated `Document` values using lopdf's writer.
- `Content::encode` serializes replacement page/form content streams in `src/raster_merge.rs` and `src/flatten_pages.rs`.
- Tests construct complete documents with `Document::with_version`, `add_object`, `Stream::new`, `dictionary!`, `Object`, `Content`, and `Operation`: `tests/inspection.rs`, `tests/export.rs`, `tests/raster_merge.rs`, and `tests/flatten_pages.rs`.

## Upgrade/API adaptations

Pending. Only `lopdf` will be changed, to the minimum patched `0.42.0`; every required source adaptation will be listed here.

## Regression, output, render, hostile-corpus and performance results

Pending upgrade.

## Security audit

Baseline: `RUSTSEC-2026-0187` remains through `lopdf 0.36.0`. The previously addressed `time` and `quick-xml` advisories are resolved.

## Final recommendation

Pending investigation: **NOT SAFE TO UPGRADE YET** until every decision criterion has been exercised.
