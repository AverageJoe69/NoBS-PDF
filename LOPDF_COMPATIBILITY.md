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

`Cargo.toml` was changed only from `lopdf = "=0.36.0"` to the minimum patched `lopdf = "=0.42.0"`. Root and desktop lockfiles were refreshed deliberately. No Rust source adaptation was required: `cargo check` succeeded immediately, so there are no API compatibility shims and no application-logic changes.

The required lopdf graph changes remove `bytecount`, `nom_locate`, the old rand 0.9 support chain and related WASI/zerocopy packages; they add/update lopdf's selected `getrandom`, `rand 0.10`, `chacha20`, and `ttf-parser 0.25.1` dependencies. The direct `time = 0.3.47` security pin remains unchanged.

## Regression, output, render, hostile-corpus and performance results

### Existing regression suite

- `cargo test`: PASS, 35/35 normal tests; 9 expensive golden tests correctly skipped by default.
- `cargo test --release`: PASS, 35/35 normal release tests.
- `cargo test --release --test benchmark -- --ignored --test-threads=1`: PASS, 9/9 golden checks.
- Desktop Rust/licensing tests: PASS, 2/2.
- Website tests: PASS, 18/18; website production build PASS.
- Desktop frontend and Tauri macOS release application bundle: PASS.

### Golden benchmark and render comparison

The 0.42 benchmark is exactly equal to the production reference:

- input: 61,002,045 bytes; 15 pages; 108 raster objects; 88 placements
- output: 11,835,505 bytes; reduction 80.59818322484107%
- raster objects after: 15; pages composited: 15
- text operations: 645 before / 232 after
- vector operations: 267 before / 11 after
- page count, geometry, boxes, rotation, annotations and aspect ratios: PASS
- selectable text and native vectors: present
- maximum mean render error: 5.817019354423868 / 255; validation PASS

An explicit full-page export was also generated independently from baseline commit `50701d8` (lopdf 0.36) and this branch (lopdf 0.42). Both outputs are byte-for-byte identical: 11,858,548 bytes with SHA-256 `e7c9a7f7bcfa8bd60e58ecbc9a39ba7f2b8d97f53696e83ef312142f1dbabc7f`. Therefore object counts/order, xref serialization, stream lengths/data, image dimensions/formats/filters, page resources/content streams, fonts, annotations and metadata are identical for the golden document. All 15 per-page pixel dimensions and mean render errors are also identical; the maximum for this explicit export is 4.759586709104938.

### Benign malformed/hostile corpus

`tests/malformed_pdfs.rs` adds bounded regression coverage. All cases complete without panic or stack overflow: header-only/truncated inputs, invalid object reference, malformed stream length, invalid xref, incomplete encryption dictionary, corrupt font/image streams, unusual filter chain, and a direct object nested to depth 128. The depth-128 object is rejected as an error beyond lopdf 0.42's 100-level limit. The payload is intentionally small and benign.

### Available real-document coverage

The golden mixed 15-page presentation and synthetic existing fixtures cover raster-heavy/mixed pages, page geometry/rotation/boxes, annotations, fonts, selectable text, vectors, repeated images, unusual placement geometry, and multi-page output. The workspace contains no independent real InDesign export, Illustrator-heavy file, encrypted PDF, transparency-specific file, dedicated text-heavy publication, corrupt real font/image sample, or unusual page-tree corpus. Those categories therefore remain unverified; generated output PDFs are not independent source documents.

### Performance

Single warm-cache local samples with `/usr/bin/time -lp`:

- lopdf 0.36: 7.15 s wall, 9.49 s user, 0.23 s system, 839,254,016-byte maximum RSS.
- lopdf 0.42: 7.14 s wall, 6.63 s user, 0.27 s system, 862,879,744-byte maximum RSS.

Wall time is effectively unchanged. The one-sample maximum RSS increase is approximately 2.8%; this is not an unreasonable regression, but single samples are not a rigorous memory benchmark.

## Security audit

Both root and desktop `cargo audit` runs report zero vulnerabilities. `RUSTSEC-2026-0187`, `RUSTSEC-2026-0009`, `RUSTSEC-2026-0194`, and `RUSTSEC-2026-0195` are resolved. The new lopdf graph produces an informational `RUSTSEC-2026-0192` warning because `ttf-parser 0.25.1` is unmaintained; RustSec does not classify it as a vulnerability. Existing target-specific desktop maintenance/unsoundness warnings remain unchanged apart from this additional warning.

## Final recommendation

**NOT SAFE TO UPGRADE YET.**

The code and golden evidence strongly support compatibility: no API or logic changes were needed, every automated test passes, golden observable behaviour is exactly unchanged, an explicit export is byte-identical, the bounded malformed corpus passes, performance is acceptable, the release application builds, and the target vulnerability is removed.

However, the brief permits **SAFE TO UPGRADE** only after the requested representative real-world and hostile document coverage is exercised. Those source documents are not present in the workspace, particularly independent InDesign/Illustrator/transparency/encrypted/text-heavy/unusual-page-tree and corrupt-font/image cases. The upgrade should remain on this branch until that corpus is supplied and produces no panic, uncontrolled resource use, validation failure, or unexplained output/render difference.

To reach **SAFE TO UPGRADE**, add sanitized representative PDFs for the missing categories, record provenance/type, run inspection and the applicable optimisation path under both baseline and 0.42, compare output structure/render/performance, and append the results here. If those pass, the exact merge consists of the `Cargo.toml` lopdf pin, both deliberate lockfile changes, this compatibility report, and the malformed-PDF regression test; the golden manifest must remain unchanged.
