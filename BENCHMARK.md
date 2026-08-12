# NoBS PDF golden optimisation benchmark

The frozen benchmark protects the selective 1080p graphics-flattening behaviour that produced the current quality baseline. It does not alter the optimisation algorithm.

Run the measured benchmark from the repository root:

```sh
cargo run --release -- benchmark tests/MT_AngelRaise_01.pdf --document-target 1080p
```

The human summary is written to stderr and structured JSON to stdout. To retain a report without committing a generated PDF:

```sh
cargo run --release -- benchmark tests/MT_AngelRaise_01.pdf --document-target 1080p > benchmark-report.json
```

Run the golden regression assertions explicitly:

```sh
cargo test --release --test benchmark -- --ignored --test-threads=1
```

The machine-readable baseline is `tests/benchmarks/MT_AngelRaise_1080p.json`. Its output-size window is deliberately broad enough to tolerate small encoder/platform differences while catching a major regression. The generated PDF is temporary and is deleted after inspection and validation.

## What is deterministic

- The target is fixed at a 1920-pixel page long edge.
- Page traversal and PDF content-stream paint order are authoritative.
- JPEG quality and rendering configuration are fixed.
- The same source must retain its recorded SHA-256 digest.
- Structural outcomes and output size must remain within the manifest contract.

Byte-for-byte output identity is not required. PDF object serialization and native PDFium builds can differ without changing the rendered or structural result, so the benchmark protects material consistency instead.

## Metric boundaries

Counts in the JSON report are inspector measurements. `images_downsampled` is `null` because selective compositing cannot truthfully attribute the resulting pixels to independently downsampled source objects. The foreground-text policy bakes the page through the final non-text artwork paint operation, then retains the untouched trailing text suffix. Text below or interleaved with later artwork is intentionally rasterised. The report records source, below-boundary, above-boundary, retained-native, and searchable-foreground results per page.
