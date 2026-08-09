# NoBS PDF — `pdfdoctor`

`pdfdoctor` is the first, read-only inspection milestone for NoBS PDF. It answers what is in a PDF, how raster images are placed, and where measurable oversampling exists. It does **not** rewrite, compress, upload, or rasterise the document.

## Run

```sh
cargo run -- inspect input.pdf
cargo run -- plan input.pdf > plan.json
cargo run -- plan input.pdf --document-target 1080p > screen-plan.json
cargo run -- export input.pdf --document-target 1080p --output output.pdf > export-report.json
cargo run --release -- benchmark tests/MT_AngelRaise_01.pdf --document-target 1080p
cargo test
```

JSON is written to stdout; actionable errors go to stderr with a non-zero exit status.

The frozen golden benchmark and its acceptable regression envelope are documented in [`BENCHMARK.md`](BENCHMARK.md). The benchmark produces measured structured data, never retains its generated PDF, and exits unsuccessfully when hard validation fails.

`plan` writes its human-readable summary to stderr and its versioned JSON plan to stdout, so redirecting stdout produces a clean machine-readable file. Use `--target-dpi 150` (default: 300) to select the deterministic planning target. Planning is read-only.

Screen planning is a separate model from print DPI. Select `--document-target original|4k|1440p|1080p|720p|custom`; custom additionally requires `--custom-long-dimension N`. A screen profile maps each page's actual aspect ratio to the profile's long pixel dimension and calculates each raster placement's pixel occupancy from its transformation geometry. It never rasterises a page and never changes page boxes, rotation, physical dimensions, text, vectors, links, or annotations. `Original` disables resolution recommendations. Screen 1080p follows the profile contract `long_dimension_px = 1920`; computed page dimensions are always reported explicitly.

## Conservative 1080p export

`export` currently supports only the 1080p document target. It approves direct 8-bit JPEG (`DCTDecode`) streams in DeviceRGB, DeviceGray, or DeviceCMYK with no image/soft mask. CMYK channels remain CMYK and are never routed through RGB. Approved rasters are resized with deterministic Lanczos3 and re-encoded as JPEG; lossless, JPEG2000, CCITT, masked, indirect-colour-space, and otherwise uncertain images remain untouched with a `skipped_reason`.

The source is never overwritten and an existing destination is rejected. A temporary sibling PDF is rebuilt first, reopened, inspected, and validated. Page count/geometry/rotation, image references/matrices/bounds/aspect/format/colour depth, and all logical non-targeted indirect objects must be preserved. PDF bookkeeping containers and linearisation hints may be regenerated. Only after validation passes is the temporary file atomically persisted. `--dry-run` performs inspection, planning, and candidate approval but writes nothing.

## Conservative raster merge

```sh
cargo run --release -- raster-merge input.pdf --document-target 1080p --output merged.pdf
```

This opt-in stage composites only provably safe direct raster-first page content. It preserves content-stream image order, crops to the raster union, derives pixels from the 1080p page scale, and leaves text/vector operators intact. The first implementation supports opaque direct DeviceRGB/DeviceGray JPEGs, affine placement, and safe rectangular clipping. Pages with raster/text/vector interleaving, forms, masks, non-normal transparency, complex clipping, unsupported colour/encoding, or uncovered transparent union areas are skipped with reasons. It writes through a temporary file, reinspects the result, and validates page geometry plus text/vector operation counts. It is intentionally separate from normal resolution export.

## Full-page raster export

```sh
cargo run --release -- flatten-pages input.pdf --document-target 1080p --output flattened.pdf > flatten-report.json
```

This explicit compatibility mode uses PDFium to render complete pages without annotation appearances, replaces each page content stream with one RGB JPEG, and preserves page boxes, page rotation, and annotation/link dictionaries. It handles forms, CMYK, clipping, transparency, blending, text, and vectors because PDFium resolves them before rebuilding.

This mode is destructive: text is no longer selectable/searchable, vectors lose infinite scalability, and accessibility structure tied to page content may stop working. It is intended for screen-only sharing copies, never as the default NoBS PDF export. Validation reopens and rerenders the output at the same dimensions, checks all page boxes/rotations/annotation references, requires exactly one page image, and limits mean per-channel render error to 12/255.

The local development build looks for `vendor/pdfium/lib/libpdfium.dylib` by default. A packaged desktop application must bundle the correct PDFium binary for each platform. Use `--pdfium-library /path/to/library` to override it.

## Architecture

The library is independent of the CLI:

- `parser`: `PdfParser` boundary and the current `LopdfParser`; walks the indirect-object graph, inherited page resources, content streams, graphics-state stack, and nested Form XObjects.
- `geometry`: auditable affine-matrix, unit, and DPI mathematics with no PDF dependency.
- `analysis`: deterministic target-DPI analysis; never changes a PDF.
- `model`: versioned serialisable report and reserved `OptimisationCandidate` concept.
- `optimisation`: non-negotiable pre/post raster invariant validator; it performs no mutation.
- `main`: thin CLI adapter.

`lopdf` 0.36 was selected because it provides direct access to indirect objects, raw encoded stream payloads, dictionaries, filters, and content operations while remaining portable Rust. Those capabilities matter more here than rendering. The `PdfParser` trait permits a later PDFium or MuPDF validation backend for difficult files without coupling analysis to it. Dependencies are pinned where necessary to support Rust 1.83.

## Transform and effective DPI handling

The inspector interprets `q`, `Q`, `cm`, and `Do`. It composes the current transformation matrix (CTM), including Form XObject matrices. An image maps a unit square through the CTM. Displayed axis lengths are:

```text
width_pt  = hypot(a, b)
height_pt = hypot(c, d)
dpi_x = pixel_width  * 72 / width_pt
dpi_y = pixel_height * 72 / height_pt
effective_dpi = min(dpi_x, dpi_y)
```

This handles translation, rotation, and non-uniform scale. Page rotation is reported but intentionally not applied to physical lengths: rotating the page coordinate system does not change a placed object's physical scale. Bounding boxes report the transformed unit square; consumers should normalise corner ordering if needed. Clipping is not yet used to reduce displayed dimensions because a clipped image's appropriate resampling policy requires preserving crop geometry separately.

Targets 96, 150, 200, 300, and 600 DPI include per-axis oversampling ratios and rounded required pixel dimensions. Zero/non-finite display dimensions produce `null` DPI and `status: "unknown"`.

## Object and size model

Images are keyed by PDF indirect object ID. Every invocation becomes a placement, so reuse across pages and within a page is preserved. SHA-256 is calculated over encoded stream bytes without decoding pixels; this detects byte-identical data in separate objects. Encoded image, font-program, embedded-file, and XML metadata stream payload sizes are exact. Container syntax, xref data, dictionaries, content streams, and unclassified objects are deliberately placed in `unknown_bytes`; the report never claims they are attributable savings.

The raw pixel estimate currently assumes three colour components when bits-per-component is known. It is explicitly an estimate and should be refined using resolved ICC, indexed, separation, masks, and decode parameters before optimisation work begins.

## JSON schema 0.1.0

The canonical schema is documented in [`schema/report.schema.json`](schema/report.schema.json). Top-level fields are `schema_version`, `file`, `summary`, `pages`, `images`, `fonts`, `embedded_files`, `metadata`, and `warnings`. Optional/indeterminable measurements serialize as `null`; warnings preserve partial results.

## Performance and safety

Image hashes use encoded bytes and image pixel data is never decoded. `lopdf` currently loads the PDF object graph in memory, so this milestone is not fully streaming. Form streams larger than 16 MiB encoded are skipped with a warning because this compatible `lopdf` release lacks bounded decompression. A production hardening pass should move to a bounded/streaming parser release or secondary backend and enforce global object, nesting, and decoded-byte budgets.

## Known limitations

- Inline images (`BI`/`ID`/`EI`) are warned about but not catalogued in 0.1.
- Password-protected PDFs requiring a non-empty password are unsupported.
- Pattern content, Type 3 glyph programs, soft masks, annotation appearances, optional-content visibility, clipping, and malformed resource cycles need deeper traversal.
- Fonts currently catalogue embedded font-program streams rather than resolving every font dictionary to its program.
- Colour spaces that are arrays/indirect objects are not yet rendered to a friendly name.
- Encoded duplicate hashes will not identify visually identical images encoded differently.
- Object payload attribution excludes PDF syntax overhead and may overlap conceptual ownership; `unknown_bytes` is the conservative remainder.

Tests cover A4 conversion, canonical DPI examples, non-square/non-uniform scale, rotation, unknown dimensions, page rotation, repeated references, and duplicate encoded data using a generated real PDF fixture.

## Non-negotiable raster invariants

Any future optimiser is fail-closed. Before an image replacement may be committed, it must validate the rebuilt PDF object against its captured pre-optimisation state. Pixel dimensions and encoded bytes may change, but aspect ratio (allowing only unavoidable integer rounding), placement matrix, rendered bounds, and rotation must remain unchanged. Cropping, stretching, and squashing are forbidden. Format, colour space, and colour depth remain unchanged unless an explicit policy permits a specific change; permitted colour changes will additionally require visual-impact evaluation. A failed or indeterminate check means the original image remains untouched.

These contracts are represented by `RasterState`, `ExplicitPolicyPermissions`, and `validate_raster_invariants()`. They exist now so optimisation cannot later be implemented without a testable post-rebuild validation gate.
